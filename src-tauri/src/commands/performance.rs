use crate::db::Database;
use crate::models::performance::{PerformanceReport, ReturnDataPoint};
use crate::services::performance_service;
use crate::services::performance_service::PerformanceFilter;
use tauri::State;

/// How many calendar days before the requested start to fetch so we can find
/// the previous trading day's closing price for the baseline.
const BENCHMARK_BASELINE_LOOKBACK_DAYS: i64 = 10;

fn parse_date(s: &str) -> Result<chrono::NaiveDate, String> {
    crate::models::performance::parse_date(s)
}

fn build_filter(market: Option<String>, account_id: Option<String>) -> PerformanceFilter {
    PerformanceFilter { market, account_id }
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_performance_report(
    db: State<'_, Database>,
    start_date: String,
    end_date: String,
    market: Option<String>,
    account_id: Option<String>,
    ranking_limit: u32,
) -> Result<PerformanceReport, String> {
    let start = parse_date(&start_date)?;
    let end = parse_date(&end_date)?;
    let filter = build_filter(market, account_id);
    performance_service::get_performance_report(
        &db,
        start,
        end,
        "pnl",
        ranking_limit as usize,
        &filter,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_benchmark_return_series(
    db: State<'_, Database>,
    symbol: String,
    start_date: String,
    end_date: String,
) -> Result<Vec<ReturnDataPoint>, String> {
    let start = parse_date(&start_date)?;
    let end = parse_date(&end_date)?;
    // Fetch a few extra days before start so we can find the previous
    // trading day's closing price to use as the baseline.
    let fetch_start = start - chrono::Duration::days(BENCHMARK_BASELINE_LOOKBACK_DAYS);
    let points =
        performance_service::fetch_benchmark_history(&db, &symbol, fetch_start, end).await?;
    let start_str = start.format("%Y-%m-%d").to_string();
    let base_price = points
        .iter()
        .rfind(|p| p.date < start_str)
        .map(|p| p.close_price);
    let visible: Vec<_> = points.into_iter().filter(|p| p.date >= start_str).collect();
    Ok(performance_service::benchmark_to_return_series(
        &visible, base_price,
    ))
}
