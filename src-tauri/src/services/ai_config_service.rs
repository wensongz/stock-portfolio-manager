use crate::db::Database;
use crate::models::ai_config::AiConfig;
use chrono::Utc;

pub fn get_ai_config(db: &Database) -> Result<AiConfig, String> {
    let conn = db.conn.lock().unwrap();

    let result = conn.query_row(
        "SELECT provider, api_key, model, base_url, system_prompt FROM ai_config WHERE id = 1",
        [],
        |row| {
            Ok(AiConfig {
                provider: row.get(0)?,
                api_key: row.get(1)?,
                model: row.get(2)?,
                base_url: row.get(3)?,
                system_prompt: row.get(4)?,
            })
        },
    );

    match result {
        Ok(config) => Ok(config),
        Err(_) => Ok(AiConfig::default()),
    }
}

pub fn update_ai_config(db: &Database, config: &AiConfig) -> Result<bool, String> {
    let conn = db.conn.lock().unwrap();
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO ai_config (id, provider, api_key, model, base_url, system_prompt, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(id) DO UPDATE SET
           provider = excluded.provider,
           api_key = excluded.api_key,
           model = excluded.model,
           base_url = excluded.base_url,
           system_prompt = excluded.system_prompt,
           updated_at = excluded.updated_at",
        rusqlite::params![
            config.provider,
            config.api_key,
            config.model,
            config.base_url,
            config.system_prompt,
            now
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(true)
}
