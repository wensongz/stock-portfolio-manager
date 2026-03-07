#[cfg(test)]
mod tests {
    use crate::db::Database;

    fn create_test_db() -> Database {
        Database::new(":memory:").expect("failed to create in-memory database")
    }

    #[test]
    fn test_database_creation() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        // Verify all tables exist (including new Phase 2 tables)
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('accounts', 'categories', 'holdings', 'transactions', 'daily_portfolio_values', 'daily_holding_snapshots')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 6);
    }

    #[test]
    fn test_system_categories_seeded() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM categories WHERE is_system = 1",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 4);
    }

    #[test]
    fn test_system_category_names() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT name FROM categories WHERE is_system = 1 ORDER BY sort_order")
            .unwrap();
        let names: Vec<String> = stmt
            .query_map([], |row| row.get(0))
            .unwrap()
            .map(|r| r.unwrap())
            .collect();
        assert_eq!(names, vec!["现金类", "分红股", "成长股", "套利"]);
    }

    #[test]
    fn test_create_and_get_account() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO accounts (id, name, market, description, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![id, "Robinhood", "US", Option::<String>::None, now, now],
        ).unwrap();
        let name: String = conn
            .query_row(
                "SELECT name FROM accounts WHERE id = ?1",
                rusqlite::params![id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(name, "Robinhood");
    }

    #[test]
    fn test_foreign_key_constraint() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        // Try to insert a holding with non-existent account_id
        let result = conn.execute(
            "INSERT INTO holdings (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
             VALUES ('h1', 'nonexistent', 'AAPL', 'Apple', 'US', 100.0, 150.0, 'USD', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
            [],
        );
        assert!(result.is_err(), "Should fail due to FK constraint");
    }

    #[test]
    fn test_market_check_constraint() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let result = conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at) VALUES ('a1', 'Test', 'INVALID', '2024-01-01T00:00:00Z', '2024-01-01T00:00:00Z')",
            [],
        );
        assert!(result.is_err(), "Should fail due to CHECK constraint on market");
    }

    #[test]
    fn test_daily_portfolio_values_table() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO daily_portfolio_values (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value, hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
             VALUES ('2024-01-15', 10000.0, 12000.0, 10000.0, 12000.0, 0.0, 0.0, 0.0, 0.0, '{}', 2000.0, 2000.0)",
            [],
        ).unwrap();
        let total_value: f64 = conn
            .query_row(
                "SELECT total_value FROM daily_portfolio_values WHERE date = '2024-01-15'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!((total_value - 12000.0).abs() < 0.001);
    }

    #[test]
    fn test_daily_portfolio_values_upsert() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        // Insert once
        conn.execute(
            "INSERT OR REPLACE INTO daily_portfolio_values (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value, hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
             VALUES ('2024-01-15', 10000.0, 12000.0, 10000.0, 12000.0, 0.0, 0.0, 0.0, 0.0, '{}', 2000.0, 2000.0)",
            [],
        ).unwrap();
        // Replace
        conn.execute(
            "INSERT OR REPLACE INTO daily_portfolio_values (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value, hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
             VALUES ('2024-01-15', 11000.0, 13000.0, 11000.0, 13000.0, 0.0, 0.0, 0.0, 0.0, '{}', 2000.0, 2000.0)",
            [],
        ).unwrap();
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM daily_portfolio_values WHERE date = '2024-01-15'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        // UNIQUE constraint on date means upsert replaces the row
        assert_eq!(count, 1);
        let total_cost: f64 = conn
            .query_row(
                "SELECT total_cost FROM daily_portfolio_values WHERE date = '2024-01-15'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!((total_cost - 11000.0).abs() < 0.001);
    }

    #[test]
    fn test_daily_holding_snapshots_table() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        // Create a test account first
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at) VALUES ('acct1', 'Test', 'US', ?1, ?1)",
            rusqlite::params![now],
        ).unwrap();
        conn.execute(
            "INSERT INTO daily_holding_snapshots (date, account_id, symbol, market, category_name, shares, avg_cost, close_price, market_value)
             VALUES ('2024-01-15', 'acct1', 'AAPL', 'US', 'Growth', 100.0, 150.0, 175.0, 17500.0)",
            [],
        ).unwrap();
        let market_value: f64 = conn
            .query_row(
                "SELECT market_value FROM daily_holding_snapshots WHERE date = '2024-01-15' AND symbol = 'AAPL'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert!((market_value - 17500.0).abs() < 0.001);
    }
}
