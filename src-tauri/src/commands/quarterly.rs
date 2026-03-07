use crate::db::Database;
use crate::models::quarterly::{
    HoldingNoteHistory, QuarterComparison, QuarterlyNotesSummary, QuarterlySnapshot,
    QuarterlySnapshotDetail, QuarterlyTrends,
};
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quarterly_service;
use tauri::State;

#[tauri::command(rename_all = "snake_case")]
pub async fn create_quarterly_snapshot(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    quarter: Option<String>,
) -> Result<QuarterlySnapshot, String> {
    quarterly_service::create_quarterly_snapshot(&db, &cache, quarter).await
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_quarterly_snapshots(
    db: State<'_, Database>,
) -> Result<Vec<QuarterlySnapshot>, String> {
    quarterly_service::get_quarterly_snapshots(&db)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_quarterly_snapshot_detail(
    db: State<'_, Database>,
    snapshot_id: String,
) -> Result<QuarterlySnapshotDetail, String> {
    quarterly_service::get_quarterly_snapshot_detail(&db, &snapshot_id)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn delete_quarterly_snapshot(
    db: State<'_, Database>,
    snapshot_id: String,
) -> Result<bool, String> {
    quarterly_service::delete_quarterly_snapshot(&db, &snapshot_id)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn check_missing_snapshots(
    db: State<'_, Database>,
) -> Result<Vec<String>, String> {
    quarterly_service::check_missing_snapshots(&db)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn compare_quarters(
    db: State<'_, Database>,
    quarter1: String,
    quarter2: String,
) -> Result<QuarterComparison, String> {
    quarterly_service::compare_quarters(&db, &quarter1, &quarter2)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn update_holding_notes(
    db: State<'_, Database>,
    snapshot_id: String,
    symbol: String,
    notes: String,
) -> Result<bool, String> {
    quarterly_service::update_holding_notes(&db, &snapshot_id, &symbol, &notes)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_holding_notes_history(
    db: State<'_, Database>,
    symbol: String,
) -> Result<Vec<HoldingNoteHistory>, String> {
    quarterly_service::get_holding_notes_history(&db, &symbol)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn update_quarterly_notes(
    db: State<'_, Database>,
    snapshot_id: String,
    notes: String,
) -> Result<bool, String> {
    quarterly_service::update_quarterly_notes(&db, &snapshot_id, &notes)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_quarterly_notes_history(
    db: State<'_, Database>,
) -> Result<Vec<QuarterlyNotesSummary>, String> {
    quarterly_service::get_quarterly_notes_history(&db)
}

#[tauri::command(rename_all = "snake_case")]
pub async fn get_quarterly_trends(
    db: State<'_, Database>,
) -> Result<QuarterlyTrends, String> {
    quarterly_service::get_quarterly_trends(&db)
}
