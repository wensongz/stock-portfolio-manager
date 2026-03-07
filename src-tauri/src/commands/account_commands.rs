use crate::db::Database;
use crate::models::account::{Account, CreateAccountRequest, UpdateAccountRequest};
use crate::services::account_service;
use tauri::State;

#[tauri::command]
pub fn create_account(
    db: State<'_, Database>,
    request: CreateAccountRequest,
) -> Result<Account, String> {
    account_service::create_account(&db, request)
}

#[tauri::command]
pub fn list_accounts(db: State<'_, Database>) -> Result<Vec<Account>, String> {
    account_service::list_accounts(&db)
}

#[tauri::command]
pub fn get_account(db: State<'_, Database>, id: String) -> Result<Account, String> {
    account_service::get_account(&db, &id)
}

#[tauri::command]
pub fn update_account(
    db: State<'_, Database>,
    id: String,
    request: UpdateAccountRequest,
) -> Result<Account, String> {
    account_service::update_account(&db, &id, request)
}

#[tauri::command]
pub fn delete_account(db: State<'_, Database>, id: String) -> Result<(), String> {
    account_service::delete_account(&db, &id)
}
