use crate::db::Database;
use crate::models::DailyPortfolioValue;
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::snapshot_service::{get_daily_values, take_daily_snapshot};
use chrono::NaiveDate;
use tauri::State;

#[tauri::command]
pub async fn take_snapshot(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    date: Option<String>,
) -> Result<bool, String> {
    let target_date = match date {
        Some(d) => NaiveDate::parse_from_str(&d, "%Y-%m-%d")
            .map_err(|e| format!("Invalid date format (expected YYYY-MM-DD): {}", e))?,
        None => chrono::Utc::now().date_naive(),
    };

    take_daily_snapshot(&db, &cache, target_date).await?;
    Ok(true)
}

#[tauri::command]
pub fn get_portfolio_history(
    db: State<'_, Database>,
    start_date: String,
    end_date: String,
) -> Result<Vec<DailyPortfolioValue>, String> {
    let start = NaiveDate::parse_from_str(&start_date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid start_date format (expected YYYY-MM-DD): {}", e))?;
    let end = NaiveDate::parse_from_str(&end_date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid end_date format (expected YYYY-MM-DD): {}", e))?;

    get_daily_values(&db, start, end)
}
