use super::*;

fn fixture() -> Database {
    let db = Database::new(":memory:").unwrap();
    db.conn
        .lock()
        .unwrap()
        .execute_batch(
            "INSERT INTO accounts(id,name,market,created_at,updated_at) VALUES
           ('a','Account A','US','2025-01-01','2025-01-01'),
           ('b','Account B','US','2025-01-01','2025-01-01'),
           ('c','Account C','US','2025-01-01','2025-01-01');
         INSERT INTO quarterly_snapshots(id,quarter,snapshot_date,created_at) VALUES
           ('q1','2026-Q1','2026-03-31','2026-04-01'),
           ('q3','2026-Q3','2026-09-07','2026-09-07');",
        )
        .unwrap();
    for snapshot in ["q1", "q3"] {
        for account in ["a", "b", "c"] {
            holding(
                &db,
                &format!("{snapshot}-{account}-stock"),
                snapshot,
                account,
                "ACME",
                "US",
                "USD",
            );
            holding(
                &db,
                &format!("{snapshot}-{account}-cash"),
                snapshot,
                account,
                "$CASH-USD",
                "US",
                "USD",
            );
        }
    }
    db
}

fn holding(
    db: &Database,
    id: &str,
    snapshot: &str,
    account: &str,
    symbol: &str,
    market: &str,
    currency: &str,
) {
    db.conn.lock().unwrap().execute(
        "INSERT INTO quarterly_holding_snapshots(id,quarterly_snapshot_id,account_id,symbol,name,market,currency,notes)
         VALUES(?1,?2,?3,?4,?4,?5,?6,?1)",
        rusqlite::params![id,snapshot,account,symbol,market,currency],
    ).unwrap();
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
    amount: f64,
    fee: f64,
    date: &str,
) {
    // No live holding is needed: historical transactions may outlive its row.
    db.conn.lock().unwrap().execute(
        "INSERT INTO transactions(id,account_id,symbol,name,market,currency,transaction_type,shares,price,total_amount,commission,traded_at,created_at,notes)
         VALUES(?1,?2,?3,?3,?4,?5,?6,?7,1,?7,?8,?9,?9,'original note')",
        rusqlite::params![id,account,symbol,market,currency,kind,amount,fee,date],
    ).unwrap();
}

fn ids(history: &QuarterlyHoldingHistory) -> Vec<&str> {
    history
        .rows
        .iter()
        .map(|row| row.transaction.id.as_str())
        .collect()
}

#[test]
fn quarterly_holding_history_scopes_stock_to_clicked_account_security_and_quarter() {
    let db = fixture();
    for (index, kind) in ["OPEN", "BUY", "SELL", "PAY", "STOCK_IN", "STOCK_OUT"]
        .iter()
        .enumerate()
    {
        for account in ["a", "b", "c"] {
            transaction(
                &db,
                &format!("{account}-{kind}"),
                account,
                "acme",
                "US",
                "USD",
                kind,
                10.0,
                1.0,
                &format!("2026-01-{:02}", index + 1),
            );
        }
    }
    transaction(
        &db,
        "q3-operation",
        "a",
        "ACME",
        "US",
        "USD",
        "BUY",
        999.0,
        0.0,
        "2026-07-01",
    );
    transaction(
        &db,
        "other-market",
        "a",
        "ACME",
        "HK",
        "USD",
        "BUY",
        999.0,
        0.0,
        "2026-01-01",
    );
    transaction(
        &db,
        "other-currency",
        "a",
        "ACME",
        "US",
        "HKD",
        "BUY",
        999.0,
        0.0,
        "2026-01-01",
    );
    transaction(
        &db,
        "other-symbol",
        "a",
        "OTHER",
        "US",
        "USD",
        "BUY",
        999.0,
        0.0,
        "2026-01-01",
    );

    let history = get_quarterly_holding_history(&db, "q1", "q1-a-stock").unwrap();
    assert_eq!(
        ids(&history),
        vec![
            "a-STOCK_OUT",
            "a-STOCK_IN",
            "a-PAY",
            "a-SELL",
            "a-BUY",
            "a-OPEN"
        ]
    );
    assert_eq!(
        (
            history.account_id.as_str(),
            history.symbol.as_str(),
            history.currency.as_str(),
            history.quarter.as_str()
        ),
        ("a", "ACME", "USD", "2026-Q1")
    );
    assert_eq!(
        (history.start_date.as_str(), history.end_date.as_str()),
        ("2026-01-01", "2026-03-31")
    );
    assert!(history.rows.iter().all(|row| row.cash_delta.is_none()
        && row.running_balance.is_none()
        && row.transaction.holding_id.is_none()));
    let json = serde_json::to_value(&history).unwrap();
    assert_eq!(json["rows"][0]["id"], "a-STOCK_OUT");
    assert_eq!(json["rows"][0]["notes"], "original note");
    assert!(json["rows"][0]["cash_delta"].is_null());
    assert!(json["rows"][0].get("transaction").is_none());
    let other_quarter = get_quarterly_holding_history(&db, "q3", "q3-a-stock").unwrap();
    assert_eq!(ids(&other_quarter), vec!["q3-operation"]);
}

#[test]
fn quarterly_holding_history_cash_preserves_prior_balance_and_filters_account_currency() {
    let db = fixture();
    transaction(
        &db,
        "prior-opening",
        "a",
        "$CASH-USD",
        "US",
        "USD",
        "OPEN",
        1000.0,
        0.0,
        "2025-12-01",
    );
    transaction(
        &db,
        "prior-buy",
        "a",
        "ACME",
        "US",
        "USD",
        "BUY",
        100.0,
        2.0,
        "2025-12-02",
    );
    transaction(
        &db,
        "deposit",
        "a",
        "$CASH-USD",
        "US",
        "USD",
        "BUY",
        50.0,
        1.0,
        "2026-01-01",
    );
    transaction(
        &db,
        "buy",
        "a",
        "ACME",
        "US",
        "USD",
        "BUY",
        20.0,
        2.0,
        "2026-01-02",
    );
    transaction(
        &db,
        "sell",
        "a",
        "ACME",
        "US",
        "USD",
        "SELL",
        40.0,
        3.0,
        "2026-01-03",
    );
    transaction(
        &db,
        "dividend",
        "a",
        "OTHER",
        "HK",
        "USD",
        "PAY",
        10.0,
        1.0,
        "2026-01-04",
    );
    transaction(
        &db,
        "withdrawal",
        "a",
        "$CASH-USD",
        "US",
        "USD",
        "SELL",
        5.0,
        1.0,
        "2026-03-31",
    );
    transaction(
        &db,
        "future-opening",
        "a",
        "$CASH-USD",
        "US",
        "USD",
        "OPEN",
        3000.0,
        0.0,
        "2026-07-01",
    );
    for kind in ["OPEN", "STOCK_IN", "STOCK_OUT"] {
        transaction(
            &db,
            kind,
            "a",
            "ACME",
            "US",
            "USD",
            kind,
            999.0,
            0.0,
            "2026-01-10",
        );
    }
    for account in ["b", "c"] {
        transaction(
            &db,
            account,
            account,
            "$CASH-USD",
            "US",
            "USD",
            "BUY",
            999.0,
            0.0,
            "2026-01-01",
        );
    }
    transaction(
        &db,
        "other-currency",
        "a",
        "$CASH-HKD",
        "US",
        "HKD",
        "BUY",
        999.0,
        0.0,
        "2026-01-01",
    );

    let history = get_quarterly_holding_history(&db, "q1", "q1-a-cash").unwrap();
    assert_eq!(
        ids(&history),
        vec!["withdrawal", "dividend", "sell", "buy", "deposit"]
    );
    assert_eq!(
        history
            .rows
            .iter()
            .map(|row| row.cash_delta)
            .collect::<Vec<_>>(),
        vec![Some(-6.0), Some(9.0), Some(37.0), Some(-22.0), Some(51.0)]
    );
    assert_eq!(
        history
            .rows
            .iter()
            .map(|row| row.running_balance)
            .collect::<Vec<_>>(),
        vec![
            Some(967.0),
            Some(973.0),
            Some(964.0),
            Some(927.0),
            Some(949.0)
        ]
    );
    assert!(history
        .rows
        .iter()
        .all(|row| row.transaction.account_id == "a" && row.transaction.currency == "USD"));
    let q3 = get_quarterly_holding_history(&db, "q3", "q3-a-cash").unwrap();
    assert_eq!(ids(&q3), vec!["future-opening"]);
    assert_eq!(
        (q3.rows[0].cash_delta, q3.rows[0].running_balance),
        (Some(2033.0), Some(3000.0))
    );
}

#[test]
fn quarterly_holding_history_uses_utc_quarter_bounds_and_stable_time_order() {
    let db = fixture();
    for (id, date) in [
        ("outside-start", "2026-01-01T00:30:00+08:00"),
        ("start", "2026-01-01T00:00:00Z"),
        ("last-day", "2026-04-01T00:30:00+08:00"),
        ("outside-end", "2026-03-31T18:00:00-07:00"),
        ("next-quarter", "2026-04-01T00:00:00Z"),
        ("same-time-a", "2026-01-02T00:00:00Z"),
        ("same-time-b", "2026-01-02T08:00:00+08:00"),
        ("same-time-c", "2026-01-02T00:00:00Z"),
    ] {
        transaction(&db, id, "a", "ACME", "US", "USD", "BUY", 1.0, 0.0, date);
    }
    db.conn.lock().unwrap().execute_batch(
        "UPDATE transactions SET created_at='2026-01-03T23:00:00+08:00' WHERE id='same-time-a';
         UPDATE transactions SET created_at='2026-01-03T20:00:00Z' WHERE id IN ('same-time-b','same-time-c');"
    ).unwrap();
    let history = get_quarterly_holding_history(&db, "q1", "q1-a-stock").unwrap();
    assert_eq!(
        ids(&history),
        vec![
            "last-day",
            "same-time-c",
            "same-time-b",
            "same-time-a",
            "start"
        ]
    );
    let cash = get_quarterly_holding_history(&db, "q1", "q1-a-cash").unwrap();
    assert_eq!(ids(&cash), ids(&history));
    assert_eq!(cash.rows.last().unwrap().running_balance, Some(-2.0));
}

#[test]
fn quarterly_holding_history_returns_empty_quarter_without_inventing_operations() {
    let db = fixture();
    transaction(
        &db,
        "before",
        "a",
        "$CASH-USD",
        "US",
        "USD",
        "OPEN",
        100.0,
        0.0,
        "2025-12-01",
    );
    for holding_id in ["q1-a-stock", "q1-a-cash", "q1-b-cash"] {
        let history = get_quarterly_holding_history(&db, "q1", holding_id).unwrap();
        assert_eq!(history.snapshot_id, "q1");
        assert_eq!(history.holding_snapshot_id, holding_id);
        assert!(history.rows.is_empty());
    }
}

#[test]
fn quarterly_holding_history_rejects_missing_or_foreign_snapshot_rows() {
    let db = fixture();
    for (snapshot, holding) in [
        ("missing", "q1-a-stock"),
        ("q1", "missing"),
        ("q1", "q3-a-stock"),
    ] {
        assert!(get_quarterly_holding_history(&db, snapshot, holding).is_err());
    }
}

#[test]
fn quarterly_holding_history_resolves_legacy_cash_currency_and_rejects_mismatch() {
    let db = fixture();
    holding(&db, "legacy", "q1", "a", "$CASH-HKD", "US", "");
    transaction(
        &db,
        "hkd",
        "a",
        "$CASH-HKD",
        "US",
        "HKD",
        "BUY",
        20.0,
        0.0,
        "2026-01-01",
    );
    let history = get_quarterly_holding_history(&db, "q1", "legacy").unwrap();
    assert_eq!(history.currency, "HKD");
    assert_eq!(ids(&history), vec!["hkd"]);
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE quarterly_holding_snapshots SET currency='USD' WHERE id='legacy'",
            [],
        )
        .unwrap();
    assert!(get_quarterly_holding_history(&db, "q1", "legacy").is_err());
}

#[test]
fn quarterly_holding_notes_save_targets_one_snapshot_row() {
    let db = fixture();
    assert!(update_holding_notes(&db, "q1", "q1-a-stock", "only A Q1").unwrap());
    assert!(!update_holding_notes(&db, "q1", "q3-a-stock", "wrong quarter").unwrap());
    assert!(!update_holding_notes(&db, "q1", "ACME", "symbol is not an id").unwrap());
    assert!(!update_holding_notes(&db, "q1", "missing", "missing").unwrap());
    let conn = db.conn.lock().unwrap();
    let mut statement = conn
        .prepare("SELECT id,notes FROM quarterly_holding_snapshots ORDER BY id")
        .unwrap();
    let notes = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    for (id, note) in notes {
        assert_eq!(note, if id == "q1-a-stock" { "only A Q1" } else { &id });
    }
}
