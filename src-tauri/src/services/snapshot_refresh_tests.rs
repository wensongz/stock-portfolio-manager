use super::*;

fn refresh_fixture() -> (Database, ExchangeRateCache) {
    let db = Database::new(":memory:").unwrap();
    let cache = ExchangeRateCache::new();
    cache.set(crate::models::ExchangeRates {
        usd_cny: 7.2,
        usd_hkd: 7.8,
        cny_hkd: 7.8 / 7.2,
        updated_at: "2024-01-03".to_string(),
    });
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at)
         VALUES ('account', 'US account', 'US', '2024-01-01', '2024-01-01')",
            [],
        )
        .unwrap();
    (db, cache)
}

fn insert_holding_and_opening(db: &Database, symbol: &str, shares: f64) {
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO holdings
         (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
         VALUES ('holding', 'account', ?1, ?1, 'US', ?2, 10, 'USD', '2024-01-01', '2024-01-01')",
        rusqlite::params![symbol, shares],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO transactions
         (id, holding_id, account_id, symbol, name, market, transaction_type,
          shares, price, total_amount, commission, currency, traded_at, created_at)
         VALUES ('opening', 'holding', 'account', ?1, ?1, 'US', 'OPEN',
                 ?2, 10, ?2 * 10, 0, 'USD', '2024-01-01T12:00:00Z', '2024-01-01T12:00:00Z')",
        rusqlite::params![symbol, shares],
    )
    .unwrap();
}

fn seed_cached_day(db: &Database, date: &str, symbol: &str) {
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO daily_portfolio_values (date, total_cost, total_value, us_cost, us_value)
         VALUES (?1, 100, 100, 100, 100)",
        [date],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO daily_holding_snapshots
         (date, account_id, symbol, market, shares, avg_cost, close_price, market_value)
         VALUES (?1, 'account', ?2, 'US', 10, 10, 10, 100)",
        rusqlite::params![date, symbol],
    )
    .unwrap();
}

fn date(day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(2024, 1, day).unwrap()
}

#[tokio::test]
async fn forced_refresh_revalues_existing_holdings_without_transactions_in_range() {
    let (db, cache) = refresh_fixture();
    insert_holding_and_opening(&db, "AAPL", 10.0);
    seed_cached_day(&db, "2024-01-01", "AAPL");
    seed_cached_day(&db, "2024-01-02", "AAPL");
    seed_cached_day(&db, "2024-01-03", "AAPL");

    let created = backfill_snapshots_with_fetcher(
        &db,
        &cache,
        date(2),
        date(3),
        true,
        |symbol, market, _, _, _| async move {
            assert_eq!((symbol.as_str(), market.as_str()), ("AAPL", "US"));
            Ok(vec![(date(2), 20.0), (date(3), 21.0)])
        },
    )
    .await
    .unwrap();

    assert_eq!(created, 2);
    let values = get_daily_values(&db, date(1), date(3)).unwrap();
    assert_eq!(
        values
            .iter()
            .map(|value| value.total_value)
            .collect::<Vec<_>>(),
        vec![100.0, 200.0, 210.0]
    );
    let conn = db.conn.lock().unwrap();
    let refreshed: (f64, f64) = conn.query_row(
        "SELECT close_price, market_value FROM daily_holding_snapshots WHERE date = '2024-01-02'",
        [], |row| Ok((row.get(0)?, row.get(1)?)),
    ).unwrap();
    assert_eq!(refreshed, (20.0, 200.0));
}

#[tokio::test]
async fn forced_refresh_clears_cached_range_after_last_transaction_and_position_removed() {
    let (db, cache) = refresh_fixture();
    insert_holding_and_opening(&db, "AAPL", 10.0);
    seed_cached_day(&db, "2024-01-01", "AAPL");
    seed_cached_day(&db, "2024-01-02", "AAPL");
    {
        let conn = db.conn.lock().unwrap();
        conn.execute("DELETE FROM transactions WHERE id = 'opening'", [])
            .unwrap();
        conn.execute("UPDATE holdings SET shares = 0 WHERE id = 'holding'", [])
            .unwrap();
    }

    let created = backfill_snapshots_with_fetcher(
        &db,
        &cache,
        date(2),
        date(2),
        true,
        |_, _, _, _, _| async { panic!("an empty portfolio needs no history") },
    )
    .await
    .unwrap();

    assert_eq!(created, 0);
    let remaining = get_daily_values(&db, date(1), date(2)).unwrap();
    assert_eq!(
        remaining
            .iter()
            .map(|value| value.date.as_str())
            .collect::<Vec<_>>(),
        vec!["2024-01-01"]
    );
    let conn = db.conn.lock().unwrap();
    let remaining_details: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM daily_holding_snapshots WHERE date = '2024-01-02'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(remaining_details, 0);
}

#[tokio::test]
async fn forced_refresh_replaces_stale_value_with_zero_after_last_cash_transaction_removed() {
    let (db, cache) = refresh_fixture();
    insert_holding_and_opening(&db, "$CASH-USD", 100.0);
    seed_cached_day(&db, "2024-01-02", "$CASH-USD");
    {
        let conn = db.conn.lock().unwrap();
        conn.execute("DELETE FROM transactions WHERE id = 'opening'", [])
            .unwrap();
        conn.execute(
            "UPDATE holdings SET shares = 0, avg_cost = 1 WHERE id = 'holding'",
            [],
        )
        .unwrap();
    }

    let created = backfill_snapshots_with_fetcher(
        &db,
        &cache,
        date(2),
        date(2),
        true,
        |_, _, _, _, _| async { panic!("cash needs no history") },
    )
    .await
    .unwrap();

    assert_eq!(created, 1);
    let values = get_daily_values(&db, date(2), date(2)).unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].total_value, 0.0);
    assert_eq!(values[0].total_cost, 0.0);
    let conn = db.conn.lock().unwrap();
    let details: (f64, f64, f64) = conn.query_row(
        "SELECT shares, close_price, market_value FROM daily_holding_snapshots WHERE date = '2024-01-02'",
        [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
    ).unwrap();
    assert_eq!(details, (0.0, 1.0, 0.0));
}

#[tokio::test]
async fn forced_refresh_clears_stale_dates_before_the_only_position_was_opened() {
    let (db, cache) = refresh_fixture();
    insert_holding_and_opening(&db, "AAPL", 10.0);
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE transactions SET traded_at = '2024-01-03T12:00:00Z' WHERE id = 'opening'",
            [],
        )
        .unwrap();
    seed_cached_day(&db, "2024-01-01", "AAPL");
    seed_cached_day(&db, "2024-01-02", "AAPL");
    seed_cached_day(&db, "2024-01-03", "AAPL");

    let created = backfill_snapshots_with_fetcher(
        &db,
        &cache,
        date(2),
        date(2),
        true,
        |_, _, _, _, _| async { panic!("there was no position to price before its OPEN") },
    )
    .await
    .unwrap();

    assert_eq!(created, 0);
    assert!(get_daily_values(&db, date(2), date(2)).unwrap().is_empty());
    assert_eq!(get_daily_values(&db, date(1), date(3)).unwrap().len(), 2);
    let detail_count: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM daily_holding_snapshots WHERE date = '2024-01-02'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(detail_count, 0);
}

#[tokio::test]
async fn forced_refresh_preserves_unpriced_active_positions_instead_of_clearing_them() {
    let (db, cache) = refresh_fixture();
    insert_holding_and_opening(&db, "AAPL", 10.0);
    db.conn
        .lock()
        .unwrap()
        .execute("DELETE FROM transactions WHERE id = 'opening'", [])
        .unwrap();
    seed_cached_day(&db, "2024-01-02", "AAPL");

    let created = backfill_snapshots_with_fetcher(
        &db,
        &cache,
        date(2),
        date(2),
        true,
        |_, _, _, _, _| async { Ok(Vec::new()) },
    )
    .await
    .unwrap();

    assert_eq!(created, 0);
    let values = get_daily_values(&db, date(2), date(2)).unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].total_value, 100.0);
    let details: (f64, f64) = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT shares, market_value FROM daily_holding_snapshots WHERE date = '2024-01-02'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(details, (10.0, 100.0));
}

#[tokio::test]
async fn history_provider_failure_returns_an_error_and_preserves_existing_snapshots() {
    for force in [true, false] {
        let (db, cache) = refresh_fixture();
        insert_holding_and_opening(&db, "AAPL", 10.0);
        backfill_snapshots_with_fetcher(
            &db,
            &cache,
            date(2),
            date(2),
            false,
            |_, _, _, _, _| async { Ok(vec![(date(2), 20.0)]) },
        )
        .await
        .unwrap();

        let result = backfill_snapshots_with_fetcher(
            &db,
            &cache,
            date(2),
            date(3),
            force,
            |_, _, _, _, _| async { Err("history provider unavailable".to_string()) },
        )
        .await;

        assert!(
            result.is_err(),
            "provider failure must reach the caller (force={force})"
        );
        let values = get_daily_values(&db, date(2), date(3)).unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].date, "2024-01-02");
        assert_eq!(values[0].total_value, 200.0);
        let details: (f64, f64) = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT close_price, market_value FROM daily_holding_snapshots WHERE date = '2024-01-02'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(details, (20.0, 200.0));
    }
}

#[tokio::test]
async fn ordinary_refresh_reuses_existing_daily_cache() {
    let (db, cache) = refresh_fixture();
    insert_holding_and_opening(&db, "AAPL", 10.0);
    seed_cached_day(&db, "2024-01-02", "AAPL");

    let created = backfill_snapshots_with_fetcher(
        &db,
        &cache,
        date(2),
        date(2),
        false,
        |_, _, _, _, _| async { panic!("valid cached snapshots must not fetch history") },
    )
    .await
    .unwrap();

    assert_eq!(created, 0);
    let values = get_daily_values(&db, date(2), date(2)).unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].total_value, 100.0);
}

#[tokio::test]
async fn ordinary_refresh_rebuilds_values_after_a_real_historical_buy_edit() {
    use crate::services::portfolio_mutation::{
        create_transaction_in, update_transaction_in, CreateTransactionInput,
    };

    let (db, cache) = refresh_fixture();
    let mut buy = CreateTransactionInput {
        account_id: "account".into(),
        symbol: "AAPL".into(),
        name: "Apple".into(),
        market: "US".into(),
        transaction_type: "BUY".into(),
        shares: 10.0,
        price: 10.0,
        total_amount: 100.0,
        commission: 0.0,
        currency: "USD".into(),
        traded_at: "2024-01-01T12:00:00Z".into(),
        notes: None,
    };
    let buy_id = {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        create_transaction_in(
            &tx,
            &CreateTransactionInput {
                symbol: "$CASH-USD".into(),
                name: "USD cash".into(),
                shares: 0.0,
                price: 1.0,
                total_amount: 1_000.0,
                traded_at: "2024-01-01T09:00:00Z".into(),
                ..buy.clone()
            },
        )
        .unwrap();
        let created = create_transaction_in(&tx, &buy).unwrap();
        tx.commit().unwrap();
        created.id
    };

    // Both transactions predate the requested performance range.
    backfill_snapshots_with_fetcher(
        &db,
        &cache,
        date(2),
        date(3),
        false,
        |_, _, _, _, _| async { Ok(vec![(date(2), 20.0), (date(3), 21.0)]) },
    )
    .await
    .unwrap();
    let before = get_daily_values(&db, date(2), date(3)).unwrap();
    assert_eq!(
        before
            .iter()
            .map(|value| value.total_value)
            .collect::<Vec<_>>(),
        vec![1_100.0, 1_110.0]
    );

    buy.shares = 20.0;
    buy.total_amount = 200.0;
    {
        let mut conn = db.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        update_transaction_in(&tx, &buy_id, &buy).unwrap();
        tx.commit().unwrap();
    }

    let report_start = date(3);
    let created = backfill_snapshots_with_fetcher(
        &db,
        &cache,
        crate::services::snapshot_cache_service::backfill_start_with_baseline(report_start),
        date(3),
        false,
        |_, _, _, _, _| async { Ok(vec![(date(2), 20.0), (date(3), 21.0)]) },
    )
    .await
    .unwrap();

    assert_eq!(created, 2);
    let after = get_daily_values(&db, date(2), date(3)).unwrap();
    assert_eq!(
        after
            .iter()
            .map(|value| value.total_value)
            .collect::<Vec<_>>(),
        vec![1_200.0, 1_220.0]
    );
    assert_eq!(after[0].total_cost, 1_000.0);
    assert_eq!(after[1].total_cost, 1_000.0);
    assert_eq!(after[0].cumulative_pnl, 200.0);
    assert_eq!(after[1].cumulative_pnl, 220.0);
    assert_eq!(after[1].daily_pnl, 20.0);

    let summary = crate::services::performance_service::get_performance_summary(
        &db,
        report_start,
        date(3),
        &crate::services::performance_service::PerformanceFilter::default(),
    )
    .unwrap();
    assert_eq!(summary.start_value, 1_200.0);
    assert_eq!(summary.end_value, 1_220.0);
    assert_eq!(summary.total_pnl, 20.0);
    assert!((summary.total_return - 1.666_666_666_666_666_7).abs() < 1e-9);

    let conn = db.conn.lock().unwrap();
    let holdings: Vec<(String, f64, f64, f64)> = conn
        .prepare(
            "SELECT symbol, shares, avg_cost, market_value FROM daily_holding_snapshots
             WHERE date = '2024-01-03' ORDER BY symbol",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(
        holdings,
        vec![
            ("$CASH-USD".into(), 800.0, 1.0, 800.0),
            ("AAPL".into(), 20.0, 10.0, 420.0),
        ]
    );
}

#[tokio::test]
async fn backfill_rejects_old_inputs_when_transaction_changes_during_history_fetch() {
    let (db, cache) = refresh_fixture();
    insert_holding_and_opening(&db, "AAPL", 10.0);
    seed_cached_day(&db, "2024-01-02", "AAPL");
    let fetch_started = tokio::sync::Notify::new();
    let mutation_finished = tokio::sync::Notify::new();

    let backfill = backfill_snapshots_with_fetcher(
        &db,
        &cache,
        date(2),
        date(2),
        true,
        |_, _, _, _, _| async {
            fetch_started.notify_one();
            mutation_finished.notified().await;
            Ok(vec![(date(2), 20.0)])
        },
    );
    let mutation = async {
        fetch_started.notified().await;
        {
            let mut conn = db.conn.lock().unwrap();
            let tx = conn.transaction().unwrap();
            tx.execute(
                "UPDATE transactions SET shares = 5, total_amount = 50 WHERE id = 'opening'",
                [],
            )
            .unwrap();
            tx.execute("UPDATE holdings SET shares = 5 WHERE id = 'holding'", [])
                .unwrap();
            crate::services::snapshot_cache_service::invalidate_from(&tx, "2024-01-01T12:00:00Z")
                .unwrap();
            tx.commit().unwrap();
        }
        mutation_finished.notify_one();
    };

    let (result, ()) = tokio::join!(backfill, mutation);

    assert!(
        result.is_err(),
        "backfill must reject inputs read before a committed mutation"
    );
    assert!(get_daily_values(&db, date(2), date(2)).unwrap().is_empty());
    let detail_count: i64 = db
        .conn
        .lock()
        .unwrap()
        .query_row(
            "SELECT COUNT(*) FROM daily_holding_snapshots WHERE date = '2024-01-02'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(detail_count, 0);

    backfill_snapshots_with_fetcher(
        &db,
        &cache,
        date(2),
        date(2),
        false,
        |_, _, _, _, _| async { Ok(vec![(date(2), 20.0)]) },
    )
    .await
    .unwrap();
    let values = get_daily_values(&db, date(2), date(2)).unwrap();
    assert_eq!(values.len(), 1);
    assert_eq!(values[0].total_value, 100.0);
    let shares: f64 = db.conn.lock().unwrap().query_row(
        "SELECT shares FROM daily_holding_snapshots WHERE date = '2024-01-02' AND symbol = 'AAPL'",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(shares, 5.0);
}
