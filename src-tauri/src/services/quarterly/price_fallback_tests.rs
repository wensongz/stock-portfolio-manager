use super::*;

fn account(db: &Database, id: &str) {
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO accounts(id,name,market,created_at,updated_at)
         VALUES(?1,?1,'CN','2025-01-01','2025-01-01')",
            [id],
        )
        .unwrap();
}

#[allow(clippy::too_many_arguments)]
fn transaction(
    db: &Database,
    id: &str,
    account: &str,
    symbol: &str,
    market: &str,
    currency: &str,
    kind: &str,
    shares: f64,
    price: f64,
    fee: f64,
    date: &str,
) {
    db.conn.lock().unwrap().execute(
        "INSERT INTO transactions(id,account_id,symbol,name,market,currency,transaction_type,shares,price,total_amount,commission,traded_at,created_at)
         VALUES(?1,?2,?3,?3,?4,?5,?6,?7,?8,?9,?10,?11,?11)",
        rusqlite::params![id,account,symbol,market,currency,kind,shares,price,shares*price,fee,date],
    ).unwrap();
}

fn position(account: &str, market: &str, currency: &str) -> WorkingHolding {
    WorkingHolding {
        account_id: account.into(),
        account_name: account.into(),
        symbol: "sz001248".into(),
        name: "IPO".into(),
        market: market.into(),
        currency: currency.into(),
        category_name: "股票".into(),
        category_color: "#fff".into(),
        shares: 1.0,
        avg_cost: 999.0,
        notes: None,
        decision_quality: None,
    }
}

fn ipo_fixture() -> Database {
    let db = Database::new(":memory:").unwrap();
    account(&db, "a");
    transaction(
        &db,
        "funding",
        "a",
        "$CASH-CNY",
        "CN",
        "CNY",
        "BUY",
        20000.0,
        1.0,
        0.0,
        "2025-01-01",
    );
    transaction(
        &db,
        "ipo-buy",
        "a",
        "sz001248",
        "CN",
        "CNY",
        "BUY",
        1000.0,
        10.11,
        0.0,
        "2025-06-24",
    );
    transaction(
        &db,
        "q3-sell",
        "a",
        "sz001248",
        "CN",
        "CNY",
        "SELL",
        1000.0,
        23.11,
        16.79,
        "2025-07-02",
    );
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO holdings(id,account_id,symbol,name,market,currency,shares,avg_cost,created_at,updated_at)
         VALUES('cash','a','$CASH-CNY','Cash','CN','CNY',?1,1,'2025-01-01','2025-07-02')",
        [20000.0 - 10110.0 + 23110.0 - 16.79],
    ).unwrap();
    conn.execute_batch(
        "INSERT INTO quarterly_snapshots(id,quarter,snapshot_date,total_value,exchange_rates,overall_notes,created_at)
         VALUES('q2','2025-Q2','2025-06-30',999,'{\"usd_cny\":5,\"usd_hkd\":8,\"cny_hkd\":1.6,\"updated_at\":\"2025-06-30\"}','keep review','2025-07-01');
         INSERT INTO quarterly_holding_snapshots(id,quarterly_snapshot_id,account_id,account_name,symbol,name,market,currency,shares,avg_cost,close_price,notes,decision_quality)
         VALUES('old-stock','q2','a','a','sz001248','IPO','CN','CNY',1000,10.11,1,'keep IPO note','good');",
    ).unwrap();
    drop(conn);
    db
}

fn snapshot_dump(db: &Database) -> Vec<Vec<String>> {
    let conn = db.conn.lock().unwrap();
    let mut dump = Vec::new();
    for table in ["quarterly_snapshots", "quarterly_holding_snapshots"] {
        let mut statement = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY id"))
            .unwrap();
        let columns = statement.column_count();
        let rows = statement
            .query_map([], |row| {
                (0..columns)
                    .map(|index| row.get_ref(index).map(|value| format!("{value:?}")))
                    .collect::<rusqlite::Result<Vec<_>>>()
            })
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        dump.extend(rows);
    }
    dump
}

#[tokio::test]
async fn quarterly_price_fallback_rebuilds_ipo_and_cash_without_using_q3_sale_or_quote() {
    for provider_has_future_price in [false, true] {
        let db = ipo_fixture();
        let result = rebuild_quarterly_snapshot_with_history_fetcher(
            &db,
            &ExchangeRateCache::new(),
            &QuoteCache::new(),
            &QuoteServiceState::new(),
            "2025-Q2",
            Some("q2"),
            |symbol, market, _, _, _| async move {
                assert!(symbol.eq_ignore_ascii_case("sz001248"));
                assert_eq!(market, "CN");
                Ok(if provider_has_future_price {
                    vec![(NaiveDate::from_ymd_opt(2025, 7, 2).unwrap(), 23.11)]
                } else {
                    vec![]
                })
            },
        )
        .await
        .unwrap();
        assert_eq!(result.id, "q2");
        assert_eq!(result.snapshot_date, "2025-06-30");
        assert!((result.total_value - 4000.0).abs() < 1e-8);
        assert!((result.total_cost - 4000.0).abs() < 1e-8);
        assert!(result.total_pnl.abs() < 1e-8);
        let detail =
            crate::services::quarterly_service::get_quarterly_snapshot_detail(&db, "q2").unwrap();
        assert_eq!(detail.holdings.len(), 2);
        let stock = detail
            .holdings
            .iter()
            .find(|holding| holding.symbol == "sz001248")
            .unwrap();
        assert_eq!(
            (
                stock.shares,
                stock.close_price,
                stock.avg_cost,
                stock.market_value
            ),
            (1000.0, 10.11, 10.11, 10110.0)
        );
        assert_eq!(stock.notes.as_deref(), Some("keep IPO note"));
        let cash = detail
            .holdings
            .iter()
            .find(|holding| holding.symbol == "$CASH-CNY")
            .unwrap();
        assert!((cash.shares - 9890.0).abs() < 1e-8);
        assert_eq!((cash.avg_cost, cash.close_price), (1.0, 1.0));
        assert_eq!(
            detail.snapshot.overall_notes.as_deref(),
            Some("keep review")
        );
    }
}

#[tokio::test]
async fn quarterly_price_fallback_uses_latest_finite_raw_acquisition_price_for_each_identity() {
    let db = Database::new(":memory:").unwrap();
    for id in ["a", "b", "c"] {
        account(&db, id);
    }
    for (id, account, market, currency, kind, price, date) in [
        ("old-open", "a", "CN", "CNY", "OPEN", 5.0, "2025-05-01"),
        ("old-buy", "a", "CN", "CNY", "BUY", 7.0, "2025-06-01"),
        (
            "latest-transfer",
            "a",
            "CN",
            "CNY",
            "STOCK_IN",
            11.0,
            "2025-07-01T00:30:00+08:00",
        ),
        (
            "future-buy",
            "a",
            "CN",
            "CNY",
            "BUY",
            99.0,
            "2025-07-01T00:00:00Z",
        ),
        (
            "sale",
            "a",
            "CN",
            "CNY",
            "SELL",
            90.0,
            "2025-06-30T20:00:00Z",
        ),
        (
            "payment",
            "a",
            "CN",
            "CNY",
            "PAY",
            91.0,
            "2025-06-30T20:00:00Z",
        ),
        (
            "outgoing-transfer",
            "a",
            "CN",
            "CNY",
            "STOCK_OUT",
            92.0,
            "2025-06-30T20:00:00Z",
        ),
        (
            "other-account-open",
            "b",
            "CN",
            "CNY",
            "OPEN",
            22.0,
            "2025-06-30",
        ),
        (
            "other-market-buy",
            "a",
            "US",
            "USD",
            "BUY",
            33.0,
            "2025-06-30",
        ),
        (
            "hk-transfer",
            "a",
            "HK",
            "HKD",
            "STOCK_IN",
            44.0,
            "2025-06-30",
        ),
        (
            "other-currency",
            "a",
            "CN",
            "USD",
            "BUY",
            88.0,
            "2025-06-30T21:00:00Z",
        ),
        (
            "unselected-account",
            "c",
            "CN",
            "CNY",
            "BUY",
            77.0,
            "2025-06-30T21:00:00Z",
        ),
        (
            "invalid-price",
            "a",
            "CN",
            "CNY",
            "BUY",
            f64::INFINITY,
            "2025-06-30T22:00:00Z",
        ),
    ] {
        transaction(
            &db, id, account, "SZ001248", market, currency, kind, 1.0, price, 2.0, date,
        );
    }
    let positions = vec![
        position("a", "CN", "CNY"),
        position("b", "CN", "CNY"),
        position("a", "US", "USD"),
        position("a", "HK", "HKD"),
    ];
    let prices = resolve_historical_prices(
        &db,
        &positions,
        NaiveDate::from_ymd_opt(2025, 6, 30).unwrap(),
        &|_, _, _, _, _| async { Ok(vec![]) },
    )
    .await
    .unwrap();
    assert_eq!(prices[&PositionKey::from_holding(&positions[0])], 11.0);
    assert_eq!(prices[&PositionKey::from_holding(&positions[1])], 22.0);
    assert_eq!(prices[&PositionKey::from_holding(&positions[2])], 33.0);
    assert_eq!(prices[&PositionKey::from_holding(&positions[3])], 44.0);
}

#[tokio::test]
async fn quarterly_price_fallback_keeps_local_and_provider_historical_close_precedence() {
    for has_daily_close in [false, true] {
        let db = Database::new(":memory:").unwrap();
        account(&db, "a");
        transaction(
            &db,
            "buy",
            "a",
            "sz001248",
            "CN",
            "CNY",
            "BUY",
            1.0,
            10.11,
            0.0,
            "2025-06-24",
        );
        if has_daily_close {
            db.conn.lock().unwrap().execute_batch(
                "INSERT INTO daily_holding_snapshots(date,account_id,symbol,market,close_price)
                 VALUES('2025-06-27','a','sz001248','CN',12),('2025-07-02','a','sz001248','CN',99);",
            ).unwrap();
        }
        let holding = position("a", "CN", "CNY");
        let prices = resolve_historical_prices(
            &db,
            std::slice::from_ref(&holding),
            NaiveDate::from_ymd_opt(2025, 6, 30).unwrap(),
            &|_, _, _, _, _| async move {
                assert!(
                    !has_daily_close,
                    "a saved historical close should not require a provider request"
                );
                Ok(vec![
                    (NaiveDate::from_ymd_opt(2025, 6, 30).unwrap(), 15.0),
                    (NaiveDate::from_ymd_opt(2025, 7, 2).unwrap(), 99.0),
                ])
            },
        )
        .await
        .unwrap();
        assert_eq!(
            prices[&PositionKey::from_holding(&holding)],
            if has_daily_close { 12.0 } else { 15.0 }
        );
    }
}

#[tokio::test]
async fn quarterly_price_fallback_does_not_hide_network_failure_or_replace_saved_snapshot() {
    let db = ipo_fixture();
    let before = snapshot_dump(&db);
    let error = rebuild_quarterly_snapshot_with_history_fetcher(
        &db,
        &ExchangeRateCache::new(),
        &QuoteCache::new(),
        &QuoteServiceState::new(),
        "2025-Q2",
        Some("q2"),
        |_, _, _, _, _| async { Err("provider unavailable".into()) },
    )
    .await
    .unwrap_err();
    assert!(error.contains("missing closing price"));
    assert_eq!(snapshot_dump(&db), before);
}

#[tokio::test]
async fn quarterly_price_fallback_rejects_absent_positive_finite_acquisition_prices() {
    let db = Database::new(":memory:").unwrap();
    account(&db, "a");
    for (id, price) in [
        ("zero", 0.0),
        ("negative", -1.0),
        ("infinite", f64::INFINITY),
    ] {
        transaction(
            &db,
            id,
            "a",
            "sz001248",
            "CN",
            "CNY",
            "BUY",
            1.0,
            price,
            0.0,
            "2025-06-24",
        );
    }
    let error = resolve_historical_prices(
        &db,
        &[position("a", "CN", "CNY")],
        NaiveDate::from_ymd_opt(2025, 6, 30).unwrap(),
        &|_, _, _, _, _| async { Ok(vec![]) },
    )
    .await
    .unwrap_err();
    assert!(error.contains("missing closing price"));
}
