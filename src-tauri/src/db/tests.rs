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
        // Verify tables exist
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('accounts', 'categories', 'holdings', 'transactions')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 4);
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
}
