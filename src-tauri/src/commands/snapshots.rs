use crate::db::Database;
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::quote_service::QuoteServiceState;
use chrono::NaiveDate;
use tauri::State;

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
    quote_state: State<'_, QuoteServiceState>,
    start_date: String,
    end_date: String,
    force: Option<bool>,
) -> Result<i32, String> {
    let start = NaiveDate::parse_from_str(&start_date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid start_date format (expected YYYY-MM-DD): {}", e))?;
    let end = NaiveDate::parse_from_str(&end_date, "%Y-%m-%d")
        .map_err(|e| format!("Invalid end_date format (expected YYYY-MM-DD): {}", e))?;

    crate::services::snapshot_service::backfill_snapshots(
        &db,
        &cache,
        &quote_state,
        start,
        end,
        force.unwrap_or(false),
    )
    .await
}
