use crate::db::Database;
use crate::models::alert_history::{AlertHistoryPage, AlertHistoryQuery};
use crate::services::alert_history_service;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub fn get_alert_history(
    db: State<'_, Database>,
    query: AlertHistoryQuery,
) -> Result<AlertHistoryPage, String> {
    alert_history_service::get_alert_history(&db, query)
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_alert_history(db: State<'_, Database>, id: String) -> Result<bool, String> {
    alert_history_service::delete_alert_history(&db, &id)
}
