use super::{migrations::run_migrations, Database};

#[test]
fn upgrade_from_v6_archives_rates_before_daily_cache_cleanup() {
    let db = Database::new(":memory:").unwrap();
    let mut conn = db.conn.lock().unwrap();
    conn.execute_batch(r#"
        INSERT INTO daily_portfolio_values (date, exchange_rates) VALUES
          ('2025-03-28', '{"usd_cny":7,"usd_hkd":7.8,"cny_hkd":1.114285714,"updated_at":"2025-03-28"}');
        INSERT INTO quarterly_snapshots (id,quarter,snapshot_date,exchange_rates,overall_notes,created_at)
          VALUES ('q','2025-Q1','2025-03-31','{"usd_cny":7.1,"usd_hkd":7.8,"cny_hkd":1.098591549,"updated_at":"2025-03-31"}','Keep notes','2025-04-02');
        INSERT INTO quarterly_holding_snapshots (id,quarterly_snapshot_id,account_id,symbol,name,market,notes,decision_quality)
          VALUES ('h','q','a','$CASH-USD','USD cash','CN','Keep cash note','good');
        PRAGMA user_version=6;
    "#).unwrap();

    run_migrations(&mut conn).unwrap();

    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM historical_exchange_rates", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(count, 2);
    let daily: i64 = conn
        .query_row("SELECT COUNT(*) FROM daily_portfolio_values", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(daily, 0);
    let row: (String, String, String) = conn.query_row("SELECT currency, notes, decision_quality FROM quarterly_holding_snapshots WHERE id='h'", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?))).unwrap();
    assert_eq!(row, ("USD".into(), "Keep cash note".into(), "good".into()));
    assert_eq!(
        conn.query_row(
            "SELECT overall_notes FROM quarterly_snapshots WHERE id='q'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "Keep notes"
    );
}

#[test]
fn upgrade_keeps_future_observation_dates_and_ignores_invalid_rate_records() {
    let db = Database::new(":memory:").unwrap();
    let mut conn = db.conn.lock().unwrap();
    conn.execute_batch(r#"
        INSERT INTO daily_portfolio_values (date, exchange_rates) VALUES
          ('2025-03-28', '{"usd_cny":7,"usd_hkd":7.8,"cny_hkd":1.114285714,"updated_at":"2025-06-01"}'),
          ('2025-03-29', '{}'),
          ('2025-03-30', '{"usd_cny":0,"usd_hkd":7.8,"cny_hkd":1.1,"updated_at":"2025-03-30"}');
        PRAGMA user_version=7;
    "#).unwrap();
    run_migrations(&mut conn).unwrap();
    let archived: Vec<String> = conn
        .prepare("SELECT rate_date FROM historical_exchange_rates")
        .unwrap()
        .query_map([], |r| r.get(0))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap();
    assert_eq!(archived, ["2025-06-01"]);
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM daily_portfolio_values", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
    run_migrations(&mut conn).unwrap();
    assert_eq!(
        conn.query_row("SELECT COUNT(*) FROM historical_exchange_rates", [], |r| {
            r.get::<_, i64>(0)
        })
        .unwrap(),
        1
    );
}
