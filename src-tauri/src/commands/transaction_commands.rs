use crate::db::Database;
use crate::models::transaction::{CreateTransactionRequest, Transaction};
use crate::services::transaction_service;
use tauri::State;

#[tauri::command]
pub fn create_transaction(
    db: State<'_, Database>,
    request: CreateTransactionRequest,
) -> Result<Transaction, String> {
    transaction_service::create_transaction(&db, request)
}

#[tauri::command]
pub fn list_transactions(
    db: State<'_, Database>,
    account_id: Option<String>,
    symbol: Option<String>,
) -> Result<Vec<Transaction>, String> {
    transaction_service::list_transactions(&db, account_id.as_deref(), symbol.as_deref())
}

#[tauri::command]
pub fn delete_transaction(db: State<'_, Database>, id: String) -> Result<(), String> {
    transaction_service::delete_transaction(&db, &id)
}
