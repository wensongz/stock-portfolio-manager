use crate::commands::backup::{config_path, BackupConfig};
use crate::db::Database;
use crate::models::ai_config::{AiConfig, DEFAULT_SYSTEM_PROMPT};
use crate::models::quote_provider::QuoteProviderConfig;
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quote_service::{self, QuoteCache};
use chrono::Utc;
use rusqlite::{Result as SqlResult, Transaction};
use tauri::State;
use tracing::warn;

/// The same four system categories seeded by `db::run_migrations`.
/// Re-seeded here because migrations only run on startup; after wiping the
/// `categories` table the app would otherwise be left without them until
/// the next launch. Tuple = (name, color, icon, sort_order).
const SYSTEM_CATEGORIES: [(&str, &str, &str, i64); 4] = [
    ("现金类", "#22C55E", "💵", 1),
    ("分红股", "#3B82F6", "💰", 2),
    ("成长股", "#F97316", "🚀", 3),
    ("套利", "#8B5CF6", "🔄", 4),
];

pub(crate) fn clear_stock_review_data(tx: &Transaction<'_>) -> SqlResult<()> {
    for table in [
        "stock_daily_prices",
        "stock_review_annotations",
        "stock_review_overrides",
    ] {
        tx.execute(&format!("DELETE FROM {table};"), [])?;
    }
    Ok(())
}

/// Wipe every user-owned row in the database and reset the two config
/// tables to their built-in defaults, then clear the in-memory caches and
/// the backup config file.
///
/// This is the "factory reset" entry point. It runs in a single transaction
/// so a failure at any step rolls back every prior change — the user is
/// never left in a half-wiped state. localStorage-backed preferences are
/// cleared separately by the frontend after this command returns Ok.
#[tauri::command(rename_all = "camelCase")]
pub fn factory_reset(
    app: tauri::AppHandle,
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    rate_cache: State<'_, ExchangeRateCache>,
) -> Result<(), String> {
    // Reset the persisted backup config JSON to defaults. Done outside the
    // DB transaction because it is a plain file; a failure here aborts the
    // whole reset so the user gets a clear signal rather than a mixed state.
    let backup_defaults = BackupConfig::default();
    let backup_json = serde_json::to_string_pretty(&backup_defaults).map_err(|e| e.to_string())?;
    std::fs::write(config_path(&app), backup_json).map_err(|e| e.to_string())?;

    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    let tx = conn.transaction().map_err(|e| e.to_string())?;

    // --- Business data tables -------------------------------------------------
    // Order matters: delete children before parents so foreign-key
    // CASCADE / SET NULL rules don't leave dangling rows. We turn FK
    // enforcement off for the duration of the wipe so we can also DELETE
    // from tables in any order without surprises, then re-enable it.
    tx.execute_batch("PRAGMA foreign_keys = OFF;")
        .map_err(|e| e.to_string())?;

    clear_stock_review_data(&tx).map_err(|e| format!("failed to clear stock review data: {e}"))?;

    for table in [
        // AI chat history
        "chat_messages",
        "chat_sessions",
        // Quarterly reviews
        "quarterly_holding_snapshots",
        "quarterly_snapshots",
        // Daily snapshots / benchmarks
        "daily_holding_snapshots",
        "daily_portfolio_values",
        "benchmark_daily_prices",
        // Alerts
        "price_alerts",
        // Options
        "option_records",
        "option_share_lots",
        "stock_splits",
        // Core portfolio (transactions reference holdings & accounts)
        "transactions",
        "holdings",
        "accounts",
        // Cached upstream data
        "cached_quotes",
        "cached_exchange_rates",
        // Categories — re-seeded below
        "categories",
    ] {
        tx.execute(&format!("DELETE FROM {};", table), [])
            .map_err(|e| format!("failed to clear {}: {}", table, e))?;
    }

    // --- Reset config tables to built-in defaults ----------------------------
    let now = Utc::now().to_rfc3339();
    let qpp = QuoteProviderConfig::default();
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
            qpp.us_provider,
            qpp.hk_provider,
            qpp.cn_provider,
            qpp.cn_adjust_sell_pay_cost as i64,
            qpp.us_adjust_sell_pay_cost as i64,
            qpp.hk_adjust_sell_pay_cost as i64,
            now,
        ],
    )
    .map_err(|e| format!("failed to reset quote_provider_config: {}", e))?;

    let ai = AiConfig::default();
    tx.execute(
        "INSERT INTO ai_config (id, provider, api_key, model, base_url, system_prompt, updated_at)
         VALUES (1, ?1, ?2, ?3, NULL, ?4, ?5)
         ON CONFLICT(id) DO UPDATE SET
           provider = excluded.provider,
           api_key = excluded.api_key,
           model = excluded.model,
           base_url = NULL,
           system_prompt = excluded.system_prompt,
           updated_at = excluded.updated_at",
        rusqlite::params![
            ai.provider,
            ai.api_key,
            ai.model,
            DEFAULT_SYSTEM_PROMPT,
            now
        ],
    )
    .map_err(|e| format!("failed to reset ai_config: {}", e))?;

    // --- Re-seed system categories -------------------------------------------
    // Mirrors db::run_migrations so a fresh install and a reset app expose
    // the exact same initial category set.
    for (name, color, icon, sort_order) in SYSTEM_CATEGORIES.iter() {
        let id = uuid::Uuid::new_v4().to_string();
        tx.execute(
            "INSERT INTO categories (id, name, color, icon, is_system, sort_order, created_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6)",
            rusqlite::params![id, name, color, icon, sort_order, now],
        )
        .map_err(|e| format!("failed to re-seed category {}: {}", name, e))?;
    }

    tx.execute_batch("PRAGMA foreign_keys = ON;")
        .map_err(|e| e.to_string())?;

    tx.commit().map_err(|e| e.to_string())?;

    // --- In-memory caches ----------------------------------------------------
    // Done after commit so a clean DB is never paired with stale prices.
    quote_cache.clear();
    rate_cache.clear();

    // Forget any user-supplied Xueqiu credentials and invalidate the session
    // token so the next fetch rebuilds state from the now-empty config.
    quote_service::set_xueqiu_user_cookie(None);
    quote_service::set_xueqiu_user_u(None);
    quote_service::reset_xueqiu_token();

    // Restore AI skills to the factory set. Best-effort: a failure here
    // shouldn't undo the otherwise-successful wipe.
    if let Err(e) = crate::services::skill_service::reset_all_skills(&app) {
        warn!("factory_reset: failed to reset skills: {}", e);
    }

    Ok(())
}
