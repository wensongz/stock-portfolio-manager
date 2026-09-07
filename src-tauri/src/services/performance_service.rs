use crate::db::Database;
use crate::models::performance::PerformanceReport;
use chrono::NaiveDate;

// Transfers are external asset flows valued at the latest historical close on
// or before the transfer date, never at their acquisition cost or zero proceeds.
// Keep this expression shared by portfolio returns, attribution and rankings.
pub(super) const TRANSFER_VALUE_SQL: &str = "CASE
    WHEN t.transaction_type IN ('STOCK_IN', 'STOCK_OUT') THEN t.shares * (
        SELECT h.close_price FROM daily_holding_snapshots h
        WHERE h.account_id = t.account_id AND UPPER(h.symbol) = UPPER(t.symbol)
          AND h.market = t.market AND h.date <= DATE(t.traded_at)
          AND h.close_price > 0
        ORDER BY h.date DESC, h.id DESC LIMIT 1)
    ELSE t.total_amount END";

pub(super) fn require_flow_value(value: Option<f64>) -> Result<f64, String> {
    value
        .filter(|value| value.is_finite())
        .ok_or_else(|| "缺少股票存入或提取时的历史估值，请先补齐发生日或此前的持仓快照".to_string())
}

pub(super) const RISK_FREE_RATE: f64 = 0.045; // 4.5% US 10-year treasury default
pub(super) const TRADING_DAYS_PER_YEAR: f64 = 252.0;

mod attribution;
mod benchmark;
mod calculation;
mod ranking;

pub use attribution::get_return_attribution;
#[allow(unused_imports)]
pub use benchmark::{
    benchmark_to_return_series, cache_benchmark_prices, fetch_benchmark_history,
    read_cached_benchmark,
};
#[allow(unused_imports)]
pub use calculation::{
    build_twr_return_series, calculate_sharpe_from_daily_returns, calculate_volatility,
    get_drawdown_analysis, get_monthly_returns, get_performance_summary, get_risk_metrics,
};
pub use ranking::get_holding_performance_ranking;

#[cfg(test)]
mod tests;

/// Optional filters for narrowing performance analysis to a specific market
/// or account.
#[derive(Debug, Clone, Default)]
pub struct PerformanceFilter {
    pub market: Option<String>,
    pub account_id: Option<String>,
}

impl PerformanceFilter {
    pub fn is_active(&self) -> bool {
        self.market.is_some() || self.account_id.is_some()
    }

    /// Append optional WHERE clauses for market/account_id and push
    /// corresponding parameter values. Returns the number of params added.
    pub(super) fn append_where_clauses(
        &self,
        sql: &mut String,
        params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
    ) {
        if let Some(ref market) = self.market {
            sql.push_str(&format!(" AND market = ?{}", params.len() + 1));
            params.push(Box::new(market.clone()));
        }
        if let Some(ref account_id) = self.account_id {
            sql.push_str(&format!(" AND account_id = ?{}", params.len() + 1));
            params.push(Box::new(account_id.clone()));
        }
    }
}

/// Build every performance-page section from one canonical data load.
pub fn get_performance_report(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    ranking_sort_by: &str,
    ranking_limit: usize,
    filter: &PerformanceFilter,
) -> Result<PerformanceReport, String> {
    let started = std::time::Instant::now();
    let calculation = calculation::PerformanceCalculation::load(db, start_date, end_date, filter)?;
    let report = PerformanceReport {
        summary: calculation::performance_summary_from(&calculation, start_date, end_date),
        drawdown: calculation::drawdown_analysis_from(&calculation),
        attribution: attribution::return_attribution_from(db, &calculation, filter)?,
        monthly_returns: calculation::monthly_returns_from(&calculation),
        holding_performances: ranking::holding_performance_ranking_from(
            db,
            &calculation,
            ranking_sort_by,
            ranking_limit,
            filter,
        )?,
        risk_metrics: calculation::risk_metrics_from(&calculation),
    };
    tracing::debug!(
        elapsed_ms = started.elapsed().as_millis(),
        start_date = %start_date,
        end_date = %end_date,
        "built aggregate performance report"
    );
    Ok(report)
}
