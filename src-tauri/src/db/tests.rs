#[cfg(test)]
#[allow(clippy::module_inception)]
mod tests {
    use crate::db::{
        migrations::{column_exists, run_migrations, CURRENT_SCHEMA_VERSION},
        schema, Database,
    };
    use rusqlite::Connection;

    fn create_test_db() -> Database {
        Database::new(":memory:").expect("failed to create in-memory database")
    }

    fn schema_version(conn: &Connection) -> i64 {
        conn.pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap()
    }

    fn table_exists(conn: &Connection, table: &str) -> bool {
        conn.query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1",
            [table],
            |_| Ok(()),
        )
        .is_ok()
    }

    fn price_alert_count(conn: &Connection) -> i64 {
        conn.query_row("SELECT COUNT(*) FROM price_alerts", [], |row| row.get(0))
            .unwrap()
    }

    fn target_count(conn: &Connection, config_id: &str) -> i64 {
        conn.query_row(
            "SELECT COUNT(*) FROM portfolio_alert_targets WHERE config_id = ?1",
            [config_id],
            |row| row.get(0),
        )
        .unwrap()
    }

    fn version_three_database_with_price_alert() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE price_alerts (
               id TEXT PRIMARY KEY NOT NULL,
               holding_id TEXT,
               symbol TEXT NOT NULL,
               name TEXT NOT NULL,
               market TEXT NOT NULL,
               alert_type TEXT NOT NULL,
               threshold REAL NOT NULL,
               is_active INTEGER NOT NULL,
               is_triggered INTEGER NOT NULL,
               triggered_at TEXT,
               created_at TEXT NOT NULL
             );
             INSERT INTO price_alerts VALUES
               ('price-alert-1', NULL, 'AAPL', 'Apple', 'US', 'PRICE_ABOVE', 200, 1, 0, NULL, 'old');
             PRAGMA user_version = 3;",
        )
        .unwrap();
        conn
    }

    fn migrated_database() -> Connection {
        let mut conn = Connection::open_in_memory().unwrap();
        run_migrations(&mut conn).unwrap();
        conn
    }

    fn seed_account_and_category(conn: &Connection, account_id: &str, category_id: &str) {
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at)
             VALUES (?1, 'Test account', 'US', '2026-09-06', '2026-09-06')",
            [account_id],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO categories (id, name, color, icon, created_at)
             VALUES (?1, 'Growth', '#F97316', '🚀', '2026-09-06')",
            [category_id],
        )
        .unwrap();
    }

    fn insert_market_config(conn: &Connection, id: &str, market: &str) -> rusqlite::Result<usize> {
        conn.execute(
            "INSERT INTO portfolio_alert_configs
               (id, scope_key, scope_kind, market, account_id, base_currency,
                deviation_threshold, concentration_threshold, is_active, created_at, updated_at)
             VALUES (?1, ?2, 'MARKET', ?3, NULL, 'USD', 20, 20, 1, '2026-09-06', '2026-09-06')",
            rusqlite::params![id, format!("market:{market}"), market],
        )
    }

    fn insert_target(conn: &Connection, config_id: &str, category_id: &str, target_percent: f64) {
        conn.execute(
            "INSERT INTO portfolio_alert_targets (config_id, category_id, target_percent)
             VALUES (?1, ?2, ?3)",
            rusqlite::params![config_id, category_id, target_percent],
        )
        .unwrap();
    }

    #[test]
    fn migration_v4_adds_portfolio_alert_tables_without_touching_price_alerts() {
        let mut conn = version_three_database_with_price_alert();

        run_migrations(&mut conn).unwrap();

        assert_eq!(schema_version(&conn), CURRENT_SCHEMA_VERSION);
        for table in [
            "portfolio_alert_configs",
            "portfolio_alert_targets",
            "portfolio_alert_breaches",
        ] {
            assert!(table_exists(&conn, table));
        }
        assert_eq!(price_alert_count(&conn), 1);
    }

    #[test]
    fn portfolio_alert_scope_is_unique_and_deleted_category_targets_cascade() {
        let conn = migrated_database();
        seed_account_and_category(&conn, "acct-1", "cat-growth");
        insert_market_config(&conn, "config-1", "US").unwrap();
        insert_target(&conn, "config-1", "cat-growth", 60.0);

        let duplicate = insert_market_config(&conn, "config-2", "US");
        assert!(duplicate.is_err());

        conn.execute("DELETE FROM categories WHERE id = 'cat-growth'", [])
            .unwrap();
        assert_eq!(target_count(&conn, "config-1"), 0);
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
    fn stock_operation_review_creates_only_the_price_cache() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let price_cache: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='stock_daily_prices'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let legacy_tables: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN (
                    'stock_market_sessions', 'stock_market_calendar_coverage',
                    'stock_review_annotations', 'stock_review_overrides'
                )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(price_cache, 1);
        assert_eq!(legacy_tables, 0);
    }

    #[test]
    fn reset_clears_price_cache_but_leaves_an_existing_legacy_table_inert() {
        let db = create_test_db();
        let mut conn = db.conn.lock().unwrap();
        conn.execute_batch(
            "INSERT INTO stock_daily_prices
               (symbol, market, date, close, source, updated_at)
             VALUES ('AAPL', 'US', '2026-07-31', 200, 'test', '2026-07-31');
             CREATE TABLE stock_review_overrides (id TEXT PRIMARY KEY);
             INSERT INTO stock_review_overrides VALUES ('legacy');",
        )
        .unwrap();
        let tx = conn.transaction().unwrap();
        crate::commands::reset::clear_stock_operation_review_cache(&tx).unwrap();
        tx.commit().unwrap();
        let cached_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM stock_daily_prices", [], |row| {
                row.get(0)
            })
            .unwrap();
        let inert_rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM stock_review_overrides", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(cached_rows, 0);
        assert_eq!(inert_rows, 1);
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
        assert!(
            result.is_err(),
            "Should fail due to CHECK constraint on market"
        );
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
        let config =
            crate::services::quote_provider_service::get_quote_provider_config(&db).unwrap();
        assert_eq!(config.us_provider, "xueqiu");
        assert_eq!(config.hk_provider, "xueqiu");
        assert_eq!(config.cn_provider, "xueqiu");
    }

    #[test]
    fn test_quote_provider_config_update_and_get() {
        let db = create_test_db();
        let config = crate::models::quote_provider::QuoteProviderConfig {
            us_provider: "yahoo".to_string(),
            hk_provider: "yahoo".to_string(),
            cn_provider: "eastmoney".to_string(),
            xueqiu_cookie: None,
            xueqiu_u: None,
            ..Default::default()
        };
        let result =
            crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
        assert!(result.is_ok());

        let loaded =
            crate::services::quote_provider_service::get_quote_provider_config(&db).unwrap();
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
            xueqiu_cookie: None,
            xueqiu_u: None,
            ..Default::default()
        };
        let result =
            crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_quote_provider_config_invalid_cn_provider() {
        let db = create_test_db();
        let config = crate::models::quote_provider::QuoteProviderConfig {
            us_provider: "yahoo".to_string(),
            hk_provider: "yahoo".to_string(),
            cn_provider: "yahoo".to_string(),
            xueqiu_cookie: None,
            xueqiu_u: None,
            ..Default::default()
        };
        let result =
            crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
        assert!(result.is_err());
    }

    #[test]
    fn test_quote_provider_config_xueqiu_cookie_round_trip() {
        let db = create_test_db();
        let config = crate::models::quote_provider::QuoteProviderConfig {
            us_provider: "xueqiu".to_string(),
            hk_provider: "eastmoney".to_string(),
            cn_provider: "eastmoney".to_string(),
            xueqiu_cookie: Some("xq_a_token=abc123".to_string()),
            xueqiu_u: None,
            ..Default::default()
        };
        let result =
            crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
        assert!(result.is_ok());

        let loaded =
            crate::services::quote_provider_service::get_quote_provider_config(&db).unwrap();
        assert_eq!(loaded.xueqiu_cookie, Some("xq_a_token=abc123".to_string()));
    }

    #[test]
    fn test_quote_provider_config_xueqiu_u_round_trip() {
        let db = create_test_db();
        let config = crate::models::quote_provider::QuoteProviderConfig {
            us_provider: "xueqiu".to_string(),
            hk_provider: "eastmoney".to_string(),
            cn_provider: "eastmoney".to_string(),
            xueqiu_cookie: None,
            xueqiu_u: Some("9095890697".to_string()),
            ..Default::default()
        };
        let result =
            crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
        assert!(result.is_ok());

        let loaded =
            crate::services::quote_provider_service::get_quote_provider_config(&db).unwrap();
        assert_eq!(loaded.xueqiu_u, Some("9095890697".to_string()));
    }

    #[test]
    fn test_quote_provider_config_xueqiu_u_empty_normalized_to_none() {
        let db = create_test_db();
        let config = crate::models::quote_provider::QuoteProviderConfig {
            us_provider: "eastmoney".to_string(),
            hk_provider: "eastmoney".to_string(),
            cn_provider: "eastmoney".to_string(),
            xueqiu_cookie: None,
            xueqiu_u: Some("   ".to_string()),
            ..Default::default()
        };
        let result =
            crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
        assert!(result.is_ok());

        let loaded =
            crate::services::quote_provider_service::get_quote_provider_config(&db).unwrap();
        assert_eq!(loaded.xueqiu_u, None);
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
                pe_ttm: Some(31.2),
                pb: Some(48.5),
                market_cap: Some(3_200_000_000_000.0),
                dividend_yield: Some(0.41),
                eps: Some(7.15),
                roe: Some(152.0),
                turnover_rate: Some(0.61),
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
                ..Default::default()
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
        assert_eq!(aapl.pe_ttm, Some(31.2));
        assert_eq!(aapl.pb, Some(48.5));
        assert_eq!(aapl.market_cap, Some(3_200_000_000_000.0));
        assert_eq!(aapl.dividend_yield, Some(0.41));
        assert_eq!(aapl.eps, Some(7.15));
        assert_eq!(aapl.roe, Some(152.0));
        assert_eq!(aapl.turnover_rate, Some(0.61));

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
            ..Default::default()
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
            ..Default::default()
        };
        crate::services::quote_service::save_quotes_to_db(&db, &[updated_quote]).unwrap();

        let loaded = crate::services::quote_service::load_quotes_from_db(&db).unwrap();
        assert_eq!(loaded.len(), 1); // Should be 1 row, not 2
        assert!((loaded[0].current_price - 180.0).abs() < 0.001);
        assert_eq!(loaded[0].volume, 60000000);
    }

    #[test]
    fn cached_quotes_keep_same_symbol_in_different_markets_after_reload() {
        // A symbol is only meaningful within its market. Reopening the cache
        // must not discard either quote when two exchanges use the same text.
        let db = create_test_db();
        let quotes = vec![
            crate::models::StockQuote {
                symbol: "BABA".to_string(),
                name: "Alibaba US".to_string(),
                market: "US".to_string(),
                current_price: 120.0,
                updated_at: "2026-09-06T00:00:00Z".to_string(),
                ..Default::default()
            },
            crate::models::StockQuote {
                symbol: "BABA".to_string(),
                name: "Alibaba HK".to_string(),
                market: "HK".to_string(),
                current_price: 90.0,
                updated_at: "2026-09-06T00:00:00Z".to_string(),
                ..Default::default()
            },
        ];

        crate::services::quote_service::save_quotes_to_db(&db, &quotes).unwrap();
        let reloaded = crate::services::quote_service::load_quotes_from_db(&db).unwrap();
        let restarted_cache = crate::services::quote_service::QuoteCache::new();
        restarted_cache.set_batch(&reloaded);

        assert_eq!(reloaded.len(), 2);
        assert_eq!(
            restarted_cache.get("US", "BABA").unwrap().current_price,
            120.0
        );
        assert_eq!(
            restarted_cache.get("HK", "BABA").unwrap().current_price,
            90.0
        );
    }

    #[test]
    fn test_load_cached_quotes_empty() {
        let db = create_test_db();
        let loaded = crate::services::quote_service::load_quotes_from_db(&db).unwrap();
        assert!(loaded.is_empty());
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Transaction cost-basis and data integrity tests
    // ─────────────────────────────────────────────────────────────────────────

    /// Helper: create an account and a holding, returning (account_id, holding_id).
    fn setup_account_and_holding(
        conn: &rusqlite::Connection,
        symbol: &str,
        shares: f64,
        avg_cost: f64,
    ) -> (String, String) {
        let acct_id = uuid::Uuid::new_v4().to_string();
        let holding_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at) VALUES (?1, 'Test', 'US', ?2, ?2)",
            rusqlite::params![acct_id, now],
        ).unwrap();
        conn.execute(
            "INSERT INTO holdings (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3, 'US', ?4, ?5, 'USD', ?6, ?6)",
            rusqlite::params![holding_id, acct_id, symbol, shares, avg_cost, now],
        ).unwrap();
        (acct_id, holding_id)
    }

    /// Simulate a transaction and update holdings the same way create_transaction does.
    /// Returns Ok(new_shares, new_avg_cost) or Err if validation fails.
    fn simulate_transaction(
        conn: &rusqlite::Connection,
        acct_id: &str,
        symbol: &str,
        tx_type: &str,
        shares: f64,
        price: f64,
    ) -> Result<(f64, f64), String> {
        conn.execute_batch("BEGIN IMMEDIATE")
            .map_err(|e| e.to_string())?;

        let result = (|| -> Result<(f64, f64), String> {
            let holding_id: Option<String> = conn
                .query_row(
                    "SELECT id FROM holdings WHERE account_id = ?1 AND symbol = ?2",
                    rusqlite::params![acct_id, symbol],
                    |row| row.get(0),
                )
                .ok();

            if let Some(ref hid) = holding_id {
                let (current_shares, current_avg_cost): (f64, f64) = conn
                    .query_row(
                        "SELECT shares, avg_cost FROM holdings WHERE id = ?1",
                        rusqlite::params![hid],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .map_err(|e| e.to_string())?;

                // Guard against selling more shares than held
                if tx_type == "SELL" && shares > current_shares {
                    return Err(format!(
                        "Cannot sell {} shares of {}: only {} shares held",
                        shares, symbol, current_shares
                    ));
                }

                let (new_shares, new_avg_cost) = if tx_type == "BUY" {
                    let total_shares = current_shares + shares;
                    let new_avg = if total_shares > 0.0 {
                        (current_shares * current_avg_cost + shares * price) / total_shares
                    } else {
                        price
                    };
                    (total_shares, new_avg)
                } else {
                    (current_shares - shares, current_avg_cost)
                };

                let now = chrono::Utc::now().to_rfc3339();
                conn.execute(
                    "UPDATE holdings SET shares = ?2, avg_cost = ?3, updated_at = ?4 WHERE id = ?1",
                    rusqlite::params![hid, new_shares, new_avg_cost, now],
                )
                .map_err(|e| e.to_string())?;

                let tx_id = uuid::Uuid::new_v4().to_string();
                let total_amount = shares * price;
                conn.execute(
                    "INSERT INTO transactions (id, holding_id, account_id, symbol, name, market, transaction_type, shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                     VALUES (?1, ?2, ?3, ?4, ?4, 'US', ?5, ?6, ?7, ?8, 0, 'USD', ?9, NULL, ?9)",
                    rusqlite::params![tx_id, hid, acct_id, symbol, tx_type, shares, price, total_amount, now],
                )
                .map_err(|e| e.to_string())?;

                Ok((new_shares, new_avg_cost))
            } else {
                Err("Holding not found".to_string())
            }
        })();

        match &result {
            Ok(_) => conn.execute_batch("COMMIT").map_err(|e| e.to_string())?,
            Err(_) => {
                let _ = conn.execute_batch("ROLLBACK");
            }
        }
        result
    }

    #[test]
    fn test_buy_updates_avg_cost_correctly() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        // Start with 100 shares at $10
        let (acct_id, _) = setup_account_and_holding(&conn, "AAPL", 100.0, 10.0);

        // Buy 100 more shares at $20
        let (new_shares, new_avg) =
            simulate_transaction(&conn, &acct_id, "AAPL", "BUY", 100.0, 20.0).unwrap();

        assert!((new_shares - 200.0).abs() < 1e-9);
        // Weighted avg: (100*10 + 100*20) / 200 = 15.0
        assert!((new_avg - 15.0).abs() < 1e-9);
    }

    #[test]
    fn test_multiple_buys_avg_cost() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        // Start with 50 shares at $100
        let (acct_id, _) = setup_account_and_holding(&conn, "MSFT", 50.0, 100.0);

        // Buy 30 at $120
        let (shares, avg) =
            simulate_transaction(&conn, &acct_id, "MSFT", "BUY", 30.0, 120.0).unwrap();
        assert!((shares - 80.0).abs() < 1e-9);
        // (50*100 + 30*120) / 80 = (5000 + 3600) / 80 = 107.5
        assert!((avg - 107.5).abs() < 1e-9);

        // Buy 20 more at $90
        let (shares2, avg2) =
            simulate_transaction(&conn, &acct_id, "MSFT", "BUY", 20.0, 90.0).unwrap();
        assert!((shares2 - 100.0).abs() < 1e-9);
        // (80*107.5 + 20*90) / 100 = (8600 + 1800) / 100 = 104.0
        assert!((avg2 - 104.0).abs() < 1e-9);
    }

    #[test]
    fn test_sell_preserves_avg_cost() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let (acct_id, _) = setup_account_and_holding(&conn, "GOOG", 100.0, 150.0);

        // Sell 30 shares — avg_cost should remain 150
        let (new_shares, new_avg) =
            simulate_transaction(&conn, &acct_id, "GOOG", "SELL", 30.0, 200.0).unwrap();
        assert!((new_shares - 70.0).abs() < 1e-9);
        assert!((new_avg - 150.0).abs() < 1e-9);
    }

    #[test]
    fn test_sell_all_shares() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let (acct_id, _) = setup_account_and_holding(&conn, "TSLA", 50.0, 200.0);

        // Sell exactly all shares
        let (new_shares, new_avg) =
            simulate_transaction(&conn, &acct_id, "TSLA", "SELL", 50.0, 250.0).unwrap();
        assert!((new_shares - 0.0).abs() < 1e-9);
        assert!((new_avg - 200.0).abs() < 1e-9);
    }

    #[test]
    fn test_sell_more_than_held_is_rejected() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let (acct_id, _) = setup_account_and_holding(&conn, "NVDA", 100.0, 50.0);

        // Try to sell 150 shares when only 100 held
        let result = simulate_transaction(&conn, &acct_id, "NVDA", "SELL", 150.0, 60.0);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot sell 150 shares"));

        // Verify holding is unchanged (rollback worked)
        let (shares, avg): (f64, f64) = conn
            .query_row(
                "SELECT shares, avg_cost FROM holdings WHERE account_id = ?1 AND symbol = 'NVDA'",
                rusqlite::params![acct_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert!((shares - 100.0).abs() < 1e-9);
        assert!((avg - 50.0).abs() < 1e-9);
    }

    #[test]
    fn test_transaction_atomicity_on_failure() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let (acct_id, _) = setup_account_and_holding(&conn, "AMZN", 100.0, 180.0);

        // Attempt an invalid sell
        let result = simulate_transaction(&conn, &acct_id, "AMZN", "SELL", 200.0, 190.0);
        assert!(result.is_err());

        // Verify no transaction was recorded
        let tx_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE account_id = ?1 AND symbol = 'AMZN'",
                rusqlite::params![acct_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tx_count, 0);

        // Verify holding unchanged
        let (shares,): (f64,) = conn
            .query_row(
                "SELECT shares FROM holdings WHERE account_id = ?1 AND symbol = 'AMZN'",
                rusqlite::params![acct_id],
                |row| Ok((row.get(0)?,)),
            )
            .unwrap();
        assert!((shares - 100.0).abs() < 1e-9);
    }

    #[test]
    fn test_buy_then_sell_sequence() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let (acct_id, _) = setup_account_and_holding(&conn, "META", 0.0, 0.0);

        // Buy 100 at $300
        let (s1, a1) = simulate_transaction(&conn, &acct_id, "META", "BUY", 100.0, 300.0).unwrap();
        assert!((s1 - 100.0).abs() < 1e-9);
        assert!((a1 - 300.0).abs() < 1e-9);

        // Buy 50 at $350
        let (s2, a2) = simulate_transaction(&conn, &acct_id, "META", "BUY", 50.0, 350.0).unwrap();
        assert!((s2 - 150.0).abs() < 1e-9);
        // (100*300 + 50*350) / 150 = 47500/150 ≈ 316.67
        assert!((a2 - 316.666_666_667).abs() < 0.001);

        // Sell 80 at $400 — avg_cost stays at ~316.67
        let (s3, a3) = simulate_transaction(&conn, &acct_id, "META", "SELL", 80.0, 400.0).unwrap();
        assert!((s3 - 70.0).abs() < 1e-9);
        assert!((a3 - 316.666_666_667).abs() < 0.001);

        // Verify 3 transactions recorded
        let tx_count: i32 = conn
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE account_id = ?1 AND symbol = 'META'",
                rusqlite::params![acct_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tx_count, 3);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Cash auto-update tests
    // ─────────────────────────────────────────────────────────────────────────

    /// Helper to read the cash holding's `shares` for the given account + currency.
    fn get_cash_balance(conn: &rusqlite::Connection, acct_id: &str, currency: &str) -> Option<f64> {
        let cash_symbol = format!("$CASH-{}", currency);
        conn.query_row(
            "SELECT shares FROM holdings WHERE account_id = ?1 AND symbol = ?2",
            rusqlite::params![acct_id, cash_symbol],
            |row| row.get(0),
        )
        .ok()
    }

    #[test]
    fn test_cash_delta_buy() {
        use crate::commands::transactions::cash_delta;
        // BUY: cash decreases by total_amount + commission
        let delta = cash_delta("BUY", "AAPL", 1000.0, 5.0);
        assert!((delta - (-1005.0)).abs() < 1e-9);
    }

    #[test]
    fn test_cash_delta_sell() {
        use crate::commands::transactions::cash_delta;
        // SELL: cash increases by total_amount - commission
        let delta = cash_delta("SELL", "AAPL", 2000.0, 10.0);
        assert!((delta - 1990.0).abs() < 1e-9);
    }

    #[test]
    fn test_adjust_cash_creates_holding_when_missing() {
        use crate::commands::transactions::adjust_cash_holding;
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let acct_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at) VALUES (?1, 'Test', 'US', ?2, ?2)",
            rusqlite::params![acct_id, now],
        ).unwrap();

        // No cash holding exists yet
        assert!(get_cash_balance(&conn, &acct_id, "USD").is_none());

        // Adjust cash by +5000
        adjust_cash_holding(&conn, &acct_id, "USD", "US", 5000.0).unwrap();

        let balance = get_cash_balance(&conn, &acct_id, "USD").unwrap();
        assert!((balance - 5000.0).abs() < 1e-9);

        // Verify fields
        let (name, market, avg_cost, currency): (String, String, f64, String) = conn
            .query_row(
                "SELECT name, market, avg_cost, currency FROM holdings WHERE account_id = ?1 AND symbol = '$CASH-USD'",
                rusqlite::params![acct_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(name, "现金 (USD)");
        assert_eq!(market, "US");
        assert!((avg_cost - 1.0).abs() < 1e-9);
        assert_eq!(currency, "USD");
    }

    #[test]
    fn test_adjust_cash_updates_existing_holding() {
        use crate::commands::transactions::adjust_cash_holding;
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let acct_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at) VALUES (?1, 'Test', 'US', ?2, ?2)",
            rusqlite::params![acct_id, now],
        ).unwrap();
        // Pre-create a cash holding with 10000
        let cash_id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO holdings (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
             VALUES (?1, ?2, '$CASH-USD', '现金 (USD)', 'US', 10000.0, 1.0, 'USD', ?3, ?3)",
            rusqlite::params![cash_id, acct_id, now],
        ).unwrap();

        // Deduct 3000
        adjust_cash_holding(&conn, &acct_id, "USD", "US", -3000.0).unwrap();
        let balance = get_cash_balance(&conn, &acct_id, "USD").unwrap();
        assert!((balance - 7000.0).abs() < 1e-9);

        // Add 500
        adjust_cash_holding(&conn, &acct_id, "USD", "US", 500.0).unwrap();
        let balance2 = get_cash_balance(&conn, &acct_id, "USD").unwrap();
        assert!((balance2 - 7500.0).abs() < 1e-9);
    }

    #[test]
    fn test_buy_transaction_decreases_cash() {
        use crate::commands::transactions::{adjust_cash_holding, cash_delta};
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let (acct_id, _) = setup_account_and_holding(&conn, "AAPL", 0.0, 0.0);

        // Seed cash: 50000 USD
        adjust_cash_holding(&conn, &acct_id, "USD", "US", 50000.0).unwrap();

        // Simulate BUY: 100 shares at $150, commission $10
        let total_amount = 15000.0;
        let commission = 10.0;
        let delta = cash_delta("BUY", "AAPL", total_amount, commission);
        adjust_cash_holding(&conn, &acct_id, "USD", "US", delta).unwrap();

        // Cash should be 50000 - 15010 = 34990
        let balance = get_cash_balance(&conn, &acct_id, "USD").unwrap();
        assert!((balance - 34990.0).abs() < 1e-9);
    }

    #[test]
    fn test_sell_transaction_increases_cash() {
        use crate::commands::transactions::{adjust_cash_holding, cash_delta};
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let (acct_id, _) = setup_account_and_holding(&conn, "AAPL", 100.0, 150.0);

        // Seed cash: 10000 USD
        adjust_cash_holding(&conn, &acct_id, "USD", "US", 10000.0).unwrap();

        // Simulate SELL: 50 shares at $200, commission $8
        let total_amount = 10000.0;
        let commission = 8.0;
        let delta = cash_delta("SELL", "AAPL", total_amount, commission);
        adjust_cash_holding(&conn, &acct_id, "USD", "US", delta).unwrap();

        // Cash should be 10000 + (10000 - 8) = 19992
        let balance = get_cash_balance(&conn, &acct_id, "USD").unwrap();
        assert!((balance - 19992.0).abs() < 1e-9);
    }

    #[test]
    fn test_cash_auto_created_on_first_buy() {
        use crate::commands::transactions::{adjust_cash_holding, cash_delta};
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let (acct_id, _) = setup_account_and_holding(&conn, "TSLA", 0.0, 0.0);

        // No cash holding exists
        assert!(get_cash_balance(&conn, &acct_id, "USD").is_none());

        // BUY creates cash holding with negative balance
        let delta = cash_delta("BUY", "TSLA", 5000.0, 5.0);
        adjust_cash_holding(&conn, &acct_id, "USD", "US", delta).unwrap();

        let balance = get_cash_balance(&conn, &acct_id, "USD").unwrap();
        assert!((balance - (-5005.0)).abs() < 1e-9);
    }

    #[test]
    fn test_cny_cash_holding() {
        use crate::commands::transactions::adjust_cash_holding;
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let acct_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at) VALUES (?1, 'CN Account', 'CN', ?2, ?2)",
            rusqlite::params![acct_id, now],
        ).unwrap();

        adjust_cash_holding(&conn, &acct_id, "CNY", "CN", 100000.0).unwrap();

        let balance = get_cash_balance(&conn, &acct_id, "CNY").unwrap();
        assert!((balance - 100000.0).abs() < 1e-9);

        // Verify symbol and name
        let (symbol, name): (String, String) = conn
            .query_row(
                "SELECT symbol, name FROM holdings WHERE account_id = ?1 AND symbol LIKE '$CASH-%'",
                rusqlite::params![acct_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(symbol, "$CASH-CNY");
        assert_eq!(name, "现金 (CNY)");
    }

    #[test]
    fn test_buy_sell_sequence_updates_cash() {
        use crate::commands::transactions::{adjust_cash_holding, cash_delta};
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let (acct_id, _) = setup_account_and_holding(&conn, "GOOG", 0.0, 0.0);

        // Start with 100000 cash
        adjust_cash_holding(&conn, &acct_id, "USD", "US", 100000.0).unwrap();

        // BUY: 50 shares at $100, commission $5 → cash -= 5005
        let d1 = cash_delta("BUY", "GOOG", 5000.0, 5.0);
        adjust_cash_holding(&conn, &acct_id, "USD", "US", d1).unwrap();
        let b1 = get_cash_balance(&conn, &acct_id, "USD").unwrap();
        assert!((b1 - 94995.0).abs() < 1e-9);

        // BUY: 30 shares at $120, commission $3 → cash -= 3603
        let d2 = cash_delta("BUY", "GOOG", 3600.0, 3.0);
        adjust_cash_holding(&conn, &acct_id, "USD", "US", d2).unwrap();
        let b2 = get_cash_balance(&conn, &acct_id, "USD").unwrap();
        assert!((b2 - 91392.0).abs() < 1e-9);

        // SELL: 20 shares at $150, commission $4 → cash += 2996
        let d3 = cash_delta("SELL", "GOOG", 3000.0, 4.0);
        adjust_cash_holding(&conn, &acct_id, "USD", "US", d3).unwrap();
        let b3 = get_cash_balance(&conn, &acct_id, "USD").unwrap();
        assert!((b3 - 94388.0).abs() < 1e-9);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Net-cost SELL formula tests (adjust = true, commission included)
    // ─────────────────────────────────────────────────────────────────────────

    /// Apply the SELL net-cost formula directly:
    ///   remaining_cost = (shares × avg_cost) - (total_amount - commission)
    ///   new_avg = remaining_cost / remaining_shares
    ///
    /// `total_amount` is gross proceeds (shares_sold × price). Adding `commission`
    /// back effectively subtracts only net proceeds from the cost basis, because
    /// the commission paid is a trading cost that the seller does not receive.
    fn apply_sell_net_cost(
        shares: f64,
        avg_cost: f64,
        sold: f64,
        total_amount: f64,
        commission: f64,
    ) -> f64 {
        let remaining = shares - sold;
        if remaining > 0.0 {
            (shares * avg_cost - total_amount + commission) / remaining
        } else {
            0.0
        }
    }

    /// Reverse the SELL net-cost adjustment:
    ///   Adds back the net proceeds (total_amount - commission) that were previously
    ///   subtracted from the cost basis, and restores the pre-SELL share count.
    ///   rev_avg = (cur_total_cost + net_proceeds) / (cur_shares + sold_shares)
    ///           = (cur_shares × cur_avg_cost + total_amount - commission) / new_shares
    fn reverse_sell_net_cost(
        cur_shares: f64,
        cur_avg_cost: f64,
        sold: f64,
        total_amount: f64,
        commission: f64,
    ) -> f64 {
        let new_shares = cur_shares + sold;
        if new_shares > 0.0 {
            (cur_shares * cur_avg_cost + total_amount - commission) / new_shares
        } else {
            0.0
        }
    }

    #[test]
    fn test_sell_net_cost_reduces_by_net_proceeds() {
        // BUY 100 shares at ¥10 with ¥5 commission → avg_cost = (1000+5)/100 = 10.05
        let shares = 100.0_f64;
        let avg_cost = (100.0 * 10.0 + 5.0) / 100.0; // 10.05

        // SELL 40 shares at ¥15, commission ¥6
        let sold = 40.0_f64;
        let total_amount = sold * 15.0; // 600
        let commission = 6.0_f64;

        // Net proceeds = 600 - 6 = 594
        // Remaining total cost = 100*10.05 - 594 = 1005 - 594 = 411
        // new_avg = 411 / 60 = 6.85
        let expected = (shares * avg_cost - total_amount + commission) / (shares - sold);
        let got = apply_sell_net_cost(shares, avg_cost, sold, total_amount, commission);
        assert!((got - expected).abs() < 1e-9);
        // Verify numerical value
        assert!((got - 6.85).abs() < 1e-9);
    }

    #[test]
    fn test_sell_net_cost_zero_commission() {
        // When commission == 0, formula reduces to (shares * avg_cost - total_amount) / remaining,
        // which was the old formula. Verify backward compatibility.
        let shares = 100.0_f64;
        let avg_cost = 10.0_f64;
        let sold = 40.0_f64;
        let total_amount = sold * 15.0; // 600
        let commission = 0.0_f64;

        let got = apply_sell_net_cost(shares, avg_cost, sold, total_amount, commission);
        // (1000 - 600) / 60 ≈ 6.6667
        let expected = (shares * avg_cost - total_amount) / (shares - sold);
        assert!((got - expected).abs() < 1e-9);
    }

    #[test]
    fn test_sell_net_cost_all_shares_zeroes_avg_cost() {
        // Selling all shares should yield avg_cost = 0.0
        let shares = 50.0_f64;
        let avg_cost = 20.0_f64;
        let sold = 50.0_f64;
        let total_amount = sold * 25.0;
        let commission = 10.0_f64;

        let got = apply_sell_net_cost(shares, avg_cost, sold, total_amount, commission);
        assert!((got - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_reverse_sell_restores_original_avg_cost() {
        // Start: 100 shares at avg_cost 10.05
        let shares = 100.0_f64;
        let avg_cost = 10.05_f64;

        // Apply SELL: sell 40 at ¥15, commission ¥6
        let sold = 40.0_f64;
        let total_amount = sold * 15.0; // 600
        let commission = 6.0_f64;
        let after_avg = apply_sell_net_cost(shares, avg_cost, sold, total_amount, commission);
        let after_shares = shares - sold; // 60

        // Reverse the SELL — must recover original avg_cost
        let recovered_avg =
            reverse_sell_net_cost(after_shares, after_avg, sold, total_amount, commission);
        assert!(
            (recovered_avg - avg_cost).abs() < 1e-9,
            "recovered={recovered_avg}, expected={avg_cost}"
        );
    }

    #[test]
    fn test_reverse_sell_round_trips_with_various_commissions() {
        for &commission in &[0.0_f64, 5.0, 10.0, 25.5] {
            let shares = 200.0_f64;
            let avg_cost = 50.0_f64;
            let sold = 80.0_f64;
            let total_amount = sold * 60.0; // 4800

            let after_avg = apply_sell_net_cost(shares, avg_cost, sold, total_amount, commission);
            let after_shares = shares - sold;
            let recovered =
                reverse_sell_net_cost(after_shares, after_avg, sold, total_amount, commission);
            assert!(
                (recovered - avg_cost).abs() < 1e-9,
                "commission={commission}: recovered={recovered}, expected={avg_cost}"
            );
        }
    }

    #[test]
    fn test_recalculate_sell_net_cost_inline() {
        // Simulate the recalculate_holdings_cost SELL branch inline.
        // BUY 100 shares at ¥10, commission ¥5 → total cost = 1005, avg_cost = 10.05
        let mut shares = 0.0_f64;
        let mut avg_cost = 0.0_f64;

        // BUY
        let buy_shares = 100.0_f64;
        let buy_price = 10.0_f64;
        let buy_commission = 5.0_f64;
        let new_total = shares + buy_shares;
        avg_cost = (shares * avg_cost + buy_shares * buy_price + buy_commission) / new_total;
        shares = new_total;
        assert!((avg_cost - 10.05).abs() < 1e-9);

        // SELL 40 shares at ¥15, commission ¥6  (adjust = true)
        let sell_shares = 40.0_f64;
        let total_amount = sell_shares * 15.0_f64; // 600
        let sell_commission = 6.0_f64;
        let remaining = shares - sell_shares;
        avg_cost = (shares * avg_cost - total_amount + sell_commission) / remaining;
        shares = remaining;

        // Remaining total cost = 1005 - (600 - 6) = 1005 - 594 = 411
        // avg_cost = 411 / 60 = 6.85
        assert!((shares - 60.0).abs() < 1e-9);
        assert!((avg_cost - 6.85).abs() < 1e-9, "avg_cost={avg_cost}");
    }

    #[test]
    fn fresh_and_reopened_databases_use_the_current_schema_version() {
        let mut conn = Connection::open_in_memory().unwrap();

        run_migrations(&mut conn).unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let changes_after_first_run = conn.total_changes();
        run_migrations(&mut conn).unwrap();

        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        assert_eq!(conn.total_changes(), changes_after_first_run);
    }

    #[test]
    fn v2_database_migrates_cached_quote_metadata_to_v3() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE cached_quotes (
               symbol TEXT PRIMARY KEY NOT NULL,
               name TEXT NOT NULL,
               market TEXT NOT NULL,
               current_price REAL NOT NULL DEFAULT 0,
               previous_close REAL NOT NULL DEFAULT 0,
               change REAL NOT NULL DEFAULT 0,
               change_percent REAL NOT NULL DEFAULT 0,
               high REAL NOT NULL DEFAULT 0,
               low REAL NOT NULL DEFAULT 0,
               volume INTEGER NOT NULL DEFAULT 0,
               updated_at TEXT NOT NULL
             );
             INSERT INTO cached_quotes VALUES
               ('AAPL', 'Apple Inc.', 'US', 175.5, 174.0, 1.5, 0.86, 176.0, 173.0, 50000000, 'old');
             PRAGMA user_version = 2;",
        )
        .unwrap();

        run_migrations(&mut conn).unwrap();

        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        for column in [
            "pe_ttm",
            "pb",
            "market_cap",
            "dividend_yield",
            "eps",
            "roe",
            "turnover_rate",
        ] {
            assert!(column_exists(&conn, "cached_quotes", column).unwrap());
        }
        let existing_name: String = conn
            .query_row(
                "SELECT name FROM cached_quotes WHERE symbol = 'AAPL'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(existing_name, "Apple Inc.");
    }

    #[test]
    fn v4_database_migrates_cached_quote_key_to_market_and_symbol() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE cached_quotes (
               symbol TEXT PRIMARY KEY NOT NULL,
               name TEXT NOT NULL,
               market TEXT NOT NULL,
               current_price REAL NOT NULL DEFAULT 0,
               previous_close REAL NOT NULL DEFAULT 0,
               change REAL NOT NULL DEFAULT 0,
               change_percent REAL NOT NULL DEFAULT 0,
               high REAL NOT NULL DEFAULT 0,
               low REAL NOT NULL DEFAULT 0,
               volume INTEGER NOT NULL DEFAULT 0,
               updated_at TEXT NOT NULL,
               pe_ttm REAL,
               pb REAL,
               market_cap REAL,
               dividend_yield REAL,
               eps REAL,
               roe REAL,
               turnover_rate REAL
             );
             INSERT INTO cached_quotes (symbol, name, market, current_price, updated_at)
             VALUES ('BABA', 'Alibaba US', 'US', 120.0, 'old');
             PRAGMA user_version = 4;",
        )
        .unwrap();

        run_migrations(&mut conn).unwrap();
        conn.execute(
            "INSERT INTO cached_quotes (symbol, name, market, current_price, updated_at)
             VALUES ('BABA', 'Alibaba HK', 'HK', 90.0, 'new')",
            [],
        )
        .unwrap();

        let rows: Vec<(String, String, f64)> = conn
            .prepare(
                "SELECT market, name, current_price FROM cached_quotes
                 WHERE symbol = 'BABA' ORDER BY market",
            )
            .unwrap()
            .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(
            rows,
            vec![
                ("HK".to_string(), "Alibaba HK".to_string(), 90.0),
                ("US".to_string(), "Alibaba US".to_string(), 120.0),
            ]
        );
        let primary_key_columns: Vec<String> = conn
            .prepare(
                "SELECT name FROM pragma_table_info('cached_quotes')
                 WHERE pk > 0 ORDER BY pk",
            )
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(primary_key_columns, vec!["market", "symbol"]);
    }

    #[test]
    fn unversioned_legacy_database_adds_columns_repairs_open_rows_and_keeps_data() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (
               id TEXT PRIMARY KEY, name TEXT NOT NULL,
               market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
               description TEXT, created_at TEXT NOT NULL, updated_at TEXT NOT NULL
             );
             CREATE TABLE categories (
               id TEXT PRIMARY KEY, name TEXT NOT NULL, color TEXT NOT NULL, icon TEXT NOT NULL,
               is_system INTEGER NOT NULL DEFAULT 0, sort_order INTEGER NOT NULL DEFAULT 0,
               created_at TEXT NOT NULL
             );
             CREATE TABLE holdings (
               id TEXT PRIMARY KEY,
               account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
               symbol TEXT NOT NULL, name TEXT NOT NULL,
               market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
               category_id TEXT REFERENCES categories(id) ON DELETE SET NULL,
               shares REAL NOT NULL, avg_cost REAL NOT NULL,
               currency TEXT NOT NULL CHECK(currency IN ('USD', 'CNY', 'HKD')),
               created_at TEXT NOT NULL, updated_at TEXT NOT NULL
             );
             CREATE TABLE transactions (
               id TEXT PRIMARY KEY,
               holding_id TEXT REFERENCES holdings(id) ON DELETE SET NULL,
               account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
               symbol TEXT NOT NULL, name TEXT NOT NULL,
               market TEXT NOT NULL CHECK(market IN ('US', 'CN', 'HK')),
               transaction_type TEXT NOT NULL CHECK(transaction_type IN ('BUY', 'SELL', 'PAY')),
               shares REAL NOT NULL, price REAL NOT NULL, total_amount REAL NOT NULL,
               commission REAL NOT NULL DEFAULT 0,
               currency TEXT NOT NULL CHECK(currency IN ('USD', 'CNY', 'HKD')),
               traded_at TEXT NOT NULL, notes TEXT, created_at TEXT NOT NULL
             );
             CREATE TABLE quarterly_snapshots (
               id TEXT PRIMARY KEY, quarter TEXT NOT NULL UNIQUE, snapshot_date TEXT NOT NULL,
               total_value REAL NOT NULL, total_cost REAL NOT NULL, total_pnl REAL NOT NULL,
               us_value REAL NOT NULL, us_cost REAL NOT NULL, cn_value REAL NOT NULL,
               cn_cost REAL NOT NULL, hk_value REAL NOT NULL, hk_cost REAL NOT NULL,
               exchange_rates TEXT NOT NULL, overall_notes TEXT, created_at TEXT NOT NULL
             );
             CREATE TABLE quarterly_holding_snapshots (
               id TEXT PRIMARY KEY,
               quarterly_snapshot_id TEXT NOT NULL REFERENCES quarterly_snapshots(id) ON DELETE CASCADE,
               account_id TEXT NOT NULL,
               account_name TEXT NOT NULL, symbol TEXT NOT NULL, name TEXT NOT NULL,
               market TEXT NOT NULL, category_name TEXT NOT NULL, category_color TEXT NOT NULL,
               shares REAL NOT NULL, avg_cost REAL NOT NULL, close_price REAL NOT NULL,
               market_value REAL NOT NULL, cost_value REAL NOT NULL, pnl REAL NOT NULL,
               pnl_percent REAL NOT NULL, weight REAL NOT NULL, notes TEXT
             );
             CREATE TABLE quote_provider_config (
               id INTEGER PRIMARY KEY, us_provider TEXT NOT NULL, hk_provider TEXT NOT NULL,
               cn_provider TEXT NOT NULL, updated_at TEXT NOT NULL
             );
             CREATE TABLE ai_config (
               id INTEGER PRIMARY KEY, provider TEXT NOT NULL, api_key TEXT NOT NULL,
               model TEXT NOT NULL, base_url TEXT, system_prompt TEXT NOT NULL,
               updated_at TEXT NOT NULL
             );
             CREATE TABLE chat_sessions (
               id TEXT PRIMARY KEY, name TEXT NOT NULL, created_at TEXT NOT NULL,
               updated_at TEXT NOT NULL
             );
             CREATE TABLE chat_messages (
               id TEXT PRIMARY KEY,
               session_id TEXT NOT NULL REFERENCES chat_sessions(id) ON DELETE CASCADE,
               role TEXT NOT NULL,
               content TEXT NOT NULL, prompt_tokens INTEGER NOT NULL DEFAULT 0,
               completion_tokens INTEGER NOT NULL DEFAULT 0,
               total_tokens INTEGER NOT NULL DEFAULT 0, cached_tokens INTEGER NOT NULL DEFAULT 0,
               created_at TEXT NOT NULL
             );
             CREATE TABLE option_records (
               id TEXT PRIMARY KEY,
               account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
               option_symbol TEXT NOT NULL,
               underlying TEXT NOT NULL, expiry_date TEXT NOT NULL, strike_price REAL NOT NULL,
               option_type TEXT NOT NULL CHECK(option_type IN ('P', 'C')),
               action TEXT NOT NULL CHECK(action IN ('SELL', 'BUY')), code TEXT NOT NULL,
               quantity INTEGER NOT NULL, price REAL NOT NULL, amount REAL NOT NULL,
               commission REAL NOT NULL DEFAULT 0, fee REAL NOT NULL DEFAULT 0,
               traded_at TEXT, settled_at TEXT, created_at TEXT NOT NULL
             );
             INSERT INTO accounts VALUES ('account-1', 'Legacy', 'US', NULL, 'old', 'old');
             INSERT INTO holdings VALUES
               ('holding-1', 'account-1', 'AAPL', 'Apple', 'US', NULL, 1, 100, 'USD', 'old', 'old');
             INSERT INTO transactions VALUES
               ('transaction-1', 'holding-1', 'account-1', 'AAPL', 'Apple', 'US',
                'BUY', 1, 100, 100, 0, 'USD', 'old', 'backfill:initial', 'old');
             INSERT INTO quote_provider_config VALUES (1, 'yahoo', 'yahoo', 'eastmoney', 'old');
             INSERT INTO ai_config VALUES
               (1, 'openai', 'legacy-key', 'legacy-model', NULL, 'legacy prompt', 'old');",
        )
        .unwrap();

        run_migrations(&mut conn).unwrap();

        for (table, column) in [
            ("quarterly_holding_snapshots", "decision_quality"),
            ("quote_provider_config", "xueqiu_cookie"),
            ("quote_provider_config", "xueqiu_u"),
            ("quote_provider_config", "cn_adjust_sell_pay_cost"),
            ("quote_provider_config", "us_adjust_sell_pay_cost"),
            ("quote_provider_config", "hk_adjust_sell_pay_cost"),
            ("ai_config", "tools_enabled"),
            ("chat_messages", "reasoning"),
            ("chat_messages", "tool_calls"),
            ("option_records", "contract_status"),
        ] {
            assert!(
                column_exists(&conn, table, column).unwrap(),
                "{table}.{column}"
            );
        }
        let transaction_type: String = conn
            .query_row(
                "SELECT transaction_type FROM transactions WHERE id = 'transaction-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let api_key: String = conn
            .query_row("SELECT api_key FROM ai_config WHERE id = 1", [], |row| {
                row.get(0)
            })
            .unwrap();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        let required_indices: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master
                 WHERE type = 'index' AND name IN (
                   'idx_daily_holding_snapshots_date',
                   'idx_quarterly_holding_snapshots_snapshot_id',
                   'idx_chat_messages_session'
                 )",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let invalid_transaction = conn.execute(
            "INSERT INTO transactions
             (id, account_id, symbol, name, market, transaction_type, shares, price,
              total_amount, commission, currency, traded_at, created_at)
             VALUES ('invalid', 'account-1', 'AAPL', 'Apple', 'US', 'INVALID',
                     1, 1, 1, 0, 'USD', 'old', 'old')",
            [],
        );

        assert_eq!(transaction_type, "OPEN");
        assert_eq!(api_key, "legacy-key");
        assert_eq!(version, CURRENT_SCHEMA_VERSION);
        assert_eq!(required_indices, 3);
        assert!(invalid_transaction.is_err());
    }

    const PORTFOLIO_QUERY_INDEXES: [&str; 4] = [
        "idx_transactions_account_symbol_traded_at",
        "idx_transactions_account_traded_at",
        "idx_transactions_holding_id",
        "idx_option_records_account_symbol_traded_at",
    ];

    fn remove_v2_indexes(conn: &Connection) {
        for index in PORTFOLIO_QUERY_INDEXES {
            conn.execute_batch(&format!("DROP INDEX IF EXISTS {index}"))
                .unwrap();
        }
    }

    fn query_plan(conn: &Connection, query: &str) -> String {
        conn.prepare(&format!("EXPLAIN QUERY PLAN {query}"))
            .unwrap()
            .query_map([], |row| row.get::<_, String>(3))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n")
    }

    #[test]
    fn v1_database_migrates_through_current_with_idempotent_index_definitions() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("portfolio.sqlite");
        {
            let conn = Connection::open(&path).unwrap();
            schema::create_current_schema(&conn).unwrap();
            remove_v2_indexes(&conn);
            conn.pragma_update(None, "user_version", 1).unwrap();
        }

        let definitions = {
            let mut conn = Connection::open(&path).unwrap();
            run_migrations(&mut conn).unwrap();
            let version: i64 = conn
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .unwrap();
            assert_eq!(version, CURRENT_SCHEMA_VERSION);

            PORTFOLIO_QUERY_INDEXES
                .iter()
                .map(|name| {
                    conn.query_row(
                        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
                        [name],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap()
                })
                .collect::<Vec<_>>()
        };

        assert!(definitions[0].contains("account_id, UPPER(symbol), traded_at"));
        assert!(definitions[1].contains("account_id, traded_at DESC"));
        assert!(definitions[2].contains("holding_id"));
        assert!(definitions[3].contains("account_id, option_symbol, traded_at"));

        let mut reopened = Connection::open(&path).unwrap();
        run_migrations(&mut reopened).unwrap();
        let reopened_definitions = PORTFOLIO_QUERY_INDEXES
            .iter()
            .map(|name| {
                reopened
                    .query_row(
                        "SELECT sql FROM sqlite_master WHERE type = 'index' AND name = ?1",
                        [name],
                        |row| row.get::<_, String>(0),
                    )
                    .unwrap()
            })
            .collect::<Vec<_>>();
        assert_eq!(reopened_definitions, definitions);
        assert_eq!(schema_version(&reopened), CURRENT_SCHEMA_VERSION);
    }

    #[test]
    fn portfolio_history_queries_use_v2_indexes() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();

        let account_symbol = query_plan(
            &conn,
            "SELECT id FROM transactions
             WHERE account_id = 'account' AND UPPER(symbol) = UPPER('aapl')
             ORDER BY traded_at DESC",
        );
        assert!(
            account_symbol.contains("idx_transactions_account_symbol_traded_at"),
            "{account_symbol}"
        );

        let account_history = query_plan(
            &conn,
            "SELECT id FROM transactions
             WHERE account_id = 'account' ORDER BY traded_at DESC",
        );
        assert!(
            account_history.contains("idx_transactions_account_traded_at"),
            "{account_history}"
        );

        let holding_history = query_plan(
            &conn,
            "SELECT id FROM transactions WHERE holding_id = 'holding'",
        );
        assert!(
            holding_history.contains("idx_transactions_holding_id"),
            "{holding_history}"
        );

        let option_history = query_plan(
            &conn,
            "SELECT id FROM option_records
             WHERE account_id = 'account' AND option_symbol = 'AAPL 18SEP26 200 C'
             ORDER BY traded_at",
        );
        assert!(
            option_history.contains("idx_option_records_account_symbol_traded_at"),
            "{option_history}"
        );
    }

    #[test]
    fn incompatible_legacy_schema_returns_an_error_without_advancing_version() {
        let mut conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("CREATE TABLE transactions (id TEXT PRIMARY KEY);")
            .unwrap();

        let error = run_migrations(&mut conn).unwrap_err();
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();

        assert!(
            error.to_string().contains("column"),
            "unexpected error: {error}"
        );
        assert_eq!(version, 0);
    }
}

#[test]
fn migration_creates_import_batch_audit_tables() {
    let db = super::Database::new(":memory:").unwrap();
    let conn = db.conn.lock().unwrap();
    let count: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN ('import_batches', 'import_batch_rows')",
        [], |row| row.get(0),
    ).unwrap();
    assert_eq!(
        count, 2,
        "batch audit tables must be present after migration"
    );
}

#[cfg(test)]
mod snapshot_cache_migration {
    use crate::db::{migrations, schema, Database};
    use rusqlite::{types::Value, Connection};

    fn create_v6_schema(conn: &Connection) {
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        schema::create_current_schema(conn).unwrap();
        schema::create_import_batch_schema(conn).unwrap();
        conn.pragma_update(None, "user_version", 6).unwrap();
    }

    fn seed_ledger(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO accounts (id, name, market, created_at, updated_at)
               VALUES ('account', 'Portfolio', 'US', '2026-01-01', '2026-01-01');
             INSERT INTO categories (id, name, color, icon, created_at)
               VALUES ('growth', 'Growth', '#F97316', 'stock', '2026-01-01');
             INSERT INTO holdings
               (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                currency, created_at, updated_at)
               VALUES ('holding', 'account', 'AAPL', 'Apple', 'US', 'growth', 10,
                       100, 'USD', '2026-01-01', '2026-01-01');
             INSERT INTO transactions
               (id, holding_id, account_id, symbol, name, market, transaction_type,
                shares, price, total_amount, commission, currency, traded_at, notes, created_at)
               VALUES ('trade', 'holding', 'account', 'AAPL', 'Apple', 'US', 'BUY',
                       10, 100, 1000, 1, 'USD', '2026-01-01', 'original note', '2026-01-01');",
        )
        .unwrap();
    }

    fn seed_daily_cache(conn: &Connection) {
        conn.execute_batch(
            "INSERT INTO daily_portfolio_values
               (date, total_cost, total_value, us_cost, us_value, daily_pnl, cumulative_pnl)
               VALUES ('2026-01-02', 1000, 1100, 1000, 1100, 100, 100),
                      ('2026-01-03', 1000, 1200, 1000, 1200, 100, 200);
             INSERT INTO daily_holding_snapshots
               (date, account_id, symbol, market, category_name, shares, avg_cost,
                close_price, market_value)
               VALUES ('2026-01-02', 'account', 'AAPL', 'US', 'Growth', 10, 100, 110, 1100),
                      ('2026-01-03', 'account', 'AAPL', 'US', 'Growth', 10, 100, 120, 1200);",
        )
        .unwrap();
    }

    fn rows(conn: &Connection, table: &str) -> Vec<Vec<Value>> {
        let mut statement = conn
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .unwrap();
        let columns = statement.column_count();
        statement
            .query_map([], |row| {
                (0..columns).map(|column| row.get(column)).collect()
            })
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap()
    }

    fn revision(conn: &Connection) -> i64 {
        conn.query_row(
            "SELECT revision FROM snapshot_cache_state WHERE id = 1",
            [],
            |row| row.get(0),
        )
        .unwrap()
    }

    #[test]
    fn v7_upgrade_clears_only_daily_caches_and_preserves_ledger_imports_and_reviews() {
        let mut conn = Connection::open_in_memory().unwrap();
        create_v6_schema(&conn);
        seed_ledger(&conn);
        seed_daily_cache(&conn);
        conn.execute_batch(
            r#"INSERT INTO option_records
               (id, account_id, option_symbol, underlying, expiry_date, strike_price,
                option_type, action, code, quantity, price, amount, created_at)
               VALUES ('option', 'account', 'AAPL 18SEP26 200 C', 'AAPL', '2026-09-18',
                       200, 'C', 'SELL', 'OPEN', 1, 5, 500, '2026-01-01');
             INSERT INTO import_batches
               (id, request_id, account_id, source, file_name, source_content, parser_version,
                kind, status, created_at, before_state, after_state, expected_balances, request_json)
               VALUES ('batch', 'request', 'account', 'csv', 'trades.csv', 'original csv content',
                       'v1', 'transactions', 'applied', '2026-01-01', '{"before":1}',
                       '{"after":2}', '[{"symbol":"AAPL","shares":10}]', '{"mode":"append"}');
             INSERT INTO import_batch_rows
               (batch_id, row_key, ordinal, raw, external_id, data, fingerprint, status, record_id)
               VALUES ('batch', 'row-1', 1, 'original raw row', 'broker-trade-1', '{"shares":10}',
                       'fingerprint-1', 'imported', 'trade');
             INSERT INTO quarterly_snapshots
               (id, quarter, snapshot_date, total_value, total_cost, total_pnl,
                overall_notes, created_at)
               VALUES ('review', '2026Q1', '2026-03-31', 1300, 1000, 300,
                       'Keep this quarterly review', '2026-03-31');
             INSERT INTO quarterly_holding_snapshots
               (id, quarterly_snapshot_id, account_id, account_name, symbol, name, market,
                category_name, shares, avg_cost, close_price, market_value, cost_value,
                pnl, pnl_percent, weight, notes, decision_quality)
               VALUES ('review-holding', 'review', 'account', 'Portfolio', 'AAPL', 'Apple',
                       'US', 'Growth', 10, 100, 130, 1300, 1000, 300, 30, 100,
                       'Keep this investment rationale', 'good');
             INSERT INTO stock_daily_prices (symbol, market, date, close, source, updated_at)
               VALUES ('AAPL', 'US', '2026-01-02', 110, 'test', '2026-01-02');"#,
        )
        .unwrap();
        let preserved_tables = [
            "accounts",
            "categories",
            "holdings",
            "transactions",
            "option_records",
            "import_batches",
            "import_batch_rows",
            "quarterly_snapshots",
            "quarterly_holding_snapshots",
            "stock_daily_prices",
        ];
        let before: Vec<_> = preserved_tables
            .iter()
            .map(|table| rows(&conn, table))
            .collect();
        assert!(before.iter().all(|table| !table.is_empty()));
        for table in ["daily_portfolio_values", "daily_holding_snapshots"] {
            assert_eq!(rows(&conn, table).len(), 2);
        }

        migrations::run_migrations(&mut conn).unwrap();

        assert_eq!(
            conn.pragma_query_value::<i64, _>(None, "user_version", |row| row.get(0))
                .unwrap(),
            migrations::CURRENT_SCHEMA_VERSION
        );
        for (table, expected) in preserved_tables.into_iter().zip(before) {
            assert_eq!(rows(&conn, table), expected, "migration changed {table}");
        }
        for table in ["daily_portfolio_values", "daily_holding_snapshots"] {
            assert!(rows(&conn, table).is_empty(), "stale {table} survived");
        }
        assert_eq!(revision(&conn), 0);
        assert_eq!(rows(&conn, "snapshot_cache_state").len(), 1);
        assert!(conn
            .execute("INSERT INTO snapshot_cache_state (id) VALUES (2)", [])
            .is_err());
    }

    #[test]
    fn v7_rerun_and_reopen_preserve_rebuilt_caches_and_revision() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("snapshot-migration.sqlite");
        let expected;
        {
            let mut conn = Connection::open(&path).unwrap();
            create_v6_schema(&conn);
            seed_ledger(&conn);
            seed_daily_cache(&conn);
            migrations::run_migrations(&mut conn).unwrap();
            conn.execute("UPDATE holdings SET shares = 12 WHERE id = 'holding'", [])
                .unwrap();
            seed_daily_cache(&conn);
            expected = (
                rows(&conn, "daily_portfolio_values"),
                rows(&conn, "daily_holding_snapshots"),
                revision(&conn),
            );
            assert_eq!(expected.2, 1);

            migrations::run_migrations(&mut conn).unwrap();

            assert_eq!(rows(&conn, "daily_portfolio_values"), expected.0);
            assert_eq!(rows(&conn, "daily_holding_snapshots"), expected.1);
            assert_eq!(revision(&conn), expected.2);
        }

        let reopened = Database::new(path.to_str().unwrap()).unwrap();
        let conn = reopened.conn.lock().unwrap();
        assert_eq!(rows(&conn, "daily_portfolio_values"), expected.0);
        assert_eq!(rows(&conn, "daily_holding_snapshots"), expected.1);
        assert_eq!(revision(&conn), expected.2);
    }

    #[test]
    fn snapshot_revision_tracks_ledger_insert_update_delete_and_category_changes() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn.lock().unwrap();
        assert_eq!(revision(&conn), 0);
        seed_ledger(&conn);
        assert_eq!(revision(&conn), 2);

        let updates = [
            "UPDATE transactions SET shares = 11 WHERE id = 'trade'",
            "UPDATE transactions SET price = 101 WHERE id = 'trade'",
            "UPDATE transactions SET total_amount = 1111 WHERE id = 'trade'",
            "UPDATE transactions SET commission = 2 WHERE id = 'trade'",
            "UPDATE transactions SET transaction_type = 'OPEN' WHERE id = 'trade'",
            "UPDATE transactions SET traded_at = '2026-01-02' WHERE id = 'trade'",
            "UPDATE transactions SET symbol = 'MSFT' WHERE id = 'trade'",
            "UPDATE transactions SET market = 'HK' WHERE id = 'trade'",
            "UPDATE transactions SET currency = 'HKD' WHERE id = 'trade'",
            "UPDATE holdings SET shares = 11 WHERE id = 'holding'",
            "UPDATE holdings SET avg_cost = 101 WHERE id = 'holding'",
            "UPDATE holdings SET category_id = NULL WHERE id = 'holding'",
            "UPDATE holdings SET category_id = 'growth' WHERE id = 'holding'",
            "UPDATE holdings SET created_at = '2026-01-02' WHERE id = 'holding'",
            "UPDATE holdings SET symbol = 'MSFT' WHERE id = 'holding'",
            "UPDATE holdings SET market = 'HK' WHERE id = 'holding'",
            "UPDATE holdings SET currency = 'HKD' WHERE id = 'holding'",
            "DELETE FROM transactions WHERE id = 'trade'",
            "DELETE FROM holdings WHERE id = 'holding'",
        ];
        for query in updates {
            let before = revision(&conn);
            assert_eq!(conn.execute(query, []).unwrap(), 1, "{query}");
            assert_eq!(revision(&conn), before + 1, "{query}");
        }
    }

    #[test]
    fn snapshot_revision_ignores_metadata_only_and_noop_ledger_updates() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn.lock().unwrap();
        seed_ledger(&conn);
        let before = revision(&conn);
        conn.execute_batch(
            "UPDATE transactions SET name = 'Renamed Apple', notes = 'Updated rationale',
               holding_id = NULL, created_at = '2026-02-01' WHERE id = 'trade';
             UPDATE holdings SET name = 'Renamed Apple', updated_at = '2026-02-01'
               WHERE id = 'holding';
             UPDATE transactions SET shares = shares, price = price, traded_at = traded_at;
             UPDATE holdings SET shares = shares, avg_cost = avg_cost, category_id = category_id;",
        )
        .unwrap();

        assert_eq!(revision(&conn), before);
    }

    #[test]
    fn snapshot_revision_rolls_back_with_ledger_mutations() {
        let db = Database::new(":memory:").unwrap();
        let mut conn = db.conn.lock().unwrap();
        seed_ledger(&conn);
        let before = (
            revision(&conn),
            rows(&conn, "transactions"),
            rows(&conn, "holdings"),
        );
        let tx = conn.transaction().unwrap();
        tx.execute_batch(
            "UPDATE holdings SET shares = 20 WHERE id = 'holding';
             UPDATE transactions SET traded_at = '2026-02-01' WHERE id = 'trade';
             DELETE FROM transactions WHERE id = 'trade';
             INSERT INTO holdings
               (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
               VALUES ('new-holding', 'account', 'MSFT', 'Microsoft', 'US', 5, 200,
                       'USD', '2026-02-01', '2026-02-01');",
        )
        .unwrap();
        assert_eq!(revision(&tx), before.0 + 4);

        tx.rollback().unwrap();

        assert_eq!(revision(&conn), before.0);
        assert_eq!(rows(&conn, "transactions"), before.1);
        assert_eq!(rows(&conn, "holdings"), before.2);
    }
}
