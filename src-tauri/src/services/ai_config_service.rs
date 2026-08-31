use crate::db::Database;
use crate::models::ai_config::AiConfig;
use chrono::Utc;

pub fn get_ai_config(db: &Database) -> Result<AiConfig, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;

    let result = conn.query_row(
        "SELECT provider, api_key, model, base_url, system_prompt,
                COALESCE(tools_enabled, 1) FROM ai_config WHERE id = 1",
        [],
        |row| {
            let tools_enabled: bool = row.get(5)?;
            Ok(AiConfig {
                provider: row.get(0)?,
                api_key: row.get(1)?,
                model: row.get(2)?,
                base_url: row.get(3)?,
                system_prompt: row.get(4)?,
                tools_enabled,
            })
        },
    );

    match result {
        Ok(config) => Ok(config),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(AiConfig::default()),
        Err(error) => Err(error.to_string()),
    }
}

pub fn update_ai_config(db: &Database, config: &AiConfig) -> Result<bool, String> {
    let conn = db.conn.lock().unwrap();
    let now = Utc::now().to_rfc3339();

    conn.execute(
        "INSERT INTO ai_config (id, provider, api_key, model, base_url, system_prompt, tools_enabled, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT(id) DO UPDATE SET
           provider = excluded.provider,
           api_key = excluded.api_key,
           model = excluded.model,
           base_url = excluded.base_url,
           system_prompt = excluded.system_prompt,
           tools_enabled = excluded.tools_enabled,
           updated_at = excluded.updated_at",
        rusqlite::params![
            config.provider,
            config.api_key,
            config.model,
            config.base_url,
            config.system_prompt,
            config.tools_enabled,
            now
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::get_ai_config;
    use crate::db::Database;

    #[test]
    fn schema_errors_are_not_reported_as_default_config() {
        let db = Database::new(":memory:").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "DROP TABLE ai_config;
                 CREATE TABLE ai_config (id INTEGER PRIMARY KEY, provider TEXT);
                 INSERT INTO ai_config VALUES (1, 'broken');",
            )
            .unwrap();
        }

        let error = get_ai_config(&db).unwrap_err();
        assert!(error.contains("api_key") || error.contains("column"));
    }
}
