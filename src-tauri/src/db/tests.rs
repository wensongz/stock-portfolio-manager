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
        // Verify all tables exist (including Phase 5 quarterly tables + cached_quotes)
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('accounts', 'categories', 'holdings', 'transactions', 'daily_portfolio_values', 'daily_holding_snapshots', 'quarterly_snapshots', 'quarterly_holding_snapshots', 'cached_quotes')",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 9);
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

    #[test]
    fn test_quote_provider_config_table_exists() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='quote_provider_config'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_quote_provider_config_default() {
        let db = create_test_db();
        let config = crate::services::quote_provider_service::get_quote_provider_config(&db).unwrap();
        assert_eq!(config.us_provider, "eastmoney");
        assert_eq!(config.hk_provider, "eastmoney");
        assert_eq!(config.cn_provider, "eastmoney");
    }

    #[test]
    fn test_quote_provider_config_update_and_get() {
        let db = create_test_db();
        let config = crate::models::quote_provider::QuoteProviderConfig {
            us_provider: "yahoo".to_string(),
            hk_provider: "yahoo".to_string(),
            cn_provider: "eastmoney".to_string(),
        };
        let result = crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
        assert!(result.is_ok());

        let loaded = crate::services::quote_provider_service::get_quote_provider_config(&db).unwrap();
        assert_eq!(loaded.us_provider, "yahoo");
        assert_eq!(loaded.hk_provider, "yahoo");
        assert_eq!(loaded.cn_provider, "eastmoney");
    }

    #[test]
    fn test_quote_provider_config_invalid_us_provider() {
        let db = create_test_db();
        let config = crate::models::quote_provider::QuoteProviderConfig {
            us_provider: "invalid".to_string(),
            hk_provider: "yahoo".to_string(),
            cn_provider: "eastmoney".to_string(),
        };
        let result = crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_quote_provider_config_invalid_cn_provider() {
        let db = create_test_db();
        let config = crate::models::quote_provider::QuoteProviderConfig {
            us_provider: "yahoo".to_string(),
            hk_provider: "yahoo".to_string(),
            cn_provider: "yahoo".to_string(),
        };
        let result = crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_cached_quotes_table_exists() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='cached_quotes'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(count, 1);
    }

    #[test]
    fn test_save_and_load_cached_quotes() {
        let db = create_test_db();
        let quotes = vec![
            crate::models::StockQuote {
                symbol: "AAPL".to_string(),
                name: "Apple Inc.".to_string(),
                market: "US".to_string(),
                current_price: 175.50,
                previous_close: 174.0,
                change: 1.50,
                change_percent: 0.86,
                high: 176.0,
                low: 173.0,
                volume: 50000000,
                updated_at: "2024-01-15T16:00:00Z".to_string(),
            },
            crate::models::StockQuote {
                symbol: "sh600519".to_string(),
                name: "贵州茅台".to_string(),
                market: "CN".to_string(),
                current_price: 1800.0,
                previous_close: 1790.0,
                change: 10.0,
                change_percent: 0.56,
                high: 1810.0,
                low: 1785.0,
                volume: 3000000,
                updated_at: "2024-01-15T15:00:00Z".to_string(),
            },
        ];

        let save_result = crate::services::quote_service::save_quotes_to_db(&db, &quotes);
        assert!(save_result.is_ok());

        let loaded = crate::services::quote_service::load_quotes_from_db(&db).unwrap();
        assert_eq!(loaded.len(), 2);

        let aapl = loaded.iter().find(|q| q.symbol == "AAPL").unwrap();
        assert_eq!(aapl.name, "Apple Inc.");
        assert!((aapl.current_price - 175.50).abs() < 0.001);
        assert_eq!(aapl.volume, 50000000);

        let moutai = loaded.iter().find(|q| q.symbol == "sh600519").unwrap();
        assert_eq!(moutai.name, "贵州茅台");
        assert!((moutai.current_price - 1800.0).abs() < 0.001);
    }

    #[test]
    fn test_cached_quotes_upsert() {
        let db = create_test_db();
        let quote = crate::models::StockQuote {
            symbol: "AAPL".to_string(),
            name: "Apple Inc.".to_string(),
            market: "US".to_string(),
            current_price: 175.50,
            previous_close: 174.0,
            change: 1.50,
            change_percent: 0.86,
            high: 176.0,
            low: 173.0,
            volume: 50000000,
            updated_at: "2024-01-15T16:00:00Z".to_string(),
        };
        crate::services::quote_service::save_quotes_to_db(&db, &[quote]).unwrap();

        // Update with new price
        let updated_quote = crate::models::StockQuote {
            symbol: "AAPL".to_string(),
            name: "Apple Inc.".to_string(),
            market: "US".to_string(),
            current_price: 180.0,
            previous_close: 175.50,
            change: 4.50,
            change_percent: 2.56,
            high: 181.0,
            low: 175.0,
            volume: 60000000,
            updated_at: "2024-01-16T16:00:00Z".to_string(),
        };
        crate::services::quote_service::save_quotes_to_db(&db, &[updated_quote]).unwrap();

        let loaded = crate::services::quote_service::load_quotes_from_db(&db).unwrap();
        assert_eq!(loaded.len(), 1); // Should be 1 row, not 2
        assert!((loaded[0].current_price - 180.0).abs() < 0.001);
        assert_eq!(loaded[0].volume, 60000000);
    }

    #[test]
    fn test_load_cached_quotes_empty() {
        let db = create_test_db();
        let loaded = crate::services::quote_service::load_quotes_from_db(&db).unwrap();
        assert!(loaded.is_empty());
    }
}
