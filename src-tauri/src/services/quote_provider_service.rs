use crate::db::Database;
use crate::models::quote_provider::QuoteProviderConfig;
use chrono::Utc;

pub fn get_quote_provider_config(db: &Database) -> Result<QuoteProviderConfig, String> {
    let conn = db.conn.lock().unwrap();

    let result = conn.query_row(
        "SELECT us_provider, hk_provider, cn_provider FROM quote_provider_config WHERE id = 1",
        [],
        |row| {
            Ok(QuoteProviderConfig {
                us_provider: row.get(0)?,
                hk_provider: row.get(1)?,
                cn_provider: row.get(2)?,
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

    conn.execute(
        "INSERT INTO quote_provider_config (id, us_provider, hk_provider, cn_provider, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4)
         ON CONFLICT(id) DO UPDATE SET
           us_provider = excluded.us_provider,
           hk_provider = excluded.hk_provider,
           cn_provider = excluded.cn_provider,
           updated_at = excluded.updated_at",
        rusqlite::params![
            config.us_provider,
            config.hk_provider,
            config.cn_provider,
            now
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(true)
}
