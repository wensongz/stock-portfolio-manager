use rusqlite::{Connection, Result};

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
