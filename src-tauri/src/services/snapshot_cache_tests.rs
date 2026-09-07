use crate::db::Database;
use crate::services::portfolio_mutation::{
    create_transaction_in, delete_transaction_in, update_transaction_in, CreateTransactionInput,
};

fn database() -> Database {
    let db = Database::new(":memory:").unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO accounts VALUES ('a','A','US',NULL,'2026-01-01','2026-01-01')",
            [],
        )
        .unwrap();
    db
}

fn buy(date: &str) -> CreateTransactionInput {
    CreateTransactionInput {
        account_id: "a".into(),
        symbol: "AAPL".into(),
        name: "Apple".into(),
        market: "US".into(),
        transaction_type: "BUY".into(),
        shares: 10.0,
        price: 10.0,
        total_amount: 100.0,
        commission: 0.0,
        currency: "USD".into(),
        traded_at: date.into(),
        notes: None,
    }
}

fn insert_buy(db: &Database, date: &str) -> String {
    let mut conn = db.conn.lock().unwrap();
    let tx = conn.transaction().unwrap();
    let created = create_transaction_in(&tx, &buy(date)).unwrap();
    tx.commit().unwrap();
    created.id
}

fn seed_snapshots(db: &Database) {
    let conn = db.conn.lock().unwrap();
    for day in 10..=14 {
        let date = format!("2026-01-{day}");
        conn.execute(
            "INSERT INTO daily_portfolio_values(date,total_value) VALUES(?1,1000)",
            [&date],
        )
        .unwrap();
        conn.execute("INSERT INTO daily_holding_snapshots(date,account_id,symbol,market,market_value) VALUES(?1,'a','AAPL','US',1000)", [&date]).unwrap();
    }
}

fn assert_dates(db: &Database, expected_days: &[i32]) {
    let conn = db.conn.lock().unwrap();
    for table in ["daily_portfolio_values", "daily_holding_snapshots"] {
        let actual: Vec<String> = conn
            .prepare(&format!("SELECT date FROM {table} ORDER BY date"))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        let expected: Vec<String> = expected_days
            .iter()
            .map(|d| format!("2026-01-{d}"))
            .collect();
        assert_eq!(actual, expected, "{table}");
    }
}

#[test]
fn new_trade_expires_daily_totals_and_positions_from_its_date() {
    let db = database();
    seed_snapshots(&db);
    insert_buy(&db, "2026-01-12T12:00:00Z");
    assert_dates(&db, &[10, 11]);
}

#[test]
fn date_edits_expire_from_the_earlier_old_or_new_trade_date() {
    for (old, new) in [("2026-01-12", "2026-01-14"), ("2026-01-14", "2026-01-12")] {
        let db = database();
        let id = insert_buy(&db, old);
        seed_snapshots(&db);
        {
            let mut conn = db.conn.lock().unwrap();
            let tx = conn.transaction().unwrap();
            update_transaction_in(&tx, &id, &buy(new)).unwrap();
            tx.commit().unwrap();
        }
        assert_dates(&db, &[10, 11]);
    }
}

#[test]
fn deleting_the_last_trade_expires_its_snapshots() {
    let db = database();
    let id = insert_buy(&db, "2026-01-12");
    seed_snapshots(&db);
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        delete_transaction_in(&tx, &id).unwrap();
        tx.commit().unwrap();
    }
    assert_dates(&db, &[10, 11]);
}

#[test]
fn invalidation_uses_the_same_utc_date_as_snapshot_queries() {
    let db = database();
    seed_snapshots(&db);
    insert_buy(&db, "2026-01-12T00:30:00+08:00");
    assert_dates(&db, &[10]);
}

#[test]
fn failed_cache_invalidation_rolls_back_the_ledger_write() {
    let db = database();
    seed_snapshots(&db);
    {
        let mut conn = db.conn.lock().unwrap();
        conn.execute_batch("CREATE TRIGGER reject_invalidation BEFORE DELETE ON daily_holding_snapshots BEGIN SELECT RAISE(ABORT,'cache unavailable'); END;").unwrap();
        let tx = conn.transaction().unwrap();
        assert!(create_transaction_in(&tx, &buy("2026-01-12")).is_err());
        tx.rollback().unwrap();
        let count: i64 = conn
            .query_row("SELECT COUNT(*) FROM transactions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
        let holdings: i64 = conn
            .query_row("SELECT COUNT(*) FROM holdings", [], |r| r.get(0))
            .unwrap();
        assert_eq!(holdings, 0);
    }
    assert_dates(&db, &[10, 11, 12, 13, 14]);
}

#[test]
fn notes_only_edits_preserve_daily_cache() {
    let db = database();
    let id = insert_buy(&db, "2026-01-12");
    seed_snapshots(&db);
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        let mut input = buy("2026-01-12");
        input.notes = Some("Corrected note".into());
        update_transaction_in(&tx, &id, &input).unwrap();
        tx.commit().unwrap();
    }
    assert_dates(&db, &[10, 11, 12, 13, 14]);
}

#[test]
fn fee_edits_expire_values_even_when_trade_date_and_shares_are_unchanged() {
    let db = database();
    let id = insert_buy(&db, "2026-01-12");
    seed_snapshots(&db);
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        let mut input = buy("2026-01-12");
        input.commission = 2.5;
        update_transaction_in(&tx, &id, &input).unwrap();
        tx.commit().unwrap();
        let cash: f64 = conn
            .query_row(
                "SELECT shares FROM holdings WHERE symbol='$CASH-USD'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cash, -102.5);
    }
    assert_dates(&db, &[10, 11]);
}

#[test]
fn import_and_undo_expire_from_the_utc_trade_date() {
    use crate::services::import_batch::{
        apply_import_batch, preview_import_batch, undo_import_batch,
    };
    use serde_json::json;

    let db = database();
    seed_snapshots(&db);
    let request = serde_json::from_value(json!({
        "request_id":"cache-test", "account_id":"a", "source":"broker",
        "file_name":"trades.csv", "source_content":"cache-test", "parser_version":"1",
        "kind":"transactions", "rows":[{"key":"1", "raw":"original", "data": {
            "symbol":"AAPL", "name":"Apple", "market":"US", "currency":"USD",
            "transaction_type":"BUY", "shares":10, "price":10,
            "total_amount":100, "commission":0, "traded_at":"2026-01-12T00:30:00+08:00"
        }}]
    }))
    .unwrap();
    let batch = preview_import_batch(&db, &request).unwrap();
    let applied = apply_import_batch(&db, &batch.id, &["1".into()], &[]).unwrap();
    assert_eq!(applied.rows[0].status, "imported");
    assert_dates(&db, &[10]);

    // Simulate the next successful valuation before undoing the import.
    db.conn
        .lock()
        .unwrap()
        .execute_batch("DELETE FROM daily_portfolio_values; DELETE FROM daily_holding_snapshots;")
        .unwrap();
    seed_snapshots(&db);
    undo_import_batch(&db, &batch.id).unwrap();
    assert_dates(&db, &[10]);
}

#[test]
fn performance_backfill_includes_the_previous_weekday_baseline() {
    use crate::services::snapshot_cache_service::backfill_start_with_baseline;
    use chrono::NaiveDate;
    for (start, baseline) in [
        ("2026-02-02", "2026-01-30"),
        ("2026-02-01", "2026-01-30"),
        ("2026-01-31", "2026-01-30"),
        ("2026-01-30", "2026-01-29"),
        ("2026-01-01", "2025-12-31"),
    ] {
        let parsed = NaiveDate::parse_from_str(start, "%Y-%m-%d").unwrap();
        assert_eq!(backfill_start_with_baseline(parsed).to_string(), baseline);
    }
}
