use rusqlite::{Connection, Result};

pub(super) fn migrate_v12(conn: &Connection) -> Result<()> {
    if !super::migrations::column_exists(conn, "daily_holding_snapshots", "has_market_close")? {
        conn.execute_batch(
            "ALTER TABLE daily_holding_snapshots ADD COLUMN has_market_close INTEGER
             CHECK (has_market_close IN (0, 1));",
        )?;
    }
    Ok(())
}

pub(super) fn migrate_v7(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS snapshot_cache_state (
           id INTEGER PRIMARY KEY CHECK (id = 1),
           revision INTEGER NOT NULL DEFAULT 0
         );
         INSERT OR IGNORE INTO snapshot_cache_state (id, revision) VALUES (1, 0);

         CREATE TRIGGER IF NOT EXISTS snapshot_revision_transaction_insert
         AFTER INSERT ON transactions BEGIN
           UPDATE snapshot_cache_state SET revision = revision + 1 WHERE id = 1;
         END;
         CREATE TRIGGER IF NOT EXISTS snapshot_revision_transaction_delete
         AFTER DELETE ON transactions BEGIN
           UPDATE snapshot_cache_state SET revision = revision + 1 WHERE id = 1;
         END;
         CREATE TRIGGER IF NOT EXISTS snapshot_revision_transaction_update
         AFTER UPDATE ON transactions
         WHEN OLD.account_id IS NOT NEW.account_id
           OR OLD.symbol IS NOT NEW.symbol OR OLD.market IS NOT NEW.market
           OR OLD.transaction_type IS NOT NEW.transaction_type
           OR OLD.shares IS NOT NEW.shares OR OLD.price IS NOT NEW.price
           OR OLD.total_amount IS NOT NEW.total_amount OR OLD.commission IS NOT NEW.commission
           OR OLD.currency IS NOT NEW.currency OR OLD.traded_at IS NOT NEW.traded_at
         BEGIN
           UPDATE snapshot_cache_state SET revision = revision + 1 WHERE id = 1;
         END;

         CREATE TRIGGER IF NOT EXISTS snapshot_revision_holding_insert
         AFTER INSERT ON holdings BEGIN
           UPDATE snapshot_cache_state SET revision = revision + 1 WHERE id = 1;
         END;
         CREATE TRIGGER IF NOT EXISTS snapshot_revision_holding_delete
         AFTER DELETE ON holdings BEGIN
           UPDATE snapshot_cache_state SET revision = revision + 1 WHERE id = 1;
         END;
         CREATE TRIGGER IF NOT EXISTS snapshot_revision_holding_update
         AFTER UPDATE ON holdings
         WHEN OLD.account_id IS NOT NEW.account_id
           OR OLD.symbol IS NOT NEW.symbol OR OLD.market IS NOT NEW.market
           OR OLD.category_id IS NOT NEW.category_id
           OR OLD.shares IS NOT NEW.shares OR OLD.avg_cost IS NOT NEW.avg_cost
           OR OLD.currency IS NOT NEW.currency OR OLD.created_at IS NOT NEW.created_at
         BEGIN
           UPDATE snapshot_cache_state SET revision = revision + 1 WHERE id = 1;
         END;

         -- Earlier releases could leave stale daily values after ledger edits.
         -- Rebuild these derived caches on demand; quarterly reviews are independent.
         DELETE FROM daily_holding_snapshots;
         DELETE FROM daily_portfolio_values;",
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn v12_preserves_legacy_snapshots_with_unknown_close_status() {
        let mut conn = rusqlite::Connection::open_in_memory().unwrap();
        super::super::schema::create_current_schema(&conn).unwrap();
        conn.execute_batch(
            "PRAGMA user_version = 11;
             INSERT INTO daily_holding_snapshots(date, account_id, symbol, market, close_price)
               VALUES ('2024-01-02', 'account', 'AAPL', 'US', 20);",
        )
        .unwrap();
        super::super::migrations::run_migrations(&mut conn).unwrap();
        let legacy: (f64, Option<bool>) = conn
            .query_row(
                "SELECT close_price, has_market_close FROM daily_holding_snapshots",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(legacy, (20.0, None));
    }
}
