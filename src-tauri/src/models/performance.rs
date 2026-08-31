use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PerformanceSummary {
    pub start_date: String,
    pub end_date: String,
    pub start_value: f64,
    pub end_value: f64,
    pub total_return: f64,
    pub annualized_return: f64,
    pub total_pnl: f64,
    pub max_drawdown: f64,
    pub volatility: f64,
    pub sharpe_ratio: Option<f64>,
    /// The daily return series computed from the same DB query used for
    /// the summary metrics.  Returned here so the frontend can render
    /// both the summary card and the cumulative-return chart from a
    /// single backend call, eliminating redundant DB queries.
    pub return_series: Vec<ReturnDataPoint>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReturnDataPoint {
    pub date: String,
    pub cumulative_return: f64,
    pub daily_return: f64,
    pub portfolio_value: f64,
    pub daily_pnl: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DrawdownPoint {
    pub date: String,
    pub drawdown: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DrawdownAnalysis {
    pub max_drawdown: f64,
    pub peak_date: String,
    pub trough_date: String,
    pub recovery_date: Option<String>,
    pub drawdown_duration: i64,
    pub recovery_duration: Option<i64>,
    pub drawdown_series: Vec<DrawdownPoint>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AttributionItem {
    pub name: String,
    pub pnl: f64,
    pub contribution_percent: f64,
    pub weight: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ReturnAttribution {
    pub total_pnl: f64,
    pub by_market: Vec<AttributionItem>,
    pub by_category: Vec<AttributionItem>,
    pub by_holding: Vec<AttributionItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MonthlyReturn {
    pub year: i32,
    pub month: u32,
    pub return_rate: f64,
    pub pnl: f64,
    pub start_value: f64,
    pub end_value: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HoldingPerformance {
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub category_name: String,
    pub return_rate: f64,
    pub pnl: f64,
    pub start_value: f64,
    pub end_value: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct RiskMetrics {
    pub daily_volatility: f64,
    pub annualized_volatility: f64,
    pub sharpe_ratio: Option<f64>,
    pub risk_free_rate: f64,
    pub max_drawdown: f64,
    pub calmar_ratio: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PerformanceReport {
    pub summary: PerformanceSummary,
    pub drawdown: DrawdownAnalysis,
    pub attribution: ReturnAttribution,
    pub monthly_returns: Vec<MonthlyReturn>,
    pub holding_performances: Vec<HoldingPerformance>,
    pub risk_metrics: RiskMetrics,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct BenchmarkDataPoint {
    pub date: String,
    pub close_price: f64,
    pub change_percent: f64,
}

/// Annualise a TWR over the given number of calendar days.
pub fn annualise_return(twr: f64, days: i64) -> f64 {
    if days <= 0 {
        return 0.0;
    }
    (1.0 + twr).powf(365.0 / days as f64) - 1.0
}

/// Parse a "YYYY-MM-DD" string into a NaiveDate, returning an Err string on failure.
pub fn parse_date(s: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|e| format!("Invalid date '{}': {}", s, e))
}
