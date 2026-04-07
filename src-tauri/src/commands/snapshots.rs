use crate::db::Database;
use crate::models::DailyPortfolioValue;
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quote_service::QuoteCache;
use crate::services::snapshot_service::{get_daily_values, take_daily_snapshot};
use chrono::NaiveDate;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub async fn take_snapshot(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    quote_cache: State<'_, QuoteCache>,
    date: Option<String>,
) -> Result<bool, String> {
    let target_date = match date {
        Some(d) => NaiveDate::parse_from_str(&d, "%Y-%m-%d")
            .map_err(|e| format!("Invalid date format (expected YYYY-MM-DD): {}", e))?,
        None => {
            // Use the last market-closed date so we never snapshot a day
            // whose closing prices are not yet available.
            crate::services::snapshot_service::last_closed_market_date()
        }
    };

    take_daily_snapshot(&db, &cache, &quote_cache, target_date).await?;
    Ok(true)
}

#[tauri::command(rename_all = "camelCase")]
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

/// Backfill missing daily snapshots for the given date range using historical
/// closing prices. Returns the number of snapshots created.
///
/// When `force` is true, all snapshots in the range are re-created if
/// transactions exist (full recalculation). When false, only dates that have
/// never been computed are filled in (fast cached load).
#[tauri::command(rename_all = "camelCase")]
pub async fn backfill_snapshots(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    start_date: String,
    end_date: String,
    force: Option<bool>,
) -> Result<i32, String> {
    let start = NaiveDate::parse_from_str(&start_date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid start_date format (expected YYYY-MM-DD): {}", e))?;
    let end = NaiveDate::parse_from_str(&end_date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid end_date format (expected YYYY-MM-DD): {}", e))?;

    crate::services::snapshot_service::backfill_snapshots(&db, &cache, start, end, force.unwrap_or(false)).await
}
