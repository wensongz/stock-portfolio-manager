use crate::db::Database;
use crate::models::performance::{BenchmarkDataPoint, PerformanceReport, ReturnDataPoint};
use crate::services::performance_service;
use crate::services::performance_service::PerformanceFilter;
use crate::services::snapshot_service::last_closed_market_date_for;
use chrono::NaiveDate;
use tauri::State;

/// Look through weekends and exchange holidays for the baseline close.
const BENCHMARK_BASELINE_LOOKBACK_DAYS: i64 = 14;

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
    let benchmark_market = match symbol.as_str() {
        "^HSI" => "HK",
        "000300.SS" | "000001.SS" => "CN",
        _ => "US",
    };
    let end = parse_date(&end_date)?.min(last_closed_market_date_for(benchmark_market));
    if start >= end {
        return Ok(vec![]);
    }
    let fetch_start = start
        .checked_sub_signed(chrono::Duration::days(BENCHMARK_BASELINE_LOOKBACK_DAYS))
        .unwrap_or(start);
    let points =
        performance_service::fetch_benchmark_history(&db, &symbol, fetch_start, end).await?;
    Ok(benchmark_for_valuation_window(points, start, end))
}

fn benchmark_for_valuation_window(
    mut points: Vec<BenchmarkDataPoint>,
    start: NaiveDate,
    end: NaiveDate,
) -> Vec<ReturnDataPoint> {
    let start = start.to_string();
    let end = end.to_string();
    points.retain(|point| point.date <= end);
    let Some(baseline_index) = points.iter().rposition(|point| point.date <= start) else {
        return vec![];
    };
    if baseline_index + 1 == points.len() {
        return vec![];
    }

    // If this exchange was closed on the portfolio's baseline day, its last
    // earlier close is the as-of price at that same comparison starting point.
    points[baseline_index].date = start;
    performance_service::benchmark_to_return_series(&points[baseline_index..], None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(date: &str, close_price: f64) -> BenchmarkDataPoint {
        BenchmarkDataPoint {
            date: date.to_string(),
            close_price,
            change_percent: 0.0,
        }
    }

    #[test]
    fn comparison_uses_the_last_close_on_or_before_the_portfolio_baseline() {
        let start = parse_date("2026-11-16").unwrap();
        let end = parse_date("2026-11-20").unwrap();
        let points = vec![
            point("2026-11-13", 90.0),
            point("2026-11-16", 100.0),
            point("2026-11-18", 105.0),
            point("2026-11-23", 115.0),
        ];
        let returns = benchmark_for_valuation_window(points, start, end);
        assert_eq!(returns.len(), 2);
        assert_eq!(returns[0].date, "2026-11-16");
        assert_eq!(returns[0].cumulative_return, 0.0);
        assert_eq!(returns[1].date, "2026-11-18");
        assert!((returns[1].cumulative_return - 5.0).abs() < 1e-9);
    }

    #[test]
    fn comparison_can_start_from_an_earlier_close_but_not_a_later_close() {
        let start = parse_date("2026-11-16").unwrap();
        let end = parse_date("2026-11-20").unwrap();
        let after = point("2026-11-17", 105.0);
        let no_baseline = benchmark_for_valuation_window(vec![after.clone()], start, end);
        assert!(no_baseline.is_empty());

        let returns =
            benchmark_for_valuation_window(vec![point("2026-11-13", 100.0), after], start, end);
        assert_eq!(returns[0].date, "2026-11-16");
        assert_eq!(returns[0].cumulative_return, 0.0);
        assert!((returns[1].cumulative_return - 5.0).abs() < 1e-9);
    }
}
