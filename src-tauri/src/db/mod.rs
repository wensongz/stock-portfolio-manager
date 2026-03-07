use rusqlite::{Connection, Result};
use std::sync::Mutex;

pub struct Database {
    pub conn: Mutex<Connection>,
}

impl Database {
    pub fn new(path: &str) -> Result<Self> {
        let conn = Connection::open(path)?;
        let db = Database {
            conn: Mutex::new(conn),
        };
        db.run_migrations()?;
        Ok(db)
    }

    fn run_migrations(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();

        conn.execute_batch("PRAGMA foreign_keys = ON;")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS accounts (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
                description TEXT,
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS categories (
                id TEXT PRIMARY KEY NOT NULL,
                name TEXT NOT NULL,
                color TEXT NOT NULL,
                icon TEXT NOT NULL,
                is_system INTEGER NOT NULL DEFAULT 0,
                sort_order INTEGER NOT NULL DEFAULT 0,
                created_at TEXT NOT NULL
            );
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS holdings (
                id TEXT PRIMARY KEY NOT NULL,
                account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                symbol TEXT NOT NULL,
                name TEXT NOT NULL,
                market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
                category_id TEXT REFERENCES categories(id) ON DELETE SET NULL,
                shares REAL NOT NULL DEFAULT 0,
                avg_cost REAL NOT NULL DEFAULT 0,
                currency TEXT NOT NULL CHECK(currency IN ('USD', 'CNY', 'HKD')),
                created_at TEXT NOT NULL,
                updated_at TEXT NOT NULL
            );
        ")?;

        conn.execute_batch("
            CREATE TABLE IF NOT EXISTS transactions (
                id TEXT PRIMARY KEY NOT NULL,
                holding_id TEXT REFERENCES holdings(id) ON DELETE SET NULL,
                account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                symbol TEXT NOT NULL,
                name TEXT NOT NULL,
                market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
                transaction_type TEXT NOT NULL CHECK(transaction_type IN ('BUY', 'SELL')),
                shares REAL NOT NULL,
                price REAL NOT NULL,
                total_amount REAL NOT NULL,
                commission REAL NOT NULL DEFAULT 0,
                currency TEXT NOT NULL CHECK(currency IN ('USD', 'CNY', 'HKD')),
                traded_at TEXT NOT NULL,
                notes TEXT,
                created_at TEXT NOT NULL
            );
        ")?;

        // Seed system categories (ignore if already exist)
        let categories = [
            (uuid::Uuid::new_v4().to_string(), "现金类", "#22C55E", "💵", 1, 1),
            (uuid::Uuid::new_v4().to_string(), "分红股", "#3B82F6", "💰", 1, 2),
            (uuid::Uuid::new_v4().to_string(), "成长股", "#F97316", "🚀", 1, 3),
            (uuid::Uuid::new_v4().to_string(), "套利",   "#8B5CF6", "🔄", 1, 4),
        ];

        let now = chrono::Utc::now().to_rfc3339();
        for (id, name, color, icon, is_system, sort_order) in &categories {
            conn.execute(
                "INSERT OR IGNORE INTO categories (id, name, color, icon, is_system, sort_order, created_at)
                 SELECT ?1, ?2, ?3, ?4, ?5, ?6, ?7
                 WHERE NOT EXISTS (SELECT 1 FROM categories WHERE name = ?2 AND is_system = 1)",
                rusqlite::params![id, name, color, icon, is_system, sort_order, now],
            )?;
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests;
