use super::*;
use crate::services::portfolio_mutation::{
    create_holding_in, create_transaction_in, CreateTransactionInput,
};
use crate::services::position_replay::{
    rebuild_all_position_groups, rebuild_position_group, PositionKey,
};

fn fixture() -> (Database, Holding, CreateHoldingInput) {
    let db = Database::new(":memory:").unwrap();
    let input = CreateHoldingInput {
        account_id: "a".into(),
        symbol: "AAPL".into(),
        name: "Apple".into(),
        market: "US".into(),
        category_id: None,
        shares: 10.0,
        avg_cost: 10.0,
        currency: "USD".into(),
    };
    let holding = {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts VALUES ('a','A','US',NULL,'2026-01-01','2026-01-01')",
            [],
        )
        .unwrap();
        let holding = create_holding_in(&conn, &input).unwrap();
        conn.execute(
            "UPDATE transactions SET traded_at='2026-01-01T00:00:00Z', notes='original source'",
            [],
        )
        .unwrap();
        holding
    };
    (db, holding, input)
}

fn position(conn: &rusqlite::Connection, id: &str) -> (f64, f64) {
    conn.query_row(
        "SELECT shares,avg_cost FROM holdings WHERE id=?1",
        [id],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .unwrap()
}

fn add_buy(db: &Database) {
    let mut conn = db.conn.lock().unwrap();
    let tx = conn.transaction().unwrap();
    create_transaction_in(
        &tx,
        &CreateTransactionInput {
            account_id: "a".into(),
            symbol: "AAPL".into(),
            name: "Apple".into(),
            market: "US".into(),
            transaction_type: "BUY".into(),
            shares: 5.0,
            price: 10.0,
            total_amount: 50.0,
            commission: 0.0,
            currency: "USD".into(),
            traded_at: "2026-02-01T00:00:00Z".into(),
            notes: None,
        },
    )
    .unwrap();
    tx.commit().unwrap();
}

#[test]
fn opening_edit_survives_replay_and_preserves_effective_date() {
    let (db, holding, mut input) = fixture();
    input.shares = 20.0;
    input.avg_cost = 15.0;
    update_holding(&db, holding.id.clone(), input).unwrap();
    let conn = db.conn.lock().unwrap();
    rebuild_position_group(&conn, &PositionKey::new("a", "AAPL")).unwrap();
    assert_eq!(position(&conn, &holding.id), (20.0, 15.0));
    let opening: (f64, String, String) = conn
        .query_row(
            "SELECT total_amount,traded_at,notes FROM transactions WHERE transaction_type='OPEN'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .unwrap();
    assert_eq!(opening.0, 300.0);
    assert_eq!(opening.1, "2026-01-01T00:00:00Z");
    assert!(opening.2.contains("original source"));
}

#[test]
fn traded_position_rejects_direct_financial_and_identity_changes() {
    let (db, holding, input) = fixture();
    add_buy(&db);
    for change_identity in [false, true] {
        let mut changed = input.clone();
        changed.shares = 15.0;
        if change_identity {
            changed.symbol = "MSFT".into();
        } else {
            changed.avg_cost = 20.0;
        }
        assert!(update_holding(&db, holding.id.clone(), changed).is_err());
        let conn = db.conn.lock().unwrap();
        assert_eq!(position(&conn, &holding.id), (15.0, 10.0));
        let symbol: String = conn
            .query_row(
                "SELECT symbol FROM holdings WHERE id=?1",
                [&holding.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(symbol, "AAPL");
    }
}

#[test]
fn opening_identity_edit_moves_baseline_without_recreating_old_position() {
    let (db, holding, mut input) = fixture();
    input.symbol = "MSFT".into();
    input.name = "Microsoft".into();
    update_holding(&db, holding.id.clone(), input).unwrap();
    let conn = db.conn.lock().unwrap();
    rebuild_all_position_groups(&conn).unwrap();
    let positions: Vec<(String, f64)> = conn
        .prepare("SELECT symbol,shares FROM holdings")
        .unwrap()
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(Result::unwrap)
        .collect();
    assert_eq!(positions, vec![("MSFT".into(), 10.0)]);
}

#[test]
fn opening_sync_failure_rolls_back_holding_changes() {
    let (db, holding, mut input) = fixture();
    db.conn.lock().unwrap().execute_batch(
        "CREATE TRIGGER reject_open_edit BEFORE UPDATE ON transactions BEGIN SELECT RAISE(ABORT,'opening locked'); END;"
    ).unwrap();
    input.shares = 20.0;
    assert!(update_holding(&db, holding.id.clone(), input).is_err());
    assert_eq!(
        position(&db.conn.lock().unwrap(), &holding.id),
        (10.0, 10.0)
    );
}

#[test]
fn legacy_position_edit_creates_replayable_opening() {
    let (db, holding, mut input) = fixture();
    db.conn
        .lock()
        .unwrap()
        .execute("DELETE FROM transactions", [])
        .unwrap();
    input.shares = 20.0;
    input.avg_cost = 15.0;
    update_holding(&db, holding.id.clone(), input).unwrap();
    let conn = db.conn.lock().unwrap();
    rebuild_all_position_groups(&conn).unwrap();
    assert_eq!(position(&conn, &holding.id), (20.0, 15.0));
}

#[test]
fn metadata_edit_preserves_negative_derived_cost() {
    let (db, holding, mut input) = fixture();
    add_buy(&db);
    db.conn
        .lock()
        .unwrap()
        .execute("UPDATE holdings SET avg_cost=-2 WHERE id=?1", [&holding.id])
        .unwrap();
    input.shares = 15.0;
    input.avg_cost = -2.0;
    input.name = "Renamed".into();
    update_holding(&db, holding.id.clone(), input).unwrap();
    assert_eq!(
        position(&db.conn.lock().unwrap(), &holding.id),
        (15.0, -2.0)
    );
}

#[test]
fn corrected_opening_is_used_by_subsequent_buy() {
    let (db, holding, mut input) = fixture();
    input.shares = 20.0;
    input.avg_cost = 15.0;
    update_holding(&db, holding.id.clone(), input).unwrap();
    add_buy(&db);
    let conn = db.conn.lock().unwrap();
    assert_eq!(position(&conn, &holding.id), (25.0, 14.0));
    rebuild_all_position_groups(&conn).unwrap();
    assert_eq!(position(&conn, &holding.id), (25.0, 14.0));
}

#[test]
fn unlinked_trade_still_protects_holding_from_overwrite() {
    let (db, holding, mut input) = fixture();
    add_buy(&db);
    db.conn
        .lock()
        .unwrap()
        .execute(
            "UPDATE transactions SET holding_id=NULL WHERE transaction_type='BUY'",
            [],
        )
        .unwrap();
    input.shares = 20.0;
    assert!(update_holding(&db, holding.id.clone(), input).is_err());
    assert_eq!(
        position(&db.conn.lock().unwrap(), &holding.id),
        (15.0, 10.0)
    );
}

#[test]
fn identity_edit_rejects_orphan_history_at_destination() {
    let (db, holding, mut input) = fixture();
    db.conn.lock().unwrap().execute(
        "INSERT INTO transactions SELECT 'orphan',NULL,account_id,'MSFT','Microsoft',market,
           'OPEN',shares,price,total_amount,commission,currency,traded_at,notes,created_at FROM transactions", []
    ).unwrap();
    input.symbol = "MSFT".into();
    assert!(update_holding(&db, holding.id.clone(), input).is_err());
    let conn = db.conn.lock().unwrap();
    let source: String = conn
        .query_row(
            "SELECT symbol FROM transactions WHERE holding_id=?1",
            [&holding.id],
            |r| r.get(0),
        )
        .unwrap();
    assert_eq!(source, "AAPL");
    assert_eq!(position(&conn, &holding.id), (10.0, 10.0));
}

#[test]
fn cash_opening_can_be_corrected_but_stock_cash_flows_lock_its_balance() {
    let (db, _, mut input) = fixture();
    input.symbol = "$CASH-USD".into();
    input.name = "Cash".into();
    input.shares = 1000.25;
    input.avg_cost = 1.0;
    let cash = create_holding_in(&db.conn.lock().unwrap(), &input).unwrap();
    input.shares = 1100.50;
    update_holding(&db, cash.id.clone(), input.clone()).unwrap();
    {
        let conn = db.conn.lock().unwrap();
        let opening: f64 = conn
            .query_row(
                "SELECT total_amount FROM transactions WHERE holding_id=?1",
                [&cash.id],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(opening, 1100.50);
    }
    add_buy(&db);
    assert!(update_holding(&db, cash.id.clone(), input.clone()).is_err());
    input.shares = 1050.50;
    input.name = "Renamed cash".into();
    update_holding(&db, cash.id.clone(), input.clone()).unwrap();
    assert_eq!(position(&db.conn.lock().unwrap(), &cash.id), (1050.50, 1.0));
    input.symbol = "MSFT".into();
    assert!(update_holding(&db, cash.id, input).is_err());
}

#[test]
fn opening_correction_invalidates_only_affected_daily_snapshots() {
    let (db, holding, mut input) = fixture();
    {
        let conn = db.conn.lock().unwrap();
        for date in ["2025-12-31", "2026-01-01", "2026-02-01"] {
            conn.execute(
                "INSERT INTO daily_portfolio_values(date,total_value) VALUES(?1,100)",
                [date],
            )
            .unwrap();
            conn.execute("INSERT INTO daily_holding_snapshots(date,account_id,symbol,market) VALUES(?1,'a','AAPL','US')", [date]).unwrap();
        }
    }
    input.shares = 20.0;
    update_holding(&db, holding.id, input).unwrap();
    let conn = db.conn.lock().unwrap();
    for table in ["daily_portfolio_values", "daily_holding_snapshots"] {
        let dates: Vec<String> = conn
            .prepare(&format!("SELECT date FROM {table}"))
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .map(Result::unwrap)
            .collect();
        assert_eq!(dates, vec!["2025-12-31"]);
    }
}
