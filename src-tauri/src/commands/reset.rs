use crate::commands::backup::{save_config, BackupConfig};
use crate::db::{Database, SYSTEM_CATEGORIES};
use crate::models::ai_config::AiConfig;
use crate::models::quote_provider::QuoteProviderConfig;
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quote_service::{self, QuoteCache, QuoteServiceState};
use chrono::Utc;
use rusqlite::{Connection, Result as SqlResult, Transaction};
use tauri::State;
use tracing::warn;

pub(crate) fn clear_stock_operation_review_cache(tx: &Transaction<'_>) -> SqlResult<()> {
    tx.execute("DELETE FROM stock_daily_prices", [])?;
    Ok(())
}

pub(crate) fn reset_database_state(conn: &mut Connection, now: &str) -> Result<(), String> {
    let tx = conn.transaction().map_err(|error| error.to_string())?;
    clear_stock_operation_review_cache(&tx)
        .map_err(|error| format!("failed to clear stock operation review cache: {error}"))?;

    for table in [
        "chat_messages",
        "chat_sessions",
        "quarterly_holding_snapshots",
        "quarterly_snapshots",
        "daily_holding_snapshots",
        "daily_portfolio_values",
        "benchmark_daily_prices",
        "price_alerts",
        "option_records",
        "option_share_lots",
        "stock_splits",
        "transactions",
        "holdings",
        "portfolio_alert_breaches",
        "portfolio_alert_targets",
        "portfolio_alert_configs",
        "accounts",
        "cached_quotes",
        "cached_exchange_rates",
        "cached_quote_refresh_time",
        "categories",
    ] {
        tx.execute(&format!("DELETE FROM {table}"), [])
            .map_err(|error| format!("failed to clear {table}: {error}"))?;
    }

    let quote = QuoteProviderConfig::default();
    tx.execute(
        "INSERT INTO quote_provider_config
             (id, us_provider, hk_provider, cn_provider, xueqiu_cookie, xueqiu_u,
              cn_adjust_sell_pay_cost, us_adjust_sell_pay_cost, hk_adjust_sell_pay_cost, updated_at)
         VALUES (1, ?1, ?2, ?3, NULL, NULL, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
           us_provider = excluded.us_provider,
           hk_provider = excluded.hk_provider,
           cn_provider = excluded.cn_provider,
           xueqiu_cookie = NULL,
           xueqiu_u = NULL,
           cn_adjust_sell_pay_cost = excluded.cn_adjust_sell_pay_cost,
           us_adjust_sell_pay_cost = excluded.us_adjust_sell_pay_cost,
           hk_adjust_sell_pay_cost = excluded.hk_adjust_sell_pay_cost,
           updated_at = excluded.updated_at",
        rusqlite::params![
            quote.us_provider,
            quote.hk_provider,
            quote.cn_provider,
            quote.cn_adjust_sell_pay_cost as i64,
            quote.us_adjust_sell_pay_cost as i64,
            quote.hk_adjust_sell_pay_cost as i64,
            now,
        ],
    )
    .map_err(|error| format!("failed to reset quote_provider_config: {error}"))?;

    let ai = AiConfig::default();
    tx.execute(
        "INSERT INTO ai_config
             (id, provider, api_key, model, base_url, system_prompt, tools_enabled, updated_at)
         VALUES (1, ?1, ?2, ?3, NULL, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
           provider = excluded.provider,
           api_key = excluded.api_key,
           model = excluded.model,
           base_url = NULL,
           system_prompt = excluded.system_prompt,
           tools_enabled = excluded.tools_enabled,
           updated_at = excluded.updated_at",
        rusqlite::params![
            ai.provider,
            ai.api_key,
            ai.model,
            ai.system_prompt,
            ai.tools_enabled as i64,
            now,
        ],
    )
    .map_err(|error| format!("failed to reset ai_config: {error}"))?;

    for (name, color, icon, sort_order) in SYSTEM_CATEGORIES {
        tx.execute(
            "INSERT INTO categories (id, name, color, icon, is_system, sort_order, created_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6)",
            rusqlite::params![
                uuid::Uuid::new_v4().to_string(),
                name,
                color,
                icon,
                sort_order,
                now
            ],
        )
        .map_err(|error| format!("failed to re-seed category {name}: {error}"))?;
    }

    tx.commit().map_err(|error| error.to_string())
}

/// Wipe every active user-owned row managed by the current runtime, reset the
/// two config tables to their built-in defaults, then clear the in-memory
/// caches and the backup config file. Retired legacy stock-review tables are
/// intentionally left inert: they are not read, migrated, written, deleted,
/// or reset.
///
/// Database changes run in one transaction. The backup preference file is
/// replaced only after that commit; if the file update fails, the command
/// reports that the database reset succeeded instead of claiming cross-file
/// atomicity. localStorage-backed preferences are cleared separately by the
/// frontend after this command returns Ok.
#[tauri::command(rename_all = "camelCase")]
pub fn factory_reset(
    app: tauri::AppHandle,
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    quote_state: State<'_, QuoteServiceState>,
    rate_cache: State<'_, ExchangeRateCache>,
) -> Result<(), String> {
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = Utc::now().to_rfc3339();
    reset_database_state(&mut conn, &now)?;
    drop(conn);

    // --- In-memory caches ----------------------------------------------------
    // Done after commit so a clean DB is never paired with stale prices.
    quote_cache.clear();
    rate_cache.clear();

    // Forget any user-supplied Xueqiu credentials and invalidate the session
    // token so the next fetch rebuilds state from the now-empty config.
    quote_service::set_xueqiu_user_cookie(&quote_state, None);
    quote_service::set_xueqiu_user_u(&quote_state, None);
    quote_service::reset_xueqiu_token(&quote_state);

    // Restore AI skills to the factory set. Best-effort: a failure here
    // shouldn't undo the otherwise-successful wipe.
    if let Err(e) = crate::services::skill_service::reset_all_skills(&app) {
        warn!("factory_reset: failed to reset skills: {}", e);
    }

    save_config(&app, &BackupConfig::default())
        .map_err(|error| format!("数据库已恢复出厂设置，但备份偏好重置失败: {error}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::reset_database_state;
    use crate::db::Database;
    use crate::models::ai_config::AiConfig;

    fn row_count(conn: &rusqlite::Connection, table: &str) -> i64 {
        conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
            row.get(0)
        })
        .unwrap()
    }

    #[test]
    fn factory_reset_clears_all_portfolio_alert_rows() {
        let db = Database::new(":memory:").unwrap();
        let mut conn = db.conn.lock().unwrap();
        conn.execute_batch(
            "INSERT INTO categories (id, name, color, icon, created_at)
               VALUES ('category-1', 'Growth', '#F97316', '🚀', '2026-09-06');
             INSERT INTO portfolio_alert_configs
               (id, scope_key, scope_kind, base_currency, deviation_threshold,
                concentration_threshold, is_active, created_at, updated_at)
               VALUES ('config-1', 'overall', 'OVERALL', 'USD', 20, 20, 1, '2026-09-06', '2026-09-06');
             INSERT INTO portfolio_alert_targets (config_id, category_id, target_percent)
               VALUES ('config-1', 'category-1', 100);
             INSERT INTO portfolio_alert_breaches
               (config_id, breach_key, breach_kind, direction, first_triggered_at, last_seen_at)
               VALUES ('config-1', 'category:category-1', 'CATEGORY_DEVIATION', 'OVERWEIGHT',
                       '2026-09-06', '2026-09-06');",
        )
        .unwrap();

        reset_database_state(&mut conn, "2026-09-06T10:00:00Z").unwrap();

        assert_eq!(row_count(&conn, "portfolio_alert_breaches"), 0);
        assert_eq!(row_count(&conn, "portfolio_alert_targets"), 0);
        assert_eq!(row_count(&conn, "portfolio_alert_configs"), 0);
    }

    #[test]
    fn reset_restores_every_persisted_default_and_clears_user_data() {
        let db = Database::new(":memory:").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
                   VALUES ('account-1', 'Reset', 'US', '2026-09-01', '2026-09-01');
                 INSERT INTO holdings
                   (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
                   VALUES ('holding-1', 'account-1', 'AAPL', 'Apple', 'US', 1, 100, 'USD', '2026-09-01', '2026-09-01');
                 INSERT INTO transactions
                   (id, holding_id, account_id, symbol, name, market, transaction_type, shares, price, total_amount, commission, currency, traded_at, created_at)
                   VALUES ('transaction-1', 'holding-1', 'account-1', 'AAPL', 'Apple', 'US', 'OPEN', 1, 100, 100, 0, 'USD', '2026-09-01', '2026-09-01');
                 INSERT INTO cached_quote_refresh_time (id, updated_at)
                   VALUES (1, 'stale-refresh-time');
                 INSERT INTO ai_config
                   (id, provider, api_key, model, base_url, system_prompt, tools_enabled, updated_at)
                   VALUES (1, 'custom', 'secret', 'custom-model', 'https://example.test', 'custom prompt', 0, 'old');
                 INSERT INTO quote_provider_config
                   (id, us_provider, hk_provider, cn_provider, xueqiu_cookie, xueqiu_u,
                    cn_adjust_sell_pay_cost, us_adjust_sell_pay_cost, hk_adjust_sell_pay_cost, updated_at)
                   VALUES (1, 'yahoo', 'eastmoney', 'eastmoney', 'token', 'user', 0, 1, 1, 'old');",
            )
            .unwrap();
        }

        {
            let mut conn = db.conn.lock().unwrap();
            reset_database_state(&mut conn, "2026-09-01T00:00:00Z").unwrap();
        }

        let conn = db.conn.lock().unwrap();
        for table in [
            "accounts",
            "holdings",
            "transactions",
            "cached_quote_refresh_time",
        ] {
            let count: i64 = conn
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(count, 0, "{table} was not cleared");
        }

        let ai: (String, String, String, Option<String>, String, i64) = conn
            .query_row(
                "SELECT provider, api_key, model, base_url, system_prompt, tools_enabled
                 FROM ai_config WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                    ))
                },
            )
            .unwrap();
        let default_ai = AiConfig::default();
        assert_eq!(
            ai,
            (
                default_ai.provider,
                default_ai.api_key,
                default_ai.model,
                default_ai.base_url,
                default_ai.system_prompt,
                default_ai.tools_enabled as i64,
            )
        );

        let quote: (
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            i64,
            i64,
            i64,
        ) = conn
            .query_row(
                "SELECT us_provider, hk_provider, cn_provider, xueqiu_cookie, xueqiu_u,
                        cn_adjust_sell_pay_cost, us_adjust_sell_pay_cost, hk_adjust_sell_pay_cost
                 FROM quote_provider_config WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get(0)?,
                        row.get(1)?,
                        row.get(2)?,
                        row.get(3)?,
                        row.get(4)?,
                        row.get(5)?,
                        row.get(6)?,
                        row.get(7)?,
                    ))
                },
            )
            .unwrap();
        assert_eq!(
            quote,
            (
                "xueqiu".to_string(),
                "xueqiu".to_string(),
                "xueqiu".to_string(),
                None,
                None,
                1,
                0,
                0,
            )
        );

        let categories: Vec<(String, String, String, i64)> = conn
            .prepare(
                "SELECT name, color, icon, sort_order FROM categories
                 WHERE is_system = 1 ORDER BY sort_order",
            )
            .unwrap()
            .query_map([], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(
            categories,
            [
                ("现金类", "#22C55E", "💵", 1),
                ("分红股", "#3B82F6", "💰", 2),
                ("成长股", "#F97316", "🚀", 3),
                ("套利", "#8B5CF6", "🔄", 4),
            ]
            .map(|(name, color, icon, sort_order)| {
                (
                    name.to_string(),
                    color.to_string(),
                    icon.to_string(),
                    sort_order,
                )
            })
        );
    }
}
