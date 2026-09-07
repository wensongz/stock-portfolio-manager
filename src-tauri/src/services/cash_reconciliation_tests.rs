use super::*;

fn fixture() -> Database {
    let db = Database::new(":memory:").unwrap();
    db.conn.lock().unwrap().execute_batch(
        "INSERT INTO accounts VALUES ('a','Account A','US',NULL,'2025-01-01','2025-01-01');
         INSERT INTO accounts VALUES ('b','Account B','US',NULL,'2025-01-01','2025-01-01');
         INSERT INTO holdings VALUES ('cash','a','$CASH-USD','Cash','US',NULL,10000,1,'USD','2025-02-01','2025-02-01');",
    ).unwrap();
    db
}

#[allow(clippy::too_many_arguments)]
fn transaction(
    db: &Database,
    id: &str,
    account: &str,
    symbol: &str,
    currency: &str,
    kind: &str,
    amount: f64,
    fee: f64,
    date: &str,
) {
    db.conn.lock().unwrap().execute(
        "INSERT INTO transactions(id,holding_id,account_id,symbol,name,market,transaction_type,shares,price,total_amount,commission,currency,traded_at,notes,created_at)
         VALUES (?1,NULL,?2,?3,?3,'US',?4,?5,1,?5,?6,?7,?8,'original source',?8)",
        rusqlite::params![id,account,symbol,kind,amount,fee,currency,date],
    ).unwrap();
}

fn basic_ledger(db: &Database) {
    transaction(
        db,
        "deposit",
        "a",
        "$CASH-USD",
        "USD",
        "BUY",
        1000.0,
        2.0,
        "2025-01-01",
    );
    transaction(
        db,
        "stock-buy",
        "a",
        "AAPL",
        "USD",
        "BUY",
        100.0,
        3.0,
        "2025-01-02",
    );
    transaction(
        db,
        "stock-sell",
        "a",
        "AAPL",
        "USD",
        "SELL",
        50.0,
        1.0,
        "2025-01-03",
    );
    transaction(
        db,
        "dividend",
        "a",
        "AAPL",
        "USD",
        "PAY",
        20.0,
        2.0,
        "2025-01-04",
    );
    transaction(
        db,
        "withdraw",
        "a",
        "$CASH-USD",
        "USD",
        "SELL",
        10.0,
        1.0,
        "2025-01-05",
    );
}

fn transaction_dump(db: &Database) -> Vec<Vec<String>> {
    let conn = db.conn.lock().unwrap();
    let mut statement = conn
        .prepare("SELECT * FROM transactions ORDER BY id")
        .unwrap();
    let columns = statement.column_count();
    statement
        .query_map([], |row| {
            (0..columns)
                .map(|index| row.get_ref(index).map(|value| format!("{value:?}")))
                .collect()
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap()
}

fn correct(db: &Database, balance: f64) -> Result<Holding, String> {
    let preview = get_cash_balance_reconciliation(db, "cash")?;
    correct_cash_balance(
        db,
        "cash",
        balance,
        preview.revision,
        "Corrected cash".into(),
        None,
    )
}

#[test]
fn preview_uses_one_sqlite_snapshot_during_external_connection_writes() {
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("cash-preview.sqlite");
    let db = Database::new(path.to_str().unwrap()).unwrap();
    db.conn.lock().unwrap().execute_batch(
        "PRAGMA journal_mode=WAL;
         INSERT INTO accounts VALUES('a','Account','US',NULL,'2025-01-01','2025-01-01');
         INSERT INTO holdings VALUES('cash','a','$CASH-USD','Cash','US',NULL,10,1,'USD','2025-01-01','2025-01-01');",
    ).unwrap();
    transaction(
        &db,
        "deposit",
        "a",
        "$CASH-USD",
        "USD",
        "BUY",
        10.0,
        0.0,
        "2025-01-01",
    );
    let initial_revision =
        crate::services::snapshot_cache_service::current_revision(&db.conn.lock().unwrap())
            .unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let started = Arc::new(AtomicBool::new(false));
    let writer_stop = stop.clone();
    let writer_started = started.clone();
    let writer = std::thread::spawn(move || {
        let mut conn = Connection::open(path).unwrap();
        conn.busy_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let mut amount = 20.0;
        while !writer_stop.load(Ordering::Relaxed) {
            let tx = conn
                .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
                .unwrap();
            tx.execute("UPDATE holdings SET shares=?1 WHERE id='cash'", [amount])
                .unwrap();
            tx.execute(
                "UPDATE transactions SET shares=?1,total_amount=?1 WHERE id='deposit'",
                [amount],
            )
            .unwrap();
            tx.commit().unwrap();
            writer_started.store(true, Ordering::Release);
            amount = if amount == 20.0 { 10.0 } else { 20.0 };
        }
    });
    while !started.load(Ordering::Acquire) {
        std::thread::yield_now();
    }
    let mut mixed_snapshot = None;
    let mut preview_error = None;
    for _ in 0..300 {
        let preview = match get_cash_balance_reconciliation(&db, "cash") {
            Ok(preview) => preview,
            Err(error) => {
                preview_error = Some(error);
                break;
            }
        };
        // Each atomic writer commit updates two rows and advances the revision twice.
        let commits = (preview.revision - initial_revision) / 2;
        let expected_balance = if commits % 2 == 0 { 10.0 } else { 20.0 };
        if preview.recommended_balance != Some(preview.current_balance)
            || preview.current_balance != expected_balance
        {
            mixed_snapshot = Some((
                preview.current_balance,
                preview.recommended_balance,
                preview.revision,
            ));
            break;
        }
    }
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
    assert_eq!(preview_error, None);
    assert_eq!(
        mixed_snapshot, None,
        "one preview must not mix two committed ledger states"
    );
}

#[test]
fn cash_preview_uses_full_account_currency_ledger_and_backend_fees() {
    let db = fixture();
    basic_ledger(&db);
    transaction(
        &db,
        "other-account",
        "b",
        "$CASH-USD",
        "USD",
        "BUY",
        9000.0,
        0.0,
        "2025-01-01",
    );
    transaction(
        &db,
        "other-currency",
        "a",
        "$CASH-HKD",
        "HKD",
        "BUY",
        9000.0,
        0.0,
        "2025-01-01",
    );
    for kind in ["OPEN", "STOCK_IN", "STOCK_OUT"] {
        transaction(
            &db,
            kind,
            "a",
            "AAPL",
            "USD",
            kind,
            8000.0,
            0.0,
            "2025-01-01",
        );
    }
    let before = transaction_dump(&db);
    let preview = get_cash_balance_reconciliation(&db, "cash").unwrap();
    assert_eq!(preview.recommended_balance, Some(955.0));
    assert_eq!(preview.difference, Some(-9045.0));
    assert_eq!(preview.opening_count, 0);
    assert_eq!(preview.rows.len(), 5);
    assert_eq!(preview.rows[0].transaction.id, "withdraw");
    assert_eq!(preview.rows[0].running_balance, 955.0);
    assert_eq!(preview.rows[4].cash_delta, 1002.0);
    let json = serde_json::to_value(&preview.rows[0]).unwrap();
    assert_eq!(json["symbol"], "$CASH-USD");
    assert!(json.get("transaction").is_none());
    assert_eq!(transaction_dump(&db), before);
}

#[test]
fn cash_openings_reset_balances_and_utc_order_is_used() {
    let db = fixture();
    transaction(
        &db,
        "deposit",
        "a",
        "$CASH-USD",
        "USD",
        "BUY",
        100.0,
        0.0,
        "2025-03-31T23:30:00+08:00",
    );
    transaction(
        &db,
        "opening",
        "a",
        "$CASH-USD",
        "USD",
        "OPEN",
        500.0,
        0.0,
        "2025-03-31T20:00:00Z",
    );
    transaction(
        &db,
        "buy",
        "a",
        "AAPL",
        "USD",
        "BUY",
        50.0,
        0.0,
        "2025-04-01T05:00:00+08:00",
    );
    let ledger = load_cash_ledger(&db.conn.lock().unwrap(), "a", "USD").unwrap();
    assert_eq!(ledger.recommended_balance, Some(450.0));
    assert_eq!(ledger.opening_count, 1);
    assert_eq!(ledger.rows[1].cash_delta, 400.0);
    assert_eq!(
        ledger.balance_at_date(NaiveDate::from_ymd_opt(2025, 3, 31).unwrap()),
        Some(450.0)
    );
    assert_eq!(
        ledger.balance_at_date(NaiveDate::from_ymd_opt(2025, 3, 30).unwrap()),
        Some(0.0)
    );
}

#[test]
fn equal_trade_times_order_opening_and_flow_by_utc_creation_time() {
    let db = fixture();
    transaction(
        &db,
        "opening",
        "a",
        "$CASH-USD",
        "USD",
        "OPEN",
        500.0,
        0.0,
        "2025-01-10T00:00:00Z",
    );
    transaction(
        &db,
        "deposit",
        "a",
        "$CASH-USD",
        "USD",
        "BUY",
        100.0,
        0.0,
        "2025-01-10T00:00:00Z",
    );
    db.conn
        .lock()
        .unwrap()
        .execute_batch(
            "UPDATE transactions SET created_at='2025-01-10T23:00:00+08:00' WHERE id='opening';
         UPDATE transactions SET created_at='2025-01-10T20:00:00Z' WHERE id='deposit';",
        )
        .unwrap();
    let ledger = load_cash_ledger(&db.conn.lock().unwrap(), "a", "USD").unwrap();
    assert_eq!(ledger.recommended_balance, Some(600.0));
    assert_eq!(ledger.rows[0].transaction.id, "opening");
}

#[test]
fn no_money_history_has_no_recommendation() {
    let db = fixture();
    transaction(
        &db,
        "stock-open",
        "a",
        "AAPL",
        "USD",
        "OPEN",
        100.0,
        0.0,
        "2025-01-01",
    );
    let preview = get_cash_balance_reconciliation(&db, "cash").unwrap();
    assert_eq!(preview.recommended_balance, None);
    assert_eq!(preview.difference, None);
    assert!(preview.rows.is_empty());
}

#[test]
fn adopting_recommendation_only_repairs_holding_and_invalidates_daily_cache() {
    let db = fixture();
    basic_ledger(&db);
    db.conn.lock().unwrap().execute_batch(
        "INSERT INTO daily_portfolio_values(date,total_value) VALUES('2025-01-01',999);
         INSERT INTO daily_holding_snapshots(date,account_id,symbol,market) VALUES('2025-01-01','a','$CASH-USD','US');
         INSERT INTO quarterly_snapshots(id,quarter,snapshot_date,created_at) VALUES('q','2025-Q1','2025-03-31','2025-04-01');",
    ).unwrap();
    let before = transaction_dump(&db);
    let holding = correct(&db, 955.0).unwrap();
    assert_eq!(holding.shares, 955.0);
    assert_eq!(holding.avg_cost, 1.0);
    assert_eq!(holding.name, "Corrected cash");
    assert_eq!(transaction_dump(&db), before);
    let conn = db.conn.lock().unwrap();
    for table in ["daily_portfolio_values", "daily_holding_snapshots"] {
        assert_eq!(
            conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM quarterly_snapshots", [], |row| row
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn adopting_display_rounded_recommendation_preserves_full_ledger_precision() {
    let db = fixture();
    transaction(
        &db,
        "deposit",
        "a",
        "$CASH-USD",
        "USD",
        "BUY",
        10.004,
        0.0,
        "2025-01-01",
    );
    let before = transaction_dump(&db);
    assert_eq!(correct(&db, 10.0).unwrap().shares, 10.004);
    assert_eq!(transaction_dump(&db), before);
}

#[test]
fn custom_balance_persists_only_opening_difference_before_real_flows() {
    let db = fixture();
    basic_ledger(&db);
    let before = transaction_dump(&db);
    assert_eq!(correct(&db, 1000.0).unwrap().shares, 1000.0);
    let preview = get_cash_balance_reconciliation(&db, "cash").unwrap();
    assert_eq!(preview.recommended_balance, Some(1000.0));
    assert_eq!(preview.opening_count, 1);
    let opening = preview.rows.last().unwrap();
    assert_eq!(opening.transaction.transaction_type, "OPEN");
    assert_eq!(opening.transaction.shares, 45.0);
    assert_eq!(opening.transaction.total_amount, 45.0);
    assert_eq!(opening.transaction.price, 1.0);
    assert_eq!(opening.transaction.commission, 0.0);
    assert!(opening
        .transaction
        .notes
        .as_deref()
        .unwrap()
        .contains("校正"));
    let conn = db.conn.lock().unwrap();
    let order: bool = conn
        .query_row(
            "SELECT JULIANDAY(?1) < JULIANDAY('2025-01-01')",
            [&opening.transaction.traded_at],
            |row| row.get(0),
        )
        .unwrap();
    assert!(order);
    drop(conn);
    let after = transaction_dump(&db);
    assert!(before.iter().all(|row| after.contains(row)));
}

#[test]
fn repeated_custom_balance_updates_single_opening_and_survives_replay() {
    let db = fixture();
    transaction(
        &db,
        "opening",
        "a",
        "$CASH-USD",
        "USD",
        "OPEN",
        400.0,
        0.0,
        "2025-01-01",
    );
    transaction(
        &db,
        "deposit",
        "a",
        "$CASH-USD",
        "USD",
        "BUY",
        50.0,
        0.0,
        "2025-01-02",
    );
    assert_eq!(correct(&db, 475.0).unwrap().shares, 475.0);
    assert_eq!(correct(&db, 0.0).unwrap().shares, 0.0);
    assert_eq!(correct(&db, -25.0).unwrap().shares, -25.0);
    let preview = get_cash_balance_reconciliation(&db, "cash").unwrap();
    assert_eq!(preview.recommended_balance, Some(-25.0));
    assert_eq!(preview.opening_count, 1);
    let opening = preview.rows.last().unwrap();
    assert_eq!(opening.transaction.id, "opening");
    assert_eq!(opening.transaction.shares, -75.0);
    assert_eq!(opening.transaction.traded_at, "2025-01-01");
    assert_eq!(opening.transaction.created_at, "2025-01-01");
    assert_eq!(
        opening.transaction.notes.as_deref(),
        Some("original source")
    );
}

#[test]
fn corrected_cash_stays_consistent_after_transaction_create_edit_and_delete() {
    use crate::services::portfolio_mutation::{
        create_transaction_in, delete_transaction_in, update_transaction_in, CreateTransactionInput,
    };
    let db = fixture();
    basic_ledger(&db);
    correct(&db, 1000.0).unwrap();
    let mut input = CreateTransactionInput {
        account_id: "a".into(),
        symbol: "$CASH-USD".into(),
        name: "Cash".into(),
        market: "US".into(),
        transaction_type: "BUY".into(),
        shares: 20.0,
        price: 1.0,
        total_amount: 20.0,
        commission: 1.0,
        currency: "USD".into(),
        traded_at: "2025-03-01".into(),
        notes: None,
    };
    let created = {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        let created = create_transaction_in(&tx, &input).unwrap();
        tx.commit().unwrap();
        created
    };
    let preview = get_cash_balance_reconciliation(&db, "cash").unwrap();
    assert_eq!(preview.current_balance, 1021.0);
    assert_eq!(preview.recommended_balance, Some(1021.0));
    input.shares = 30.0;
    input.total_amount = 30.0;
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        update_transaction_in(&tx, &created.id, &input).unwrap();
        tx.commit().unwrap();
    }
    let preview = get_cash_balance_reconciliation(&db, "cash").unwrap();
    assert_eq!(preview.current_balance, 1031.0);
    assert_eq!(preview.recommended_balance, Some(1031.0));
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        delete_transaction_in(&tx, &created.id).unwrap();
        tx.commit().unwrap();
    }
    let preview = get_cash_balance_reconciliation(&db, "cash").unwrap();
    assert_eq!(preview.current_balance, 1000.0);
    assert_eq!(preview.recommended_balance, Some(1000.0));
    assert_eq!(preview.opening_count, 1);
}

#[test]
fn multiple_openings_allow_recommended_repair_but_reject_custom_changes() {
    let db = fixture();
    for (id, amount, date) in [("one", 100.0, "2025-01-01"), ("two", 200.0, "2025-02-01")] {
        transaction(&db, id, "a", "$CASH-USD", "USD", "OPEN", amount, 0.0, date);
    }
    let before = transaction_dump(&db);
    assert_eq!(correct(&db, 200.0).unwrap().shares, 200.0);
    assert!(correct(&db, 300.0).unwrap_err().contains("期初"));
    assert_eq!(transaction_dump(&db), before);
}

#[test]
fn stale_preview_is_rejected_without_overwriting_a_new_transaction() {
    let db = fixture();
    basic_ledger(&db);
    let preview = get_cash_balance_reconciliation(&db, "cash").unwrap();
    transaction(
        &db,
        "later",
        "a",
        "$CASH-USD",
        "USD",
        "BUY",
        20.0,
        0.0,
        "2025-02-01",
    );
    assert!(
        correct_cash_balance(&db, "cash", 955.0, preview.revision, "Cash".into(), None)
            .unwrap_err()
            .contains("变更")
    );
    assert_eq!(
        get_cash_balance_reconciliation(&db, "cash")
            .unwrap()
            .current_balance,
        10000.0
    );
}

#[test]
fn failed_holding_update_rolls_back_new_opening_and_cache_invalidation() {
    let db = fixture();
    basic_ledger(&db);
    db.conn.lock().unwrap().execute_batch(
        "INSERT INTO daily_portfolio_values(date,total_value) VALUES('2025-01-01',999);
         CREATE TRIGGER reject_cash_update BEFORE UPDATE ON holdings BEGIN SELECT RAISE(ABORT,'test rejection'); END;",
    ).unwrap();
    let before = transaction_dump(&db);
    assert!(correct(&db, 1000.0).unwrap_err().contains("test rejection"));
    assert_eq!(transaction_dump(&db), before);
    assert_eq!(
        db.conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM daily_portfolio_values", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
}

#[test]
fn adopting_zero_and_negative_recommendations_never_creates_opening_records() {
    for withdrawal in [20.0, 25.0] {
        let db = fixture();
        transaction(
            &db,
            "deposit",
            "a",
            "$CASH-USD",
            "USD",
            "BUY",
            20.0,
            0.0,
            "2025-01-01",
        );
        transaction(
            &db,
            "withdrawal",
            "a",
            "$CASH-USD",
            "USD",
            "SELL",
            withdrawal,
            0.0,
            "2025-01-02",
        );
        let before = transaction_dump(&db);
        let target = 20.0 - withdrawal;
        assert_eq!(correct(&db, target).unwrap().shares, target);
        assert_eq!(
            get_cash_balance_reconciliation(&db, "cash")
                .unwrap()
                .opening_count,
            0
        );
        assert_eq!(transaction_dump(&db), before);
    }
}

#[test]
fn cash_correction_api_cannot_modify_stock_holdings() {
    let db = fixture();
    db.conn
        .lock()
        .unwrap()
        .execute("UPDATE holdings SET symbol='AAPL' WHERE id='cash'", [])
        .unwrap();
    let revision =
        crate::services::snapshot_cache_service::current_revision(&db.conn.lock().unwrap())
            .unwrap();
    assert!(
        correct_cash_balance(&db, "cash", 50.0, revision, "Stock".into(), None)
            .unwrap_err()
            .contains("现金持仓")
    );
    assert_eq!(
        db.conn
            .lock()
            .unwrap()
            .query_row("SELECT shares FROM holdings WHERE id='cash'", [], |row| row
                .get::<_, f64>(0))
            .unwrap(),
        10000.0
    );
}

#[test]
fn cash_correction_rejects_invalid_amounts_and_ambiguous_identity() {
    let db = fixture();
    for amount in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(correct(&db, amount).is_err());
    }
    db.conn
        .lock()
        .unwrap()
        .execute("UPDATE holdings SET currency='HKD' WHERE id='cash'", [])
        .unwrap();
    assert!(get_cash_balance_reconciliation(&db, "cash")
        .unwrap_err()
        .contains("币种"));
    db.conn.lock().unwrap().execute_batch(
        "UPDATE holdings SET currency='USD' WHERE id='cash';
         INSERT INTO holdings SELECT 'duplicate',account_id,symbol,name,market,category_id,shares,avg_cost,currency,created_at,updated_at FROM holdings WHERE id='cash';",
    ).unwrap();
    assert!(get_cash_balance_reconciliation(&db, "cash")
        .unwrap_err()
        .contains("重复"));
}

#[test]
fn custom_cash_without_history_uses_known_holding_creation_date() {
    let db = fixture();
    let holding = correct(&db, 25.0).unwrap();
    let preview = get_cash_balance_reconciliation(&db, "cash").unwrap();
    assert_eq!(preview.recommended_balance, Some(25.0));
    assert_eq!(preview.rows[0].transaction.traded_at, holding.created_at);
}
