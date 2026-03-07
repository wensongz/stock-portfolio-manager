use crate::db::Database;
use crate::models::review::{DecisionStatistics, HoldingReview};
use crate::services::review_service;
use tauri::State;

#[tauri::command]
pub async fn get_holding_review(
    db: State<'_, Database>,
    symbol: String,
) -> Result<HoldingReview, String> {
    review_service::get_holding_review(&db, &symbol)
}

#[tauri::command]
pub async fn update_decision_quality(
    db: State<'_, Database>,
    snapshot_id: String,
    symbol: String,
    quality: String,
) -> Result<bool, String> {
    review_service::update_decision_quality(&db, &snapshot_id, &symbol, &quality)
}

#[tauri::command]
pub async fn get_decision_statistics(
    db: State<'_, Database>,
) -> Result<DecisionStatistics, String> {
    review_service::get_decision_statistics(&db)
}

#[tauri::command]
pub async fn get_reviewed_symbols(
    db: State<'_, Database>,
) -> Result<Vec<(String, String, String)>, String> {
    review_service::get_reviewed_symbols(&db)
}
