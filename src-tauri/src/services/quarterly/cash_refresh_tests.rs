use super::*;

fn account(db: &Database, id: &str) {
    db.conn
        .lock()
        .unwrap()
        .execute(
            "INSERT INTO accounts(id,name,market,created_at,updated_at)
         VALUES(?1,?1,'US','2025-01-01','2025-01-01')",
            [id],
        )
        .unwrap();
}

fn current_cash(db: &Database, account: &str, currency: &str, balance: f64) {
    db.conn.lock().unwrap().execute(
        "INSERT INTO holdings(id,account_id,symbol,name,market,currency,shares,avg_cost,created_at,updated_at)
         VALUES(?1,?2,?3,'Cash','US',?4,?5,1,'2025-01-01','2025-08-01')",
        rusqlite::params![format!("{account}-{currency}"),account,format!("$CASH-{currency}"),currency,balance],
    ).unwrap();
}

#[allow(clippy::too_many_arguments)]
fn flow(
    db: &Database,
    id: &str,
    account: &str,
    currency: &str,
    symbol: &str,
    kind: &str,
    amount: f64,
    fee: f64,
    date: &str,
) {
    db.conn.lock().unwrap().execute(
        "INSERT INTO transactions(id,account_id,symbol,name,market,currency,transaction_type,shares,price,total_amount,commission,traded_at,created_at)
         VALUES(?1,?2,?3,?3,'US',?4,?5,?6,1,?6,?7,?8,?8)",
        rusqlite::params![id,account,symbol,currency,kind,amount,fee,date],
    ).unwrap();
}

fn q2_cash(db: &Database) -> Result<BTreeMap<(String, String), f64>, String> {
    Ok(
        load_historical_holdings(db, NaiveDate::from_ymd_opt(2025, 6, 30).unwrap())?
            .into_iter()
            .filter(|holding| holding.symbol.starts_with("$CASH-"))
            .map(|holding| {
                assert_eq!(holding.symbol, format!("$CASH-{}", holding.currency));
                assert_eq!(holding.avg_cost, 1.0);
                ((holding.account_id, holding.currency), holding.shares)
            })
            .collect(),
    )
}

#[tokio::test]
async fn quarterly_cash_refresh_restores_missing_rows_and_preserves_saved_review_basis() {
    let db = Database::new(":memory:").unwrap();
    for (id, currency, current, deposit, withdrawal, future) in [
        ("a", "USD", 150.0, 100.0, 20.0, 70.0_f64),
        ("b", "HKD", -10.0, 100.0, 100.0, -10.0),
        ("c", "CNY", 20.0, 50.0, 100.0, 70.0),
    ] {
        account(&db, id);
        current_cash(&db, id, currency, current);
        let symbol = format!("$CASH-{currency}");
        flow(
            &db,
            &format!("{id}-deposit"),
            id,
            currency,
            &symbol,
            "BUY",
            deposit,
            0.0,
            "2025-03-01",
        );
        flow(
            &db,
            &format!("{id}-withdrawal"),
            id,
            currency,
            &symbol,
            "SELL",
            withdrawal,
            0.0,
            "2025-06-30T23:59:59Z",
        );
        flow(
            &db,
            &format!("{id}-future"),
            id,
            currency,
            &symbol,
            if future > 0.0 { "BUY" } else { "SELL" },
            future.abs(),
            0.0,
            "2025-07-01T00:00:00Z",
        );
    }
    let saved_rates = ExchangeRates {
        usd_cny: 5.0,
        usd_hkd: 8.0,
        cny_hkd: 1.6,
        updated_at: "2025-06-30".into(),
    };
    let saved_json = serde_json::to_string(&saved_rates).unwrap();
    {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO quarterly_snapshots(id,quarter,snapshot_date,total_value,exchange_rates,overall_notes,created_at)
             VALUES('old-q2','2025-Q2','2025-06-30',999,?1,'preserve overall notes','2025-07-01')", [&saved_json],
        ).unwrap();
        conn.execute_batch(
            "INSERT INTO quarterly_holding_snapshots(id,quarterly_snapshot_id,account_id,account_name,symbol,name,market,currency,shares,avg_cost,close_price,notes,decision_quality)
             VALUES('only-old-cash','old-q2','a','a','$CASH-USD','Cash','US','USD',999,1,1,'preserve account notes','good');",
        ).unwrap();
        let competing_rates = ExchangeRates {
            usd_cny: 9.0,
            usd_hkd: 10.0,
            cny_hkd: 10.0 / 9.0,
            updated_at: "2025-06-30".into(),
        };
        crate::services::historical_exchange_rate_service::record_snapshot_rates_in(
            &conn,
            "2025-06-30",
            &competing_rates,
        )
        .unwrap();
    }
    let cache = ExchangeRateCache::new();
    let quotes = QuoteCache::new();
    let quote_state = QuoteServiceState::new();
    for _ in 0..2 {
        // This is the public refresh service used by the command. Cash has a
        // fixed quote of one and the snapshot already has historical FX.
        let detail = crate::services::quarterly_service::refresh_quarterly_snapshot(
            &db,
            &cache,
            &quotes,
            &quote_state,
            "old-q2",
        )
        .await
        .unwrap();
        assert_eq!(detail.snapshot.id, "old-q2");
        assert_eq!(detail.snapshot.created_at, "2025-07-01");
        assert_eq!(detail.snapshot.snapshot_date, "2025-06-30");
        assert_eq!(
            detail.snapshot.overall_notes.as_deref(),
            Some("preserve overall notes")
        );
        assert_eq!(detail.snapshot.exchange_rates, saved_json);
        assert_eq!(detail.snapshot.total_value, 70.0);
        assert_eq!(detail.snapshot.total_cost, 70.0);
        assert_eq!(detail.snapshot.total_pnl, 0.0);
        assert_eq!(detail.holdings.len(), 3);
        let balances = detail
            .holdings
            .iter()
            .map(|holding| {
                assert_eq!(holding.symbol, format!("$CASH-{}", holding.currency));
                assert_eq!((holding.avg_cost, holding.close_price), (1.0, 1.0));
                assert_eq!(holding.market_value, holding.shares);
                (
                    (holding.account_id.clone(), holding.currency.clone()),
                    holding.shares,
                )
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(
            balances,
            BTreeMap::from([
                (("a".into(), "USD".into()), 80.0),
                (("b".into(), "HKD".into()), 0.0),
                (("c".into(), "CNY".into()), -50.0),
            ])
        );
        let conn = db.conn.lock().unwrap();
        let saved_note: (String, String) = conn
            .query_row(
                "SELECT notes,decision_quality FROM quarterly_holding_snapshots
             WHERE quarterly_snapshot_id='old-q2' AND account_id='a'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(saved_note, ("preserve account notes".into(), "good".into()));
    }
}

#[test]
fn quarterly_cash_missing_current_replays_explicit_deposits_through_cutoff() {
    let db = Database::new(":memory:").unwrap();
    account(&db, "a");
    account(&db, "b");
    flow(
        &db,
        "deposit",
        "a",
        "USD",
        "$CASH-USD",
        "BUY",
        100.0,
        1.0,
        "2025-01-01",
    );
    flow(
        &db,
        "stock-buy",
        "a",
        "USD",
        "TEST",
        "BUY",
        20.0,
        2.0,
        "2025-04-01",
    );
    flow(
        &db,
        "stock-sell",
        "a",
        "USD",
        "TEST",
        "SELL",
        5.0,
        1.0,
        "2025-05-01",
    );
    flow(
        &db,
        "dividend",
        "a",
        "USD",
        "TEST",
        "PAY",
        4.0,
        1.0,
        "2025-05-02",
    );
    flow(
        &db,
        "withdrawal",
        "a",
        "USD",
        "$CASH-USD",
        "SELL",
        6.0,
        1.0,
        "2025-07-01T00:30:00+08:00",
    );
    flow(
        &db,
        "future",
        "a",
        "USD",
        "$CASH-USD",
        "BUY",
        500.0,
        0.0,
        "2025-07-01T00:00:00Z",
    );
    flow(
        &db,
        "hkd-deposit",
        "a",
        "HKD",
        "$CASH-HKD",
        "BUY",
        80.0,
        0.0,
        "2025-02-01",
    );
    flow(
        &db,
        "hkd-withdrawal",
        "a",
        "HKD",
        "$CASH-HKD",
        "SELL",
        80.0,
        0.0,
        "2025-06-30",
    );
    flow(
        &db,
        "b-deposit",
        "b",
        "USD",
        "$CASH-USD",
        "BUY",
        50.0,
        0.0,
        "2025-02-01",
    );
    flow(
        &db,
        "b-withdrawal",
        "b",
        "USD",
        "$CASH-USD",
        "SELL",
        75.0,
        0.0,
        "2025-06-30",
    );
    assert_eq!(
        q2_cash(&db).unwrap(),
        BTreeMap::from([
            (("a".into(), "USD".into()), 79.0),
            (("a".into(), "HKD".into()), 0.0),
            (("b".into(), "USD".into()), -25.0),
        ])
    );
    assert_eq!(
        db.conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM holdings", [], |row| row
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
}

#[test]
fn quarterly_cash_missing_current_cannot_borrow_another_ledger_or_future_deposit() {
    for (deposit_account, deposit_currency, deposit_date) in [
        ("b", "USD", "2025-01-01"),
        ("a", "HKD", "2025-01-01"),
        ("a", "USD", "2025-07-01"),
    ] {
        let db = Database::new(":memory:").unwrap();
        account(&db, "a");
        account(&db, "b");
        flow(
            &db,
            "stock-buy",
            "a",
            "USD",
            "TEST",
            "BUY",
            20.0,
            0.0,
            "2025-04-01",
        );
        flow(
            &db,
            "deposit",
            deposit_account,
            deposit_currency,
            &format!("$CASH-{deposit_currency}"),
            "BUY",
            100.0,
            0.0,
            deposit_date,
        );
        assert!(q2_cash(&db)
            .unwrap_err()
            .contains("missing historical cash baseline for a/USD"));
    }
}

#[test]
fn quarterly_cash_deposit_fallback_preserves_current_and_opening_priority() {
    let db = Database::new(":memory:").unwrap();
    account(&db, "a");
    account(&db, "b");
    current_cash(&db, "a", "USD", 9000.0);
    flow(
        &db,
        "a-deposit",
        "a",
        "USD",
        "$CASH-USD",
        "BUY",
        1000.0,
        0.0,
        "2025-01-01",
    );
    flow(
        &db,
        "a-withdrawal",
        "a",
        "USD",
        "$CASH-USD",
        "SELL",
        100.0,
        0.0,
        "2025-04-01",
    );
    flow(
        &db,
        "a-future",
        "a",
        "USD",
        "$CASH-USD",
        "BUY",
        200.0,
        0.0,
        "2025-07-01",
    );
    flow(
        &db,
        "b-deposit",
        "b",
        "USD",
        "$CASH-USD",
        "BUY",
        1000.0,
        0.0,
        "2025-01-01",
    );
    flow(
        &db,
        "b-open",
        "b",
        "USD",
        "$CASH-USD",
        "OPEN",
        42.0,
        0.0,
        "2025-04-01",
    );
    flow(
        &db,
        "b-withdrawal",
        "b",
        "USD",
        "$CASH-USD",
        "SELL",
        2.0,
        0.0,
        "2025-06-01",
    );
    assert_eq!(
        q2_cash(&db).unwrap(),
        BTreeMap::from([
            (("a".into(), "USD".into()), 8800.0),
            (("b".into(), "USD".into()), 40.0),
        ])
    );
}
