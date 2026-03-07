use crate::db::Database;
use crate::models::import_export::{ExportFilters, ImportData, ImportPreview, ImportResult};
use crate::services::import_export_service;
use tauri::State;

#[tauri::command]
pub async fn export_holdings_csv(
    db: State<'_, Database>,
    filters: ExportFilters,
) -> Result<String, String> {
    import_export_service::export_holdings_csv(&db, &filters)
}

#[tauri::command]
pub async fn export_transactions_csv(
    db: State<'_, Database>,
    start_date: String,
    end_date: String,
    filters: ExportFilters,
) -> Result<String, String> {
    import_export_service::export_transactions_csv(&db, &start_date, &end_date, &filters)
}

#[tauri::command]
pub async fn get_import_template(data_type: String) -> Result<String, String> {
    let content = if data_type == "holdings" {
        import_export_service::get_holdings_template()
    } else {
        import_export_service::get_transactions_template()
    };
    Ok(content)
}

#[tauri::command]
pub async fn parse_import_csv(
    content: String,
    data_type: String,
) -> Result<ImportPreview, String> {
    import_export_service::parse_import_csv(&content, &data_type)
}

#[tauri::command]
pub async fn confirm_import(
    db: State<'_, Database>,
    import_data: ImportData,
) -> Result<ImportResult, String> {
    import_export_service::confirm_import(&db, &import_data)
}
