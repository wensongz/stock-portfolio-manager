use crate::db::Database;
use crate::models::alert::PriceAlert;
use crate::services::alert_service;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub async fn create_alert(
    db: State<'_, Database>,
    holding_id: Option<String>,
    symbol: String,
    name: String,
    market: String,
    alert_type: String,
    threshold: f64,
) -> Result<PriceAlert, String> {
    alert_service::create_alert(&db, holding_id, symbol, name, market, alert_type, threshold)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_alerts(db: State<'_, Database>) -> Result<Vec<PriceAlert>, String> {
    alert_service::get_alerts(&db)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn update_alert(
    db: State<'_, Database>,
    id: String,
    is_active: bool,
) -> Result<PriceAlert, String> {
    alert_service::update_alert(&db, &id, is_active)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn delete_alert(db: State<'_, Database>, id: String) -> Result<bool, String> {
    alert_service::delete_alert(&db, &id)
}
