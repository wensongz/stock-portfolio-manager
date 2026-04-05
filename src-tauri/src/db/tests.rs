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
            xueqiu_cookie: None,
            xueqiu_u: None,
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
            xueqiu_cookie: None,
            xueqiu_u: None,
        };
        let result = crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
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
        };
        let result = crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
        assert!(result.is_ok());

        let loaded = crate::services::quote_provider_service::get_quote_provider_config(&db).unwrap();
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
        };
        let result = crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
        assert!(result.is_ok());

        let loaded = crate::services::quote_provider_service::get_quote_provider_config(&db).unwrap();
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
        };
        let result = crate::services::quote_provider_service::update_quote_provider_config(&db, &config);
        assert!(result.is_ok());

        let loaded = crate::services::quote_provider_service::get_quote_provider_config(&db).unwrap();
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

    // ─────────────────────────────────────────────────────────────────────────
    // Transaction cost-basis and data integrity tests
    // ─────────────────────────────────────────────────────────────────────────

    /// Helper: create an account and a holding, returning (account_id, holding_id).
    fn setup_account_and_holding(conn: &rusqlite::Connection, symbol: &str, shares: f64, avg_cost: f64) -> (String, String) {
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
        conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;

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
            Err(_) => { let _ = conn.execute_batch("ROLLBACK"); }
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
        let (new_shares, new_avg) = simulate_transaction(&conn, &acct_id, "AAPL", "BUY", 100.0, 20.0).unwrap();

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
        let (shares, avg) = simulate_transaction(&conn, &acct_id, "MSFT", "BUY", 30.0, 120.0).unwrap();
        assert!((shares - 80.0).abs() < 1e-9);
        // (50*100 + 30*120) / 80 = (5000 + 3600) / 80 = 107.5
        assert!((avg - 107.5).abs() < 1e-9);

        // Buy 20 more at $90
        let (shares2, avg2) = simulate_transaction(&conn, &acct_id, "MSFT", "BUY", 20.0, 90.0).unwrap();
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
        let (new_shares, new_avg) = simulate_transaction(&conn, &acct_id, "GOOG", "SELL", 30.0, 200.0).unwrap();
        assert!((new_shares - 70.0).abs() < 1e-9);
        assert!((new_avg - 150.0).abs() < 1e-9);
    }

    #[test]
    fn test_sell_all_shares() {
        let db = create_test_db();
        let conn = db.conn.lock().unwrap();
        let (acct_id, _) = setup_account_and_holding(&conn, "TSLA", 50.0, 200.0);

        // Sell exactly all shares
        let (new_shares, new_avg) = simulate_transaction(&conn, &acct_id, "TSLA", "SELL", 50.0, 250.0).unwrap();
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
}
