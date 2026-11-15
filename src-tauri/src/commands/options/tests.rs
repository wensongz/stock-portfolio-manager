#[rustfmt::skip]
use super::*;
use super::contracts::recompute_option_statuses;
use super::csv::{
    export_options_csv_inner, get_field, import_options_csv_inner, normalize_action,
    preview_options_csv_inner,
};
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

fn insert_close_detail_record(
    db: &Database,
    account_id: &str,
    record: (&str, &str, &str, i64, f64, f64, f64, f64, &str),
) {
    let (id, action, code, quantity, price, amount, commission, fee, traded_at) = record;
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO option_records
             (id, account_id, option_symbol, underlying, expiry_date, strike_price,
              option_type, action, code, quantity, price, amount, commission, fee,
              traded_at, created_at, contract_status)
             VALUES (?1, ?2, 'BABA 15JAN27 160 C', 'BABA', '15JAN27', 160, 'C',
                     ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10, 'active')",
            rusqlite::params![
                id, account_id, action, code, quantity, price, amount, commission, fee, traded_at
            ],
        )
        .unwrap();
}

#[test]
fn test_tal_import_preserves_commission_rebates_in_completed_net_premium() {
    let (db, account_id) = db_with_account();
    let csv = "账户,股票,交易时间,交割时间,交易所,操作,股票数量,价格,金额,佣金,费用,类型,代码
a,TAL 20FEB26 10 P,2025-08-21,,SMART,卖出,-11,0.81,891,-17.10,0,LMT,O
a,TAL 20FEB26 10 P,2025-08-22,,SMART,卖出,-89,0.8,7120,41.45,0,LMT,O
a,TAL 15MAY26 10 P,2025-11-13,,SMART,卖出,-100,0.82,8200,35.13,0,LMT,O
a,TAL 21AUG26 10 P,2026-02-25,,SMART,卖出,-100,1.1,11000,-155.20,0,LMT,O;P
a,TAL 20FEB26 10 P,2026/2/20,,SMART,买入,100,0,0,0,0,LMT,C;Ep
a,TAL 15MAY26 10 P,2026/5/15,,SMART,买入,100,0,0,0,0,LMT,C;Ep
a,TAL 21AUG26 10 P,2026/8/21,,SMART,买入,100,0,0,0,0,LMT,C;Ep
";
    let imported = import_options_csv_inner(&db, &account_id, csv).unwrap();
    assert_eq!(imported.imported, 7);
    assert!(imported.errors.is_empty());
    let contracts = get_option_contracts_inner(&db, &account_id).unwrap();
    assert_eq!(contracts.len(), 4);
    assert!(contracts
        .iter()
        .all(|contract| contract.status == "expired"));
    assert!(
        (contracts
            .iter()
            .map(|contract| contract.commission)
            .sum::<f64>()
            + 95.72)
            .abs()
            < 1e-9
    );
    let report =
        crate::services::option_review_service::get_option_review(&db, &account_id, None).unwrap();
    assert_eq!(report.summary.completed_campaigns, 4);
    assert!((report.summary.completed_gross_premium - 27_211.0).abs() < 1e-9);
    assert!((report.summary.completed_net_premium_pnl - 27_115.28).abs() < 1e-9);
    let tal = &report.underlyings[0];
    assert!((tal.completed_net_premium_pnl - 27_115.28).abs() < 1e-9);
    let february: Vec<_> = tal
        .campaigns
        .iter()
        .filter(|campaign| campaign.option_symbol == "TAL 20FEB26 10 P")
        .collect();
    assert!((february.iter().map(|campaign| campaign.fees).sum::<f64>() + 24.35).abs() < 1e-9);
    assert!(
        (february
            .iter()
            .map(|campaign| campaign.net_premium_pnl.unwrap())
            .sum::<f64>()
            - 8_035.35)
            .abs()
            < 1e-9
    );
    assert!(
        (tal.campaigns
            .iter()
            .map(|campaign| campaign.fees)
            .sum::<f64>()
            - 95.72)
            .abs()
            < 1e-9
    );
}

#[test]
fn test_close_details_and_review_prorate_shared_commission_rebates() {
    let (db, account_id) = db_with_account();
    for record in [
        (
            "open-old",
            "SELL",
            "O",
            -6,
            10.0,
            6_000.0,
            -3.0,
            -1.0,
            "2026-08-01",
        ),
        (
            "open-new",
            "SELL",
            "O",
            -4,
            12.0,
            4_800.0,
            2.0,
            -0.5,
            "2026-08-02",
        ),
        (
            "expire-part",
            "BUY",
            "C;Ep",
            2,
            0.0,
            0.0,
            -0.5,
            -0.25,
            "2026-08-03",
        ),
        (
            "close-shared",
            "BUY",
            "C",
            6,
            4.0,
            -2_400.0,
            6.0,
            -3.0,
            "2026-08-04",
        ),
        (
            "close-final",
            "BUY",
            "C;P",
            2,
            2.0,
            -400.0,
            -1.0,
            -0.5,
            "2026-08-05",
        ),
    ] {
        insert_close_detail_record(&db, &account_id, record);
    }
    let contracts = get_option_contracts_inner(&db, &account_id).unwrap();
    let report =
        crate::services::option_review_service::get_option_review(&db, &account_id, None).unwrap();
    for (id, expected_fees, expected_pnl, expected_closing_commissions) in [
        ("open-old", 2.75, 4_397.25, vec![0.5, -4.0]),
        ("open-new", -1.0, 3_601.0, vec![-2.0, 1.0]),
    ] {
        let contract = contracts.iter().find(|contract| contract.id == id).unwrap();
        let details = contract.close_details.as_ref().unwrap();
        assert!((details.fees - expected_fees).abs() < 1e-9);
        assert!((details.realized_pnl - expected_pnl).abs() < 1e-9);
        assert_eq!(
            details
                .records
                .iter()
                .map(|record| record.commission)
                .collect::<Vec<_>>(),
            expected_closing_commissions
        );
        let campaign = report.underlyings[0]
            .campaigns
            .iter()
            .find(|campaign| campaign.id.ends_with(id))
            .unwrap();
        assert!((campaign.fees - expected_fees).abs() < 1e-9);
        assert!((campaign.net_premium_pnl.unwrap() - expected_pnl).abs() < 1e-9);
    }
    assert!((report.summary.net_premium_pnl - 7_998.25).abs() < 1e-9);
}

#[test]
fn test_partial_close_review_retains_rebates_on_active_opening() {
    let (db, account_id) = db_with_account();
    insert_close_detail_record(
        &db,
        &account_id,
        ("open", "SELL", "O", -2, 2.0, 400.0, 2.0, 0.0, "2026-08-01"),
    );
    insert_close_detail_record(
        &db,
        &account_id,
        ("close", "BUY", "C", 1, 0.5, -50.0, 0.5, 0.0, "2026-08-02"),
    );
    let report =
        crate::services::option_review_service::get_option_review(&db, &account_id, None).unwrap();
    let campaign = &report.underlyings[0].campaigns[0];
    assert_eq!(campaign.status, "active");
    assert!((campaign.fees + 2.5).abs() < 1e-9);
    assert!((campaign.net_premium_pnl.unwrap() - 352.5).abs() < 1e-9);
    assert!((report.summary.net_premium_pnl - 352.5).abs() < 1e-9);
    assert_eq!(report.summary.completed_net_premium_pnl, 0.0);
}

#[test]
fn test_closed_details_use_recorded_buyback_amount_for_baba_realized_pnl() {
    let (db, account_id) = db_with_account();
    insert_close_detail_record(
        &db,
        &account_id,
        (
            "open",
            "SELL",
            "O",
            -10,
            11.3,
            11_300.0,
            -4.9,
            0.0,
            "2026-08-01",
        ),
    );
    insert_close_detail_record(
        &db,
        &account_id,
        (
            "close",
            "BUY",
            "C",
            10,
            5.2,
            -5_200.0,
            -4.44,
            0.0,
            "2026-08-02",
        ),
    );

    let contracts = get_option_contracts_inner(&db, &account_id).unwrap();
    let contract = serde_json::to_value(&contracts[0]).unwrap();
    assert_eq!(contract["status"], "closed");
    let details = contract
        .get("close_details")
        .expect("closed contract must expose buyback details");
    assert_eq!(details["close_cost"], 5_200.0);
    assert!((details["fees"].as_f64().unwrap() - 9.34).abs() < 1e-9);
    assert!((details["realized_pnl"].as_f64().unwrap() - 6_090.66).abs() < 1e-9);
    assert_eq!(
        details["records"],
        serde_json::json!([{
            "id": "close", "code": "C", "quantity": 10, "price": 5.2,
            "amount": 5_200.0, "commission": 4.44, "fee": 0.0,
            "traded_at": "2026-08-02"
        }])
    );
}

#[test]
fn test_closed_details_prorate_shared_fifo_closes_and_keep_chronological_records() {
    let (db, account_id) = db_with_account();
    for record in [
        (
            "open-old",
            "SELL",
            "O",
            -6,
            10.0,
            6_000.0,
            -3.0,
            -1.0,
            "2026-08-01",
        ),
        (
            "open-new",
            "SELL",
            "O",
            -4,
            12.0,
            4_800.0,
            -2.0,
            -0.5,
            "2026-08-02",
        ),
        (
            "close-final",
            "BUY",
            "C;P",
            2,
            2.0,
            -400.0,
            -1.0,
            -0.5,
            "2026-08-05",
        ),
        (
            "close-shared",
            "BUY",
            "C",
            6,
            4.0,
            -2_400.0,
            -6.0,
            -3.0,
            "2026-08-04",
        ),
        (
            "expire-part",
            "BUY",
            "C;Ep",
            2,
            0.0,
            0.0,
            -0.5,
            -0.25,
            "2026-08-03",
        ),
    ] {
        insert_close_detail_record(&db, &account_id, record);
    }

    let contracts = get_option_contracts_inner(&db, &account_id).unwrap();
    let oldest = contracts
        .iter()
        .find(|contract| contract.id == "open-old")
        .unwrap();
    let newest = contracts
        .iter()
        .find(|contract| contract.id == "open-new")
        .unwrap();
    let oldest = serde_json::to_value(oldest).unwrap();
    let newest = serde_json::to_value(newest).unwrap();
    let old_details = oldest
        .get("close_details")
        .expect("oldest closed batch needs allocations");
    let new_details = newest
        .get("close_details")
        .expect("newest closed batch needs allocations");

    assert_eq!(old_details["close_cost"], 1_600.0);
    assert_eq!(old_details["fees"], 10.75);
    assert_eq!(old_details["realized_pnl"], 4_389.25);
    assert_eq!(
        old_details["records"],
        serde_json::json!([
            { "id": "expire-part", "code": "C;Ep", "quantity": 2, "price": 0.0,
              "amount": 0.0, "commission": 0.5, "fee": 0.25, "traded_at": "2026-08-03" },
            { "id": "close-shared", "code": "C", "quantity": 4, "price": 4.0,
              "amount": 1_600.0, "commission": 4.0, "fee": 2.0, "traded_at": "2026-08-04" }
        ])
    );
    assert_eq!(new_details["close_cost"], 1_200.0);
    assert_eq!(new_details["fees"], 7.0);
    assert_eq!(new_details["realized_pnl"], 3_593.0);
    assert_eq!(
        new_details["records"],
        serde_json::json!([
            { "id": "close-shared", "code": "C", "quantity": 2, "price": 4.0,
              "amount": 800.0, "commission": 2.0, "fee": 1.0, "traded_at": "2026-08-04" },
            { "id": "close-final", "code": "C;P", "quantity": 2, "price": 2.0,
              "amount": 400.0, "commission": 1.0, "fee": 0.5, "traded_at": "2026-08-05" }
        ])
    );
}

#[test]
fn test_completed_details_keep_partial_buyback_before_expiry_or_assignment() {
    for (terminal_code, expected_status) in [("C;Ep", "expired"), ("A;C", "assigned")] {
        let (db, account_id) = db_with_account();
        let csv = format!(
            "账户,股票,交易时间,操作,股票数量,价格,金额,佣金,费用,代码\n\
             a,BABA 16SEP22 170 C,2022-06-28,卖出,-50,2.53,12650,-34.86,0,O\n\
             a,BABA 16SEP22 170 C,2022-07-28,买入,40,0.15,-600,-47.54,0,C;P\n\
             a,BABA 16SEP22 170 C,2022/9/16,买入,10,0,0,0,0,{terminal_code}\n"
        );
        let imported = import_options_csv_inner(&db, &account_id, &csv).unwrap();
        assert_eq!(imported.imported, 3);
        assert!(imported.errors.is_empty());
        let contracts = get_option_contracts_inner(&db, &account_id).unwrap();
        assert_eq!(contracts.len(), 1);
        let contract = &contracts[0];
        assert_eq!(contract.status, expected_status);
        assert_eq!(contract.remaining_contracts, 0);
        let details = contract
            .close_details
            .as_ref()
            .expect("earlier buyback must remain visible after final expiry or assignment");
        assert_eq!(details.records.len(), 2);
        assert_eq!(details.records[0].code, "C;P");
        assert_eq!(details.records[0].quantity, 40);
        assert_eq!(details.records[0].amount, 600.0);
        assert_eq!(details.records[0].commission, 47.54);
        assert_eq!(details.records[1].code, terminal_code);
        assert_eq!(details.records[1].quantity, 10);
        assert_eq!(details.close_cost, 600.0);
        assert!((details.fees - 82.40).abs() < 1e-9);
        assert!((details.realized_pnl - 11_967.60).abs() < 1e-9);
        let report =
            crate::services::option_review_service::get_option_review(&db, &account_id, None)
                .unwrap();
        assert_eq!(report.summary.completed_campaigns, 1);
        assert!((report.summary.completed_net_premium_pnl - 11_967.60).abs() < 1e-9);
        assert!((report.summary.completed_net_premium_pnl - details.realized_pnl).abs() < 1e-9);
    }
}

#[test]
fn test_close_details_are_null_for_active_expired_and_assigned_contracts() {
    for (close_code, close_quantity, expected_status) in [
        ("C", 1, "active"),
        ("C;Ep", 2, "expired"),
        ("A;C", 2, "assigned"),
    ] {
        let (db, account_id) = db_with_account();
        insert_close_detail_record(
            &db,
            &account_id,
            ("open", "SELL", "O", -2, 2.0, 400.0, -1.0, 0.0, "2026-08-01"),
        );
        insert_close_detail_record(
            &db,
            &account_id,
            (
                "close",
                "BUY",
                close_code,
                close_quantity,
                0.0,
                0.0,
                0.0,
                0.0,
                "2026-08-02",
            ),
        );

        let contracts = get_option_contracts_inner(&db, &account_id).unwrap();
        let contract = serde_json::to_value(&contracts[0]).unwrap();
        assert_eq!(contract["status"], expected_status);
        assert_eq!(
            contract.get("close_details"),
            Some(&serde_json::Value::Null)
        );
    }
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

#[test]
fn test_options_csv_preview_serializes_only_accepted_rows_with_original_values() {
    let (db, account_id) = db_with_account();
    let csv = "账户,股票,交易时间,操作,股票数量,价格,金额,佣金,费用,代码
a,AAPL 20FEB26 100 P,\"2026-01-15, 10:30:00\",卖出,-2,1.234567,246.9134,-0.123456,0.00005,O
Total,,,,,,,,,
a,AAPL 20FEB26 100 P,2026-01-15,卖出,1,invalid,200,0,0,O
a,MSFT 20FEB26 100 P,2026-02-20,买入,1,0.10,10,0,0,C
a,AAPL 20FEB26 100 P,2026-02-20,Buy to Close,2,0.987654,-197.5308,-1.005,0.04,C
a,,2026-01-15,卖出,1,2.00,200,0,0,O
a,BRK B 20MAR26 330 C,,买入,1,12.123456789,,,,O
";
    db.conn
        .lock()
        .unwrap()
        .pragma_update(None, "query_only", true)
        .unwrap();

    let preview = preview_options_csv_inner(&db, &account_id, csv).unwrap();
    assert_eq!(preview.total_rows, 7);
    assert_eq!(preview.importable, 3);
    assert_eq!(preview.skipped, 4);
    assert_eq!(preview.errors.len(), 2);
    let serialized = serde_json::to_value(preview).unwrap();
    assert_eq!(
        serialized["rows"],
        serde_json::json!([
            {
                "row_number": 2,
                "option_symbol": "AAPL 20FEB26 100 P",
                "traded_at": "2026-01-15, 10:30:00",
                "action": "SELL",
                "code": "O",
                "quantity": -2,
                "price": 1.234567,
                "amount": 246.9134,
                "commission": -0.123456,
                "fee": 0.00005,
            },
            {
                "row_number": 6,
                "option_symbol": "AAPL 20FEB26 100 P",
                "traded_at": "2026-02-20",
                "action": "BUY",
                "code": "C",
                "quantity": 2,
                "price": 0.987654,
                "amount": -197.5308,
                "commission": -1.005,
                "fee": 0.04,
            },
            {
                "row_number": 8,
                "option_symbol": "BRK B 20MAR26 330 C",
                "traded_at": null,
                "action": "BUY",
                "code": "O",
                "quantity": 1,
                "price": 12.123456789,
                "amount": 0.0,
                "commission": 0.0,
                "fee": 0.0,
            },
        ])
    );
    let count: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM option_records", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 0, "preview rows must not be inserted");
}

#[test]
fn test_options_csv_preview_rows_exclude_excess_closes_after_fifo_validation() {
    let (db, account_id) = db_with_account();
    insert_exposure_record(
        &db,
        &account_id,
        "P",
        "existing-open",
        1,
        true,
        "2026-08-01",
    );
    let csv = "Symbol,Trade Date,Action,Quantity,Price,Code
AAPL 18SEP26 100 P,2026-08-02,BUY,2,0.5,C
";

    let preview = preview_options_csv_inner(&db, &account_id, csv).unwrap();
    assert_eq!(preview.importable, 0);
    assert_eq!(preview.skipped, 1);
    assert_eq!(preview.errors.len(), 1);
    assert_eq!(
        serde_json::to_value(preview).unwrap()["rows"],
        serde_json::json!([])
    );
}

#[test]
fn test_options_csv_preview_counts_every_row_using_import_validation() {
    let (db, account_id) = db_with_account();
    let csv = "账户,股票,交易时间,操作,股票数量,价格,金额,佣金,费用,代码
a,AAPL 20FEB26 100 P,2026-01-15,卖出,1,2.00,200,0,0,O
a,AAPL 20FEB26 100 P,2026-02-20,买入,1,0.10,10,0,0,C
a,AAPL 20FEB26 100 P,2026-01-15,卖出,1,oops,200,0,0,O
a,AAPL 20FEB26 100 P,2026-01-15,卖出,1.5,2.00,200,0,0,O
a,AAPL 20FEB26 100 P,2026-01-15,卖出,1,2.00,NaN,0,0,O
a,MSFT 20FEB26 100 P,2026-02-20,买入,1,0.10,10,0,0,C
Total,,,,,,,,,
a,,2026-01-15,卖出,1,2.00,200,0,0,O
,AAPL 20FEB26 100 P,2026-01-15,卖出,1,2.00,200,0,0,O
";

    let preview = preview_options_csv_inner(&db, &account_id, csv).unwrap();
    assert_eq!(preview.total_rows, 9);
    assert_eq!(preview.importable, 2);
    assert_eq!(preview.skipped, 7);
    assert_eq!(preview.errors.len(), 4);
    assert!(preview
        .errors
        .iter()
        .any(|error| error.contains("Row 4") && error.contains("price")));
    assert!(preview
        .errors
        .iter()
        .any(|error| error.contains("Row 5") && error.contains("quantity")));
    assert!(preview
        .errors
        .iter()
        .any(|error| error.contains("Row 6") && error.contains("amount")));
    assert!(preview
        .errors
        .iter()
        .any(|error| error.contains("Row 7") && error.contains("no matching open")));

    let imported = import_options_csv_inner(&db, &account_id, csv).unwrap();
    assert_eq!(imported.imported, 2);
    assert_eq!(
        imported.skipped, 3,
        "legacy result counts ordinary skips separately"
    );
    assert_eq!(imported.errors, preview.errors);
    assert_eq!(preview.skipped, imported.skipped + imported.errors.len());
}

#[test]
fn test_options_csv_preview_matches_existing_records_without_any_database_writes() {
    let (db, account_id) = db_with_account();
    insert_exposure_record(
        &db,
        &account_id,
        "P",
        "existing-open",
        3,
        true,
        "2026-08-01",
    );
    let csv = "Symbol,Trade Date,Action,Quantity,Price,Code
AAPL 18SEP26 100 P,2026-08-02,BUY,3,0.5,C
";
    {
        let conn = db.conn.lock().unwrap();
        conn.pragma_update(None, "query_only", true).unwrap();
    }

    let preview = preview_options_csv_inner(&db, &account_id, csv)
        .expect("preview must work on a query-only database");
    assert_eq!(preview.total_rows, 1);
    assert_eq!(preview.importable, 1);
    assert_eq!(preview.skipped, 0);
    assert!(preview.errors.is_empty());
    {
        let conn = db.conn.lock().unwrap();
        let (count, status): (i64, String) = conn
            .query_row(
                "SELECT COUNT(*), contract_status FROM option_records",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(count, 1, "preview must not insert the close");
        assert_eq!(
            status, "active",
            "preview must not update the open's status"
        );
        conn.pragma_update(None, "query_only", false).unwrap();
    }

    let imported = import_options_csv_inner(&db, &account_id, csv).unwrap();
    assert_eq!(imported.imported, 1);
    let status: String = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT contract_status FROM option_records WHERE id = 'existing-open'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(status, "closed", "confirmed import still recomputes status");
}

#[test]
fn test_options_csv_preview_does_not_match_another_accounts_open() {
    let (db, account_id) = db_with_account();
    insert_exposure_record(
        &db,
        &account_id,
        "P",
        "existing-open",
        1,
        true,
        "2026-08-01",
    );
    let csv = "Symbol,Trade Date,Action,Quantity,Price,Code
AAPL 18SEP26 100 P,2026-08-02,BUY,1,0.5,C
";

    let preview = preview_options_csv_inner(&db, "another-account", csv).unwrap();
    assert_eq!(preview.total_rows, 1);
    assert_eq!(preview.importable, 0);
    assert_eq!(preview.skipped, 1);
    assert_eq!(preview.errors.len(), 1);
    assert!(preview.errors[0].contains("no matching open"));
}

#[test]
fn test_options_csv_preview_preserves_duplicate_open_import_behavior() {
    let (db, account_id) = db_with_account();
    let csv = "Symbol,Trade Date,Action,Quantity,Price,Code
AAPL 18SEP26 100 P,2026-08-01,SELL,1,2,O
AAPL 18SEP26 100 P,2026-08-01,SELL,1,2,O
";

    let preview = preview_options_csv_inner(&db, &account_id, csv).unwrap();
    assert_eq!(preview.importable, 2);
    assert_eq!(preview.skipped, 0);
    let imported = import_options_csv_inner(&db, &account_id, csv).unwrap();
    assert_eq!(imported.imported, 2);
    assert_eq!(imported.skipped, 0);
    assert!(imported.errors.is_empty());

    let repeated_preview = preview_options_csv_inner(&db, &account_id, csv).unwrap();
    assert_eq!(repeated_preview.importable, 2);
    let repeated_import = import_options_csv_inner(&db, &account_id, csv).unwrap();
    assert_eq!(
        repeated_import.imported, 2,
        "a prior import must not cause ID collisions"
    );
    let count: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM option_records", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 4);
}

#[test]
fn test_options_csv_preview_avoids_existing_ids_in_other_accounts() {
    let (db, account_id) = db_with_account();
    let csv = "Symbol,Trade Date,Action,Quantity,Price,Code
AAPL 18SEP26 100 P,2026-08-01,SELL,1,2,O
";
    assert_eq!(
        import_options_csv_inner(&db, &account_id, csv)
            .unwrap()
            .imported,
        1
    );
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at)
             SELECT 'another-account', 'Another Account', market, created_at, updated_at
             FROM accounts WHERE id = ?1",
            [&account_id],
        )
        .unwrap();
        conn.execute(
            "UPDATE option_records SET account_id = 'another-account'",
            [],
        )
        .unwrap();
    }

    let preview = preview_options_csv_inner(&db, &account_id, csv).unwrap();
    assert_eq!(preview.importable, 1);
    let imported = import_options_csv_inner(&db, &account_id, csv)
        .expect("IDs in other accounts must not collide with this import");
    assert_eq!(imported.imported, 1);
    let count: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row("SELECT COUNT(*) FROM option_records", [], |row| row.get(0))
        .unwrap();
    assert_eq!(count, 2);
}

#[test]
fn test_options_csv_preview_uses_stable_order_for_competing_same_time_closes() {
    let (db, account_id) = db_with_account();
    insert_exposure_record(
        &db,
        &account_id,
        "P",
        "existing-open",
        3,
        true,
        "2026-08-01",
    );
    let csv = "Symbol,Trade Date,Action,Quantity,Price,Code
AAPL 18SEP26 100 P,2026-08-02,BUY,3,0.5,C
AAPL 18SEP26 100 P,2026-08-02,BUY,1,0.5,C
AAPL 18SEP26 100 P,2026-08-02,BUY,1,0.5,C
";

    for _ in 0..10 {
        let preview = preview_options_csv_inner(&db, &account_id, csv).unwrap();
        assert_eq!(preview.total_rows, 3);
        assert_eq!(preview.importable, 1);
        assert_eq!(preview.skipped, 2);
    }
    let imported = import_options_csv_inner(&db, &account_id, csv).unwrap();
    assert_eq!(imported.imported, 1);
    assert_eq!(imported.errors.len(), 2);
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
