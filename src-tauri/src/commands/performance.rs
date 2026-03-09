use crate::db::Database;
use crate::models::performance::{
    DrawdownAnalysis, HoldingPerformance, MonthlyReturn, PerformanceSummary, ReturnAttribution,
    ReturnDataPoint, RiskMetrics,
};
use crate::services::performance_service;
use tauri::State;

fn parse_date(s: &str) -> Result<chrono::NaiveDate, String> {
    crate::models::performance::parse_date(s)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_performance_summary(
    db: State<'_, Database>,
    start_date: String,
    end_date: String,
) -> Result<PerformanceSummary, String> {
    let start = parse_date(&start_date)?;
    let end = parse_date(&end_date)?;
    performance_service::get_performance_summary(&db, start, end)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_return_series(
    db: State<'_, Database>,
    start_date: String,
    end_date: String,
) -> Result<Vec<ReturnDataPoint>, String> {
    let start = parse_date(&start_date)?;
    let end = parse_date(&end_date)?;
    performance_service::get_return_series(&db, start, end)
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
    let fetch_start = start - chrono::Duration::days(10);
    let points =
        performance_service::fetch_benchmark_history(&db, &symbol, fetch_start, end).await?;
    let start_str = start.format("%Y-%m-%d").to_string();
    let base_price = points
        .iter()
        .filter(|p| p.date < start_str)
        .last()
        .map(|p| p.close_price);
    let visible: Vec<_> = points
        .into_iter()
        .filter(|p| p.date >= start_str)
        .collect();
    Ok(performance_service::benchmark_to_return_series(
        &visible, base_price,
    ))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_return_attribution(
    db: State<'_, Database>,
    start_date: String,
    end_date: String,
) -> Result<ReturnAttribution, String> {
    let start = parse_date(&start_date)?;
    let end = parse_date(&end_date)?;
    performance_service::get_return_attribution(&db, start, end)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_monthly_returns(
    db: State<'_, Database>,
    start_date: String,
    end_date: String,
) -> Result<Vec<MonthlyReturn>, String> {
    let start = parse_date(&start_date)?;
    let end = parse_date(&end_date)?;
    performance_service::get_monthly_returns(&db, start, end)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_holding_performance_ranking(
    db: State<'_, Database>,
    start_date: String,
    end_date: String,
    sort_by: String,
    limit: u32,
) -> Result<Vec<HoldingPerformance>, String> {
    let start = parse_date(&start_date)?;
    let end = parse_date(&end_date)?;
    performance_service::get_holding_performance_ranking(
        &db,
        start,
        end,
        &sort_by,
        limit as usize,
    )
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_risk_metrics(
    db: State<'_, Database>,
    start_date: String,
    end_date: String,
) -> Result<RiskMetrics, String> {
    let start = parse_date(&start_date)?;
    let end = parse_date(&end_date)?;
    performance_service::get_risk_metrics(&db, start, end)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_drawdown_analysis(
    db: State<'_, Database>,
    start_date: String,
    end_date: String,
) -> Result<DrawdownAnalysis, String> {
    let start = parse_date(&start_date)?;
    let end = parse_date(&end_date)?;
    let series = performance_service::get_return_series(&db, start, end)?;
    Ok(performance_service::calculate_max_drawdown(&series))
}
