use crate::db::Database;
use crate::models::holding::{CreateHoldingRequest, Holding, UpdateHoldingRequest};
use crate::services::holding_service;
use tauri::State;

#[tauri::command]
pub fn create_holding(
    db: State<'_, Database>,
    request: CreateHoldingRequest,
) -> Result<Holding, String> {
    holding_service::create_holding(&db, request)
}

#[tauri::command]
pub fn list_holdings(
    db: State<'_, Database>,
    account_id: Option<String>,
) -> Result<Vec<Holding>, String> {
    holding_service::list_holdings(&db, account_id.as_deref())
}

#[tauri::command]
pub fn get_holding(db: State<'_, Database>, id: String) -> Result<Holding, String> {
    holding_service::get_holding(&db, &id)
}

#[tauri::command]
pub fn update_holding(
    db: State<'_, Database>,
    id: String,
    request: UpdateHoldingRequest,
) -> Result<Holding, String> {
    holding_service::update_holding(&db, &id, request)
}

#[tauri::command]
pub fn delete_holding(db: State<'_, Database>, id: String) -> Result<(), String> {
    holding_service::delete_holding(&db, &id)
}
