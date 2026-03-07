use rusqlite::Connection;

pub fn run_all(conn: &Connection) -> Result<(), rusqlite::Error> {
    conn.execute_batch(
        "
        CREATE TABLE IF NOT EXISTS categories (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            color TEXT NOT NULL DEFAULT '#808080',
            icon TEXT NOT NULL DEFAULT '',
            is_system INTEGER NOT NULL DEFAULT 0,
            sort_order INTEGER NOT NULL DEFAULT 0,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS accounts (
            id TEXT PRIMARY KEY,
            name TEXT NOT NULL,
            market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
            description TEXT DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        );

        CREATE TABLE IF NOT EXISTS holdings (
            id TEXT PRIMARY KEY,
            account_id TEXT NOT NULL,
            symbol TEXT NOT NULL,
            name TEXT NOT NULL,
            market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
            category_id TEXT,
            shares REAL NOT NULL DEFAULT 0,
            avg_cost REAL NOT NULL DEFAULT 0,
            currency TEXT NOT NULL CHECK(currency IN ('USD', 'CNY', 'HKD')),
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE,
            FOREIGN KEY (category_id) REFERENCES categories(id) ON DELETE SET NULL
        );

        CREATE TABLE IF NOT EXISTS transactions (
            id TEXT PRIMARY KEY,
            holding_id TEXT,
            account_id TEXT NOT NULL,
            symbol TEXT NOT NULL,
            name TEXT NOT NULL,
            market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
            type TEXT NOT NULL CHECK(type IN ('BUY', 'SELL')),
            shares REAL NOT NULL,
            price REAL NOT NULL,
            total_amount REAL NOT NULL,
            commission REAL NOT NULL DEFAULT 0,
            currency TEXT NOT NULL CHECK(currency IN ('USD', 'CNY', 'HKD')),
            traded_at TEXT NOT NULL,
            notes TEXT DEFAULT '',
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            FOREIGN KEY (holding_id) REFERENCES holdings(id) ON DELETE SET NULL,
            FOREIGN KEY (account_id) REFERENCES accounts(id) ON DELETE CASCADE
        );

        -- Seed default categories if they don't exist
        INSERT OR IGNORE INTO categories (id, name, color, icon, is_system, sort_order)
        VALUES
            ('cat-cash', '现金类', '#22C55E', '💵', 1, 1),
            ('cat-dividend', '分红股', '#3B82F6', '💰', 1, 2),
            ('cat-growth', '成长股', '#F97316', '🚀', 1, 3),
            ('cat-arbitrage', '套利', '#8B5CF6', '🔄', 1, 4);
        ",
    )?;
    Ok(())
}
