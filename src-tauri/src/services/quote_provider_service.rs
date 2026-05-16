use crate::db::Database;
use crate::models::quote_provider::QuoteProviderConfig;
use chrono::Utc;

pub fn get_quote_provider_config(db: &Database) -> Result<QuoteProviderConfig, String> {
    let conn = db.conn.lock().unwrap();

    let result = conn.query_row(
        "SELECT us_provider, hk_provider, cn_provider, xueqiu_cookie, xueqiu_u,
                cn_adjust_sell_pay_cost, us_adjust_sell_pay_cost, hk_adjust_sell_pay_cost
         FROM quote_provider_config WHERE id = 1",
        [],
        |row| {
            Ok(QuoteProviderConfig {
                us_provider: row.get(0)?,
                hk_provider: row.get(1)?,
                cn_provider: row.get(2)?,
                xueqiu_cookie: row.get(3)?,
                xueqiu_u: row.get(4)?,
                cn_adjust_sell_pay_cost: row.get::<_, i64>(5).unwrap_or(1) != 0,
                us_adjust_sell_pay_cost: row.get::<_, i64>(6).unwrap_or(0) != 0,
                hk_adjust_sell_pay_cost: row.get::<_, i64>(7).unwrap_or(0) != 0,
            })
        },
    );

    match result {
        Ok(config) => Ok(config),
        Err(_) => Ok(QuoteProviderConfig::default()),
    }
}

pub fn update_quote_provider_config(
    db: &Database,
    config: &QuoteProviderConfig,
) -> Result<bool, String> {
    // Validate provider values
    match config.us_provider.as_str() {
        "yahoo" | "eastmoney" | "xueqiu" => {}
        _ => return Err(format!("Invalid US provider: {}", config.us_provider)),
    }
    match config.hk_provider.as_str() {
        "yahoo" | "eastmoney" | "xueqiu" => {}
        _ => return Err(format!("Invalid HK provider: {}", config.hk_provider)),
    }
    match config.cn_provider.as_str() {
        "eastmoney" | "xueqiu" => {}
        _ => return Err(format!("Invalid CN provider ({}). Only 'eastmoney' and 'xueqiu' are supported.", config.cn_provider)),
    }

    let conn = db.conn.lock().unwrap();
    let now = Utc::now().to_rfc3339();

    // Normalize empty / whitespace-only values to NULL.
    let xueqiu_cookie = config
        .xueqiu_cookie
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    let xueqiu_u = config
        .xueqiu_u
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());

    conn.execute(
        "INSERT INTO quote_provider_config
             (id, us_provider, hk_provider, cn_provider, xueqiu_cookie, xueqiu_u,
              cn_adjust_sell_pay_cost, us_adjust_sell_pay_cost, hk_adjust_sell_pay_cost, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
           us_provider = excluded.us_provider,
           hk_provider = excluded.hk_provider,
           cn_provider = excluded.cn_provider,
           xueqiu_cookie = excluded.xueqiu_cookie,
           xueqiu_u = excluded.xueqiu_u,
           cn_adjust_sell_pay_cost = excluded.cn_adjust_sell_pay_cost,
           us_adjust_sell_pay_cost = excluded.us_adjust_sell_pay_cost,
           hk_adjust_sell_pay_cost = excluded.hk_adjust_sell_pay_cost,
           updated_at = excluded.updated_at",
        rusqlite::params![
            config.us_provider,
            config.hk_provider,
            config.cn_provider,
            xueqiu_cookie,
            xueqiu_u,
            config.cn_adjust_sell_pay_cost as i64,
            config.us_adjust_sell_pay_cost as i64,
            config.hk_adjust_sell_pay_cost as i64,
            now
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(true)
}

/// Return whether SELL and PAY transactions should adjust avg_cost for the given market.
/// Reads from the single-row `quote_provider_config` table.
/// Defaults: CN = true, US = false, HK = false.
pub fn market_adjusts_sell_pay_cost(conn: &rusqlite::Connection, market: &str) -> bool {
    // Map market to a fixed SQL query — never interpolate user input into SQL.
    let (query, default_val): (&str, i64) = match market {
        "CN" => (
            "SELECT cn_adjust_sell_pay_cost FROM quote_provider_config WHERE id = 1",
            1,
        ),
        "US" => (
            "SELECT us_adjust_sell_pay_cost FROM quote_provider_config WHERE id = 1",
            0,
        ),
        "HK" => (
            "SELECT hk_adjust_sell_pay_cost FROM quote_provider_config WHERE id = 1",
            0,
        ),
        _ => return true, // unknown market: safe default (adjust)
    };
    conn.query_row(query, [], |row| row.get::<_, i64>(0))
        .unwrap_or(default_val)
        != 0
}
