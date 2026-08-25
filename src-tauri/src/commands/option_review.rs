use crate::db::Database;
use crate::models::option_review::OptionReviewReport;
use crate::services::option_review_service;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub fn get_option_review(
    db: State<Database>,
    account_id: String,
    period_days: Option<i64>,
) -> Result<OptionReviewReport, String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Err("accountId 不能为空".to_string());
    }
    option_review_service::get_option_review(&db, account_id, period_days)
}
