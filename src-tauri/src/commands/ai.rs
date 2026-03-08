use crate::db::Database;
use crate::models::ai_config::AiConfig;
use crate::services::ai_config_service;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub async fn get_ai_config(db: State<'_, Database>) -> Result<AiConfig, String> {
    ai_config_service::get_ai_config(&db)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn update_ai_config(
    db: State<'_, Database>,
    config: AiConfig,
) -> Result<bool, String> {
    ai_config_service::update_ai_config(&db, &config)
}
