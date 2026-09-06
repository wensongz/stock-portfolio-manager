#[rustfmt::skip]
use super::*;
use super::contracts::recompute_option_statuses;
use super::csv::{export_options_csv_inner, get_field, import_options_csv_inner, normalize_action};
use crate::db::Database;
use ::csv::StringRecord;

/// Build an in-memory DB with one US account.
fn db_with_account() -> (Database, String) {
    let db = Database::new(":memory:").expect("failed to create in-memory database");
    let account_id = "acct-test".to_string();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at)
                 VALUES (?1, ?2, 'US', ?3, ?3)",
            rusqlite::params![account_id, "Test Account", chrono::Utc::now().to_rfc3339()],
        )
        .expect("failed to insert account");
    }
    (db, account_id)
}

fn insert_exposure_record(
    db: &Database,
    account_id: &str,
    option_type: &str,
    id: &str,
    quantity: i64,
    is_open: bool,
    traded_at: &str,
) {
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO option_records
             (id, account_id, option_symbol, underlying, expiry_date, strike_price,
              option_type, action, code, quantity, price, amount, commission, fee,
              traded_at, created_at, contract_status)
             VALUES (?1, ?2, ?3, 'AAPL', '18SEP26', 100, ?4, ?5, ?6, ?7,
                     2, ?8, 1, 0, ?9, ?9, 'active')",
            rusqlite::params![
                id,
                account_id,
                format!("AAPL 18SEP26 100 {option_type}"),
                option_type,
                if is_open { "SELL" } else { "BUY" },
                if is_open { "O" } else { "C" },
                quantity,
                quantity.abs() as f64 * 200.0,
                traded_at,
            ],
        )
        .unwrap();
}

#[test]
fn test_partial_close_exposes_remaining_quantity_without_changing_opening_facts() {
    for (option_type, open_quantity, remaining_quantity) in [("P", 10, 6), ("C", -10, -6)] {
        let (db, account_id) = db_with_account();
        insert_exposure_record(
            &db,
            &account_id,
            option_type,
            "open",
            open_quantity,
            true,
            "2026-08-01",
        );
        insert_exposure_record(
            &db,
            &account_id,
            option_type,
            "close",
            4,
            false,
            "2026-08-02",
        );

        let contracts = get_option_contracts_inner(&db, &account_id).unwrap();
        let contract = &contracts[0];
        assert_eq!(contract.status, "active");
        assert_eq!(contract.contracts, open_quantity);
        assert_eq!(contract.open_amount, 2_000.0);
        assert_eq!(contract.commission, 1.0);
        assert_eq!(contract.close_price, None);
        assert_eq!(
            serde_json::to_value(contract).unwrap()["remaining_contracts"],
            remaining_quantity
        );
    }
}

#[test]
fn test_partial_close_sell_put_simulation_uses_only_six_remaining_contracts() {
    let (db, account_id) = db_with_account();
    insert_exposure_record(&db, &account_id, "P", "open", 10, true, "2026-08-01");
    insert_exposure_record(&db, &account_id, "P", "close", 4, false, "2026-08-02");

    let simulations = simulation::simulate_sell_put_inner(
        &db,
        &account_id,
        vec![StockPriceInput {
            symbol: "AAPL".into(),
            price: 90.0,
        }],
    )
    .unwrap();

    assert_eq!(simulations[0].total_cash_needed, 60_000.0);
    assert_eq!(simulations[0].contracts[0].contracts, 6);
    assert_eq!(simulations[0].contracts[0].cash_needed, 60_000.0);
}

#[test]
fn test_partial_close_sell_call_simulation_uses_only_six_remaining_contracts() {
    let (db, account_id) = db_with_account();
    insert_exposure_record(&db, &account_id, "C", "open", -10, true, "2026-08-01");
    insert_exposure_record(&db, &account_id, "C", "close", 4, false, "2026-08-02");

    let simulations = simulation::simulate_sell_call_inner(
        &db,
        &account_id,
        vec![StockPriceInput {
            symbol: "AAPL".into(),
            price: 110.0,
        }],
    )
    .unwrap();

    assert_eq!(simulations[0].total_shares_needed, 600);
    assert_eq!(simulations[0].contracts[0].contracts, -6);
    assert_eq!(simulations[0].contracts[0].shares_needed, 600);
}

#[test]
fn test_partial_close_simulations_follow_fifo_until_all_opens_are_closed() {
    for option_type in ["P", "C"] {
        let (db, account_id) = db_with_account();
        for (id, quantity, is_open, date) in [
            ("open-old", -10, true, "2026-08-01"),
            ("open-new", -5, true, "2026-08-02"),
            ("close-first", 4, false, "2026-08-03"),
            ("close-second", 8, false, "2026-08-04"),
        ] {
            insert_exposure_record(&db, &account_id, option_type, id, quantity, is_open, date);
        }
        let price = if option_type == "P" { 90.0 } else { 110.0 };
        let prices = || {
            vec![StockPriceInput {
                symbol: "AAPL".into(),
                price,
            }]
        };
        if option_type == "P" {
            let simulations =
                simulation::simulate_sell_put_inner(&db, &account_id, prices()).unwrap();
            assert_eq!(simulations[0].contracts.len(), 1);
            assert_eq!(simulations[0].contracts[0].contracts, -3);
            assert_eq!(simulations[0].total_cash_needed, 30_000.0);
        } else {
            let simulations =
                simulation::simulate_sell_call_inner(&db, &account_id, prices()).unwrap();
            assert_eq!(simulations[0].contracts.len(), 1);
            assert_eq!(simulations[0].contracts[0].contracts, -3);
            assert_eq!(simulations[0].total_shares_needed, 300);
        }

        insert_exposure_record(
            &db,
            &account_id,
            option_type,
            "close-final",
            3,
            false,
            "2026-08-05",
        );
        let contracts = get_option_contracts_inner(&db, &account_id).unwrap();
        assert!(contracts.iter().all(|contract| contract.status == "closed"));
        assert!(contracts
            .iter()
            .all(|contract| serde_json::to_value(contract).unwrap()["remaining_contracts"] == 0));
        assert_eq!(
            contracts
                .iter()
                .map(|contract| contract.contracts.abs())
                .sum::<i64>(),
            15
        );
        assert!(
            simulation::simulate_sell_put_inner(&db, &account_id, prices())
                .unwrap()
                .is_empty()
        );
        assert!(
            simulation::simulate_sell_call_inner(&db, &account_id, prices())
                .unwrap()
                .is_empty()
        );
    }
}

/// A sample IBKR-style English-header options trade CSV.
/// All close records have a matching open record (same symbol, enough quantity).
const ENGLISH_CSV: &str = "\
Acct ID,Symbol,Trade Date/Time,Settle Date,Exchange,Type,Quantity,Price,Proceeds,Comm,Fee,Order Type,Code
U1234567,AAPL 20FEB26 100 P,2026-01-15 10:30:00,2026-01-16,SMART,SELL,2,3.50,700,1.20,0.05,LMT,O
U1234567,AAPL 20FEB26 100 P,2026-01-15 10:30:00,2026-01-16,SMART,SELL,1,3.50,350,1.20,0.05,LMT,O
U1234567,PDD 20MAR26 80 C,2026-01-10 09:30:00,2026-01-11,SMART,SELL,3,1.50,450,1.00,0.04,LMT,O
U1234567,PDD 20MAR26 80 C,2026-02-20 09:45:00,2026-02-21,SMART,BUY TO CLOSE,3,2.00,600,0.90,0.04,MKT,C;P
Total, ,,,,,,,,,,,
";

#[test]
fn test_import_english_header_csv() {
    let (db, account_id) = db_with_account();
    let result =
        import_options_csv_inner(&db, &account_id, ENGLISH_CSV).expect("import should succeed");
    assert_eq!(result.imported, 4, "all 4 trade rows should import");
    assert_eq!(result.skipped, 1, "the Total row should be skipped");
    assert!(
        result.errors.is_empty(),
        "expected no errors, got: {:?}",
        result.errors
    );
}

#[test]
fn test_import_rejects_malformed_price() {
    let (db, account_id) = db_with_account();
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,oops,200.00,0,0,LMT,O
";

    let result =
        import_options_csv_inner(&db, &account_id, csv).expect("row error should be reported");

    assert_eq!(result.imported, 0);
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].contains("Row 2"));
    assert!(result.errors[0].contains("price"));
    let count: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM option_records", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0);
}

#[test]
fn test_import_rejects_blank_price() {
    let (db, account_id) = db_with_account();
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,,200.00,0,0,LMT,O
";

    let result =
        import_options_csv_inner(&db, &account_id, csv).expect("row error should be reported");

    assert_eq!(result.imported, 0);
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].contains("price"));
}

#[test]
fn test_import_rejects_non_finite_amount() {
    let (db, account_id) = db_with_account();
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,2.00,NaN,0,0,LMT,O
";

    let result =
        import_options_csv_inner(&db, &account_id, csv).expect("row error should be reported");

    assert_eq!(result.imported, 0);
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].contains("Row 2"));
    assert!(result.errors[0].contains("amount"));
}

#[test]
fn test_import_rejects_malformed_quantity() {
    let (db, account_id) = db_with_account();
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1.5,2.00,200.00,0,0,LMT,O
";

    let result =
        import_options_csv_inner(&db, &account_id, csv).expect("row error should be reported");

    assert_eq!(result.imported, 0);
    assert_eq!(result.skipped, 0);
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].contains("Row 2"));
    assert!(result.errors[0].contains("quantity"));
}

#[test]
fn test_import_rejects_zero_quantity() {
    let (db, account_id) = db_with_account();
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,0,2.00,200.00,0,0,LMT,O
";

    let result =
        import_options_csv_inner(&db, &account_id, csv).expect("row error should be reported");

    assert_eq!(result.imported, 0);
    assert_eq!(result.skipped, 0);
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].contains("quantity"));
}

#[test]
fn test_import_rolls_back_all_rows_when_insert_fails() {
    let (db, account_id) = db_with_account();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_second_option
                 BEFORE INSERT ON option_records
                 WHEN NEW.option_symbol = 'MSFT 20FEB26 100 P'
                 BEGIN
                   SELECT RAISE(ABORT, 'forced option insert failure');
                 END;",
        )
        .unwrap();
    }
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,2.00,200.00,0,0,LMT,O
a,MSFT 20FEB26 100 P,2026-01-15,,SMART,卖出,1,3.00,300.00,0,0,LMT,O
";

    let error = import_options_csv_inner(&db, &account_id, csv)
        .expect_err("database failure should abort the import");

    assert!(error.contains("forced option insert failure"));
    let count: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM option_records", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0, "the first insert must be rolled back too");
}

#[test]
fn test_import_rolls_back_rows_when_status_recompute_fails() {
    let (db, account_id) = db_with_account();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TRIGGER reject_option_status_update
                 BEFORE UPDATE OF contract_status ON option_records
                 BEGIN
                   SELECT RAISE(ABORT, 'forced option status failure');
                 END;",
        )
        .unwrap();
    }
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,2.00,200.00,0,0,LMT,O
";

    let error = import_options_csv_inner(&db, &account_id, csv)
        .expect_err("status failure should abort the import");

    assert!(error.contains("forced option status failure"));
    let count: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM option_records", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0, "insert must roll back with status updates");
}

#[test]
fn test_parse_english_header_csv_preview() {
    let preview = parse_options_csv(ENGLISH_CSV.to_string()).expect("preview should succeed");
    assert_eq!(preview.valid_rows, 4);
    assert!(
        preview.error_rows.is_empty(),
        "expected no errors, got: {:?}",
        preview.error_rows
    );
}

/// A Chinese-header options trade CSV with one open and one close record.
const CN_CSV: &str = "\
账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,2.00,200.00,0,0,LMT,O
a,AAPL 20FEB26 100 P,2026-02-20,,SMART,买入,1,0.10,10.00,0,0,C;P,C;P
";

#[test]
fn test_import_rejects_close_without_open() {
    // A close record (C;Ep expired) with no open record anywhere must be rejected.
    let (db, account_id) = db_with_account();
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-02-20,,SMART,买入,1,0.01,1.00,0,0,C;Ep,C;Ep
";
    let result = import_options_csv_inner(&db, &account_id, csv).expect("import should succeed");
    assert_eq!(result.imported, 0, "orphan close must not be inserted");
    assert_eq!(
        result.errors.len(),
        1,
        "expected one error, got: {:?}",
        result.errors
    );
}

#[test]
fn test_import_close_matches_open_in_same_csv() {
    let (db, account_id) = db_with_account();
    let result = import_options_csv_inner(&db, &account_id, CN_CSV).expect("import should succeed");
    assert_eq!(result.imported, 2, "open + close should both import");
    assert!(
        result.errors.is_empty(),
        "expected no errors, got: {:?}",
        result.errors
    );
}

#[test]
fn test_import_close_matches_existing_db_open() {
    // Open already in DB (from a previous import); only the close is imported now.
    let (db, account_id) = db_with_account();
    let ts = chrono::Utc::now().to_rfc3339();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
                "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
                 VALUES ('o1', ?1, 'AAPL 20FEB26 100 P', 'AAPL', '20FEB26', 100, 'P', 'SELL', 'O', 1, 2.00, 200.00, 0, 0, '2026-01-15', NULL, ?2, 'active')",
                rusqlite::params![account_id, ts],
            )
            .expect("failed to insert open record");
    }
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-02-20,,SMART,买入,1,0.10,10.00,0,0,C;P,C;P
";
    let result = import_options_csv_inner(&db, &account_id, csv).expect("import should succeed");
    assert_eq!(result.imported, 1, "close should match the existing open");
    assert!(
        result.errors.is_empty(),
        "expected no errors, got: {:?}",
        result.errors
    );
}

#[test]
fn test_import_close_exceeding_open_quantity_rejected() {
    // Open 1 contract but close 2 contracts: the extra close has no backing open.
    let (db, account_id) = db_with_account();
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,AAPL 20FEB26 100 P,2026-01-15,,SMART,卖出,1,2.00,200.00,0,0,LMT,O
a,AAPL 20FEB26 100 P,2026-02-20,,SMART,买入,2,0.10,20.00,0,0,C;P,C;P
";
    let result = import_options_csv_inner(&db, &account_id, csv).expect("import should succeed");
    assert_eq!(result.imported, 1, "only the open should import");
    assert_eq!(
        result.errors.len(),
        1,
        "close exceeding open qty must be rejected"
    );
}

#[test]
fn test_import_split_adjusted_close_matches() {
    // Contract split 2:1 configured in settings: open at strike 330 (BRK B),
    // close at strike 165 (post-split symbol). Must match cross-symbol.
    let (db, account_id) = db_with_account();
    let ts = chrono::Utc::now().to_rfc3339();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO stock_splits (stock_code, split_date, ratio_from, ratio_to, created_at)
                 VALUES ('BRK B', '2023-02-01', 1, 2, ?1)",
            rusqlite::params![ts],
        )
        .expect("failed to insert stock split");
    }
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,BRK B 16JUN23 330 C,2023-01-10,,SMART,卖出,1,2.00,200.00,0,0,LMT,O
a,BRK B 16JUN23 165 C,2023-06-10,,SMART,买入,1,0.10,10.00,0,0,C;P,C;P
";
    let result = import_options_csv_inner(&db, &account_id, csv).expect("import should succeed");
    assert_eq!(
        result.imported, 2,
        "split-adjusted close should match via split config"
    );
    assert!(
        result.errors.is_empty(),
        "expected no errors, got: {:?}",
        result.errors
    );
}

#[test]
fn test_normalize_action_english_variants() {
    assert_eq!(normalize_action("SELL"), "SELL");
    assert_eq!(normalize_action("SELL TO OPEN"), "SELL");
    assert_eq!(normalize_action("Buy to Close"), "BUY");
    assert_eq!(normalize_action("buy"), "BUY");
    assert_eq!(normalize_action("卖出开仓"), "SELL");
    assert_eq!(normalize_action("买入平仓"), "BUY");
    assert_eq!(normalize_action("unknown"), "");
}

/// An account whose records are all 'active' (e.g. only open positions,
/// or every close was rejected by the import boundary check) must not
/// cause get_option_contracts_inner to recompute endlessly and overflow
/// the stack — it should return the contracts normally.
#[test]
fn test_get_contracts_all_active_no_stack_overflow() {
    let (db, account_id) = db_with_account();
    let ts = chrono::Utc::now().to_rfc3339();
    {
        let conn = db.conn.lock().unwrap();
        // Open positions only — no close records, all contract_status = 'active'
        for (id, symbol, strike) in [
            ("o1", "AAPL 20FEB26 100 P", 100.0),
            ("o2", "TSLA 20MAR26 250 C", 250.0),
        ] {
            conn.execute(
                    "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'SELL', 'O', 1, 2.00, 200.00, 0, 0, '2026-01-15', NULL, ?8, 'active')",
                    rusqlite::params![id, account_id, symbol, symbol.split(' ').next().unwrap(), "20FEB26", strike, "P", ts],
                )
                .unwrap();
        }
    }
    let contracts = get_option_contracts_inner(&db, &account_id).expect("should not crash");
    assert_eq!(contracts.len(), 2, "both open contracts should be returned");
}

#[test]
fn test_get_contracts_projects_status_without_rewriting_rows() {
    let (db, account_id) = db_with_account();
    let created_at = chrono::Utc::now().to_rfc3339();
    {
        let conn = db.conn.lock().unwrap();
        for (id, action, code, quantity, price, traded_at) in [
            ("open", "SELL", "O", -1, 2.0, "2026-01-10"),
            ("close", "BUY", "C", 1, 0.5, "2026-01-20"),
        ] {
            conn.execute(
                "INSERT INTO option_records
                     (id, account_id, option_symbol, underlying, expiry_date, strike_price,
                      option_type, action, code, quantity, price, amount, commission, fee,
                      traded_at, created_at, contract_status)
                     VALUES (?1, ?2, 'ACME 20FEB26 100 P', 'ACME', '20FEB26', 100,
                             'P', ?3, ?4, ?5, ?6, 0, 0, 0, ?7, ?8, 'active')",
                rusqlite::params![
                    id, account_id, action, code, quantity, price, traded_at, created_at
                ],
            )
            .unwrap();
        }
    }

    let contracts = get_option_contracts_inner(&db, &account_id).unwrap();
    assert_eq!(contracts[0].status, "closed");
    assert_eq!(contracts[0].close_code.as_deref(), Some("C"));

    let persisted: String = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT contract_status FROM option_records WHERE id = 'open'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(persisted, "active");
}

#[test]
fn test_get_field_case_insensitive() {
    let headers = StringRecord::from(vec![
        "SYMBOL".to_string(),
        "Quantity".to_string(),
        "Trade Date/Time".to_string(),
    ]);
    let record = StringRecord::from(vec![
        "AAPL 20FEB26 100 P".to_string(),
        "2".to_string(),
        "2026-01-15 10:30:00".to_string(),
    ]);
    assert_eq!(
        get_field(&record, &headers, &["Symbol"]).as_deref(),
        Some("AAPL 20FEB26 100 P")
    );
    assert_eq!(
        get_field(&record, &headers, &["quantity"]).as_deref(),
        Some("2")
    );
    assert_eq!(
        get_field(&record, &headers, &["trade date/time"]).as_deref(),
        Some("2026-01-15 10:30:00")
    );
}

/// User-reported scenario: sell call 200 contracts (SELL O, qty -200),
/// buy back 100 (BUY code C), 100 expire (BUY code C;Ep). Total close qty
/// 200 matches open qty 200, so the open must NOT stay 'active'.
fn insert_857_scenario(conn: &rusqlite::Connection, account_id: &str, ts: &str) {
    for (id, action, code, qty, traded) in [
        ("r1", "SELL", "O", -200, "2023-09-06, 22:47:47"),
        ("r2", "BUY", "C", 100, "2023-09-13, 01:08:34"),
        ("r3", "BUY", "C;Ep", 100, "2023/10/30"),
    ] {
        conn.execute(
                "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
                 VALUES (?1, ?2, '857 30OCT23 6 C', '857', '30OCT23', 6.0, 'C', ?3, ?4, ?5, 0.1, 0.0, 0.0, 0.0, ?6, NULL, ?7, 'active')",
                rusqlite::params![id, account_id, action, code, qty, traded, ts],
            )
            .unwrap();
    }
}

#[test]
fn test_recompute_plain_c_close_matches_and_expires() {
    let (db, account_id) = db_with_account();
    let ts = chrono::Utc::now().to_rfc3339();
    {
        let conn = db.conn.lock().unwrap();
        insert_857_scenario(&conn, &account_id, &ts);
    }
    recompute_option_statuses(&db, &account_id).expect("recompute should succeed");
    let conn = db.conn.lock().unwrap();
    let status: String = conn
        .query_row(
            "SELECT contract_status FROM option_records WHERE id = 'r1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        status, "expired",
        "sell 200 with 100 closed (C) + 100 expired (C;Ep) should be expired, got {}",
        status
    );
}

#[test]
fn test_recompute_plain_c_close_only_marks_closed() {
    // SELL O 100 then BUY C 100 → fully closed via plain C code.
    let (db, account_id) = db_with_account();
    let ts = chrono::Utc::now().to_rfc3339();
    {
        let conn = db.conn.lock().unwrap();
        for (id, action, code, qty, traded) in [
            ("r1", "SELL", "O", -100, "2023-09-06, 22:47:47"),
            ("r2", "BUY", "C", 100, "2023-09-13, 01:08:34"),
        ] {
            conn.execute(
                    "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
                     VALUES (?1, ?2, '857 30OCT23 6 C', '857', '30OCT23', 6.0, 'C', ?3, ?4, ?5, 0.1, 0.0, 0.0, 0.0, ?6, NULL, ?7, 'active')",
                    rusqlite::params![id, account_id, action, code, qty, traded, ts],
                )
                .unwrap();
        }
    }
    recompute_option_statuses(&db, &account_id).expect("recompute should succeed");
    let conn = db.conn.lock().unwrap();
    let status: String = conn
        .query_row(
            "SELECT contract_status FROM option_records WHERE id = 'r1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        status, "closed",
        "sell 100 then buy back 100 (C) should be closed, got {}",
        status
    );
}

#[test]
fn test_partial_group_close_completes_oldest_open_fifo() {
    // Two opens share one option symbol. Closing 10 of 150 contracts must
    // complete the oldest 10-contract open while leaving the other 140 active.
    let (db, account_id) = db_with_account();
    let ts = chrono::Utc::now().to_rfc3339();
    {
        let conn = db.conn.lock().unwrap();
        for (id, action, code, qty, traded) in [
            ("o1", "SELL", "O", -10, "2024-10-01, 10:01:41"),
            ("o2", "SELL", "O", -140, "2024-10-01, 11:30:40"),
            ("c1", "BUY", "C", 10, "2024-12-04, 09:57:34"),
        ] {
            conn.execute(
                    "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
                     VALUES (?1, ?2, 'BABA 15JAN27 160 C', 'BABA', '15JAN27', 160.0, 'C', ?3, ?4, ?5, 1.0, 0.0, 0.0, 0.0, ?6, NULL, ?7, 'active')",
                    rusqlite::params![id, account_id, action, code, qty, traded, ts],
                )
                .unwrap();
        }
    }

    let contracts = get_option_contracts_inner(&db, &account_id)
        .expect("partial close should produce option contracts");
    let oldest = contracts
        .iter()
        .find(|contract| contract.id == "o1")
        .expect("oldest open should be returned");
    let remaining = contracts
        .iter()
        .find(|contract| contract.id == "o2")
        .expect("remaining open should be returned");

    assert_eq!(oldest.status, "closed");
    assert_eq!(oldest.close_code.as_deref(), Some("C"));
    assert_eq!(remaining.status, "active");
    assert_eq!(remaining.close_code, None);
    assert_eq!(
        contracts
            .iter()
            .filter(|contract| contract.status != "active")
            .map(|contract| contract.contracts.abs())
            .sum::<i64>(),
        10,
    );
    assert_eq!(
        contracts
            .iter()
            .filter(|contract| contract.status == "active")
            .map(|contract| contract.contracts.abs())
            .sum::<i64>(),
        140,
    );
}

#[test]
fn test_fifo_contracts_keep_each_opens_completing_close_details() {
    let (db, account_id) = db_with_account();
    let ts = chrono::Utc::now().to_rfc3339();
    {
        let conn = db.conn.lock().unwrap();
        for (id, action, code, qty, price, traded) in [
            ("o1", "SELL", "O", -10, 3.0, "2024-10-01, 10:01:41"),
            ("o2", "SELL", "O", -10, 4.0, "2024-10-01, 11:30:40"),
            ("c1", "BUY", "C", 10, 2.0, "2024-12-04, 09:57:34"),
            ("c2", "BUY", "C;Ep", 10, 0.0, "2025-01-15"),
        ] {
            conn.execute(
                    "INSERT INTO option_records (id, account_id, option_symbol, underlying, expiry_date, strike_price, option_type, action, code, quantity, price, amount, commission, fee, traded_at, settled_at, created_at, contract_status)
                     VALUES (?1, ?2, 'BABA 15JAN27 160 C', 'BABA', '15JAN27', 160.0, 'C', ?3, ?4, ?5, ?6, 0.0, 0.0, 0.0, ?7, NULL, ?8, 'active')",
                    rusqlite::params![id, account_id, action, code, qty, price, traded, ts],
                )
                .unwrap();
        }
    }

    let contracts = get_option_contracts_inner(&db, &account_id)
        .expect("completed FIFO opens should be returned");
    let oldest = contracts
        .iter()
        .find(|contract| contract.id == "o1")
        .expect("oldest open should be returned");
    let newest = contracts
        .iter()
        .find(|contract| contract.id == "o2")
        .expect("newest open should be returned");

    assert_eq!(oldest.status, "closed");
    assert_eq!(oldest.close_code.as_deref(), Some("C"));
    assert_eq!(oldest.close_price, Some(2.0));
    assert_eq!(newest.status, "expired");
    assert_eq!(newest.close_code.as_deref(), Some("C;Ep"));
    assert_eq!(newest.close_price, Some(0.0));
}

#[test]
fn test_export_round_trip_plain_c_close_matches() {
    // User-reported: export → clear → re-import must preserve matching.
    // SELL O 200, BUY C 100, BUY C;Ep 100 → open must be 'expired'.
    let (db, account_id) = db_with_account();
    let ts = chrono::Utc::now().to_rfc3339();
    {
        let conn = db.conn.lock().unwrap();
        insert_857_scenario(&conn, &account_id, &ts);
    }
    let csv = export_options_csv_inner(&db, &account_id).expect("export should succeed");
    // Clear all records, then re-import the exported CSV.
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM option_records WHERE account_id = ?1",
            rusqlite::params![account_id],
        )
        .unwrap();
    }
    let result = import_options_csv_inner(&db, &account_id, &csv).expect("import should succeed");
    assert_eq!(
        result.imported, 3,
        "all 3 rows re-imported, got {:?}",
        result.errors
    );
    // After import, recompute should mark the open (SELL O) as expired.
    recompute_option_statuses(&db, &account_id).expect("recompute should succeed");
    let conn = db.conn.lock().unwrap();
    let status: String = conn
        .query_row(
            "SELECT contract_status FROM option_records
                 WHERE account_id = ?1 AND action = 'SELL' AND option_symbol = '857 30OCT23 6 C'",
            rusqlite::params![account_id],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        status, "expired",
        "round-trip open should be expired, got {}",
        status
    );
}

#[test]
fn test_import_plain_c_close_accepted_with_open() {
    // Import boundary check must treat plain code C as a close record.
    let (db, account_id) = db_with_account();
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,857 30OCT23 6 C,2023-09-06,,SMART,卖出,200,0.13,52000.00,-204,0,LMT,O
a,857 30OCT23 6 C,2023-09-13,,SMART,买入,100,0.07,14000.00,-78,0,C,C
a,857 30OCT23 6 C,2023-10-30,,SMART,买入,100,0.00,0.00,0,0,C;Ep,C;Ep
";
    let result = import_options_csv_inner(&db, &account_id, csv).expect("import should succeed");
    assert_eq!(
        result.imported, 3,
        "open + C close + C;Ep close should all import"
    );
    assert!(
        result.errors.is_empty(),
        "expected no errors, got: {:?}",
        result.errors
    );
}
