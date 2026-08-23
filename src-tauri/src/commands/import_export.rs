use crate::db::Database;
use crate::models::import_export::{ExportFilters, ImportData, ImportPreview, ImportResult};
use crate::services::{broker_import_service, import_export_service};
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub async fn export_holdings_csv(
    db: State<'_, Database>,
    filters: ExportFilters,
) -> Result<String, String> {
    import_export_service::export_holdings_csv(&db, &filters)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn export_transactions_csv(
    db: State<'_, Database>,
    start_date: String,
    end_date: String,
    filters: ExportFilters,
) -> Result<String, String> {
    import_export_service::export_transactions_csv(&db, &start_date, &end_date, &filters)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_import_template(data_type: String) -> Result<String, String> {
    let content = if data_type == "holdings" {
        import_export_service::get_holdings_template()
    } else {
        import_export_service::get_transactions_template()
    };
    Ok(content)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn parse_import_csv(content: String, data_type: String) -> Result<ImportPreview, String> {
    import_export_service::parse_import_csv(&content, &data_type)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn convert_broker_statements(
    broker: String,
    ordinary_files: Vec<String>,
    credit_files: Vec<String>,
    supplement_files: Vec<String>,
    hsbc_files: Vec<String>,
) -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(move || match broker.as_str() {
        "everbright" => broker_import_service::convert_everbright(
            ordinary_files,
            credit_files,
            supplement_files,
        ),
        "hsbc_hk" => broker_import_service::convert_hsbc(hsbc_files),
        _ => Err(format!("暂不支持该券商：{}", broker)),
    })
    .await
    .map_err(|error| format!("券商文件转换任务失败：{}", error))?
}

#[tauri::command(rename_all = "camelCase")]
pub async fn confirm_import(
    db: State<'_, Database>,
    import_data: ImportData,
) -> Result<ImportResult, String> {
    import_export_service::confirm_import(&db, &import_data)
}
