use crate::db::Database;
use crate::models::import_batch::{ExpectedBalance, ImportBatch, ImportBatchRequest};
use crate::services::{import_batch, import_export_service};
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub fn preview_import_batch(
    db: State<Database>,
    request: ImportBatchRequest,
) -> Result<ImportBatch, String> {
    import_batch::preview_import_batch(&db, &request)
}
#[tauri::command(rename_all = "camelCase")]
pub fn get_import_batch(db: State<Database>, batch_id: String) -> Result<ImportBatch, String> {
    import_batch::get_import_batch(&db, &batch_id)
}
#[tauri::command(rename_all = "camelCase")]
pub fn list_import_batches(
    db: State<Database>,
    account_id: Option<String>,
) -> Result<Vec<ImportBatch>, String> {
    import_batch::list_import_batches(&db, account_id.as_deref())
}
#[tauri::command(rename_all = "camelCase")]
pub fn apply_import_batch(
    db: State<Database>,
    batch_id: String,
    row_keys: Vec<String>,
    allow_suspected_keys: Vec<String>,
) -> Result<ImportBatch, String> {
    import_batch::apply_import_batch(&db, &batch_id, &row_keys, &allow_suspected_keys)
}
#[tauri::command(rename_all = "camelCase")]
pub fn undo_import_batch(db: State<Database>, batch_id: String) -> Result<ImportBatch, String> {
    import_batch::undo_import_batch(&db, &batch_id)
}
#[tauri::command(rename_all = "camelCase")]
pub fn reconcile_import_batch(
    db: State<Database>,
    batch_id: String,
    balances: Vec<ExpectedBalance>,
) -> Result<ImportBatch, String> {
    import_batch::reconcile_import_batch(&db, &batch_id, &balances)
}
#[tauri::command(rename_all = "camelCase")]
pub fn preview_csv_import_batch(
    db: State<Database>,
    content: String,
    data_type: String,
    account_id: String,
    file_name: String,
    request_id: String,
) -> Result<ImportBatch, String> {
    import_export_service::preview_csv_import_batch(
        &db,
        &content,
        &data_type,
        &account_id,
        &file_name,
        &request_id,
    )
}
