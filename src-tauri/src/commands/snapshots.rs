use crate::db::Database;
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quote_service::QuoteServiceState;
use chrono::NaiveDate;
use tauri::State;

/// Backfill missing daily snapshots for the given date range using historical
/// closing prices, including the preceding weekday as the performance baseline.
/// Returns the number of snapshots created.
///
/// When `force` is true, all completed dates in the range are recalculated,
/// including periods without transactions. Otherwise, only missing or
/// invalidated dates are filled in (fast cached load).
#[tauri::command(rename_all = "camelCase")]
pub async fn backfill_snapshots(
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
    quote_state: State<'_, QuoteServiceState>,
    start_date: String,
    end_date: String,
    force: Option<bool>,
) -> Result<i32, String> {
    let start = NaiveDate::parse_from_str(&start_date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid start_date format (expected YYYY-MM-DD): {}", e))?;
    let end = NaiveDate::parse_from_str(&end_date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid end_date format (expected YYYY-MM-DD): {}", e))?;
    if start > end {
        return Ok(0);
    }
    let backfill_start =
        crate::services::snapshot_cache_service::backfill_start_with_baseline(start);

    crate::services::snapshot_service::backfill_snapshots(
        &db,
        &cache,
        &quote_state,
        backfill_start,
        end,
        force.unwrap_or(false),
    )
    .await
}
