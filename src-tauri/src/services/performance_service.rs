use crate::db::Database;
use crate::services::http_client;
use chrono::Datelike;

const RISK_FREE_RATE: f64 = 0.045; // 4.5% US 10-year treasury default
const TRADING_DAYS_PER_YEAR: f64 = 252.0;
const CACHE_COVERAGE_THRESHOLD: f64 = 0.5; // require 50% of expected days in cache to skip re-fetch
use crate::models::performance::{
    annualise_return, AttributionItem, BenchmarkDataPoint, DrawdownAnalysis, DrawdownPoint,
    HoldingPerformance, MonthlyReturn, PerformanceReport, PerformanceSummary, ReturnAttribution,
    ReturnDataPoint, RiskMetrics,
};
use chrono::NaiveDate;
use rusqlite::OptionalExtension;

// ─────────────────────────────────────────────────────────────────────────────
// Internal DB helpers
// ─────────────────────────────────────────────────────────────────────────────

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
    fn append_where_clauses(
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

fn parse_required_exchange_rates(
    json: Option<&str>,
    context: &str,
) -> Result<crate::models::ExchangeRates, String> {
    let json = json.ok_or_else(|| format!("missing exchange rates for {context}"))?;
    let rates = serde_json::from_str::<crate::models::ExchangeRates>(json)
        .map_err(|error| format!("invalid exchange rates for {context}: {error}"))?;
    if [rates.usd_cny, rates.usd_hkd, rates.cny_hkd]
        .iter()
        .all(|rate| rate.is_finite() && *rate > 0.0)
    {
        Ok(rates)
    } else {
        Err(format!(
            "invalid exchange rates for {context}: expected positive finite values"
        ))
    }
}

/// Fetch daily portfolio values (total_value, daily_pnl) for the date range.
fn fetch_daily_values(
    db: &Database,
    start: NaiveDate,
    end: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<Vec<(NaiveDate, f64, f64)>, String> {
    if filter.is_active() {
        return fetch_filtered_daily_values(db, start, end, filter);
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start.format("%Y-%m-%d").to_string();
    let end_str = end.format("%Y-%m-%d").to_string();

    let mut stmt = conn
        .prepare(
            "SELECT date, total_value, daily_pnl
             FROM daily_portfolio_values
             WHERE date BETWEEN ?1 AND ?2
             ORDER BY date ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(rusqlite::params![start_str, end_str], |row| {
            let date_str: String = row.get(0)?;
            let value: f64 = row.get(1)?;
            let dpnl: f64 = row.get(2)?;
            Ok((date_str, value, dpnl))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    rows.into_iter()
        .map(|(ds, v, d)| {
            let date = NaiveDate::parse_from_str(&ds, "%Y-%m-%d")
                .map_err(|e| format!("bad date '{}': {}", ds, e))?;
            Ok((date, v, d))
        })
        .collect()
}

/// Fetch daily values from `daily_holding_snapshots` aggregated by date,
/// filtered by market and/or account_id. Derives daily_pnl from consecutive
/// day value differences.
fn fetch_filtered_daily_values(
    db: &Database,
    start: NaiveDate,
    end: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<Vec<(NaiveDate, f64, f64)>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start.format("%Y-%m-%d").to_string();
    let end_str = end.format("%Y-%m-%d").to_string();

    let mut sql = String::from(
        "SELECT date, SUM(market_value) as total_value
         FROM daily_holding_snapshots
         WHERE date BETWEEN ?1 AND ?2",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(start_str), Box::new(end_str)];

    filter.append_where_clauses(&mut sql, &mut params);

    sql.push_str(" GROUP BY date ORDER BY date ASC");

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            let date_str: String = row.get(0)?;
            let value: f64 = row.get(1)?;
            Ok((date_str, value))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut result = Vec::with_capacity(rows.len());
    let mut prev_value: Option<f64> = None;
    for (ds, v) in rows {
        let date = NaiveDate::parse_from_str(&ds, "%Y-%m-%d")
            .map_err(|e| format!("bad date '{}': {}", ds, e))?;
        let dpnl = prev_value.map(|pv| v - pv).unwrap_or(0.0);
        result.push((date, v, dpnl));
        prev_value = Some(v);
    }
    Ok(result)
}

/// Fetch the portfolio value on the latest day strictly before `date`.
/// Used as the baseline for cumulative-return curves so that the first
/// visible day already shows a non-zero return.
fn fetch_previous_day_value(
    db: &Database,
    date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<Option<(NaiveDate, f64)>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let date_str = date.format("%Y-%m-%d").to_string();
    if filter.is_active() {
        let mut sql = String::from(
            "SELECT date, SUM(market_value) FROM daily_holding_snapshots WHERE date = (SELECT MAX(date) FROM daily_holding_snapshots WHERE date < ?1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(date_str.clone())];
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push(')');
        // Apply same filters to outer WHERE
        filter.append_where_clauses(&mut sql, &mut params);
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        sql.push_str(" GROUP BY date");
        let result = conn
            .query_row(&sql, param_refs.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })
            .optional()
            .map_err(|error| error.to_string())?;
        return result
            .map(|(date, value)| {
                NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                    .map(|parsed| (parsed, value))
                    .map_err(|error| format!("bad date '{date}': {error}"))
            })
            .transpose();
    }
    let result = conn
        .query_row(
            "SELECT date, total_value FROM daily_portfolio_values WHERE date < ?1 ORDER BY date DESC LIMIT 1",
            rusqlite::params![date_str],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    result
        .map(|(date, value)| {
            NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .map(|parsed| (parsed, value))
                .map_err(|error| format!("bad date '{date}': {error}"))
        })
        .transpose()
}

/// Fetch external contributions/withdrawals between two valuations.
/// Cash-symbol BUY records are contributions; SELL records are withdrawals.
/// OPEN records are in-kind position contributions with no account cash impact.
/// Unfiltered portfolio valuations are stored in USD, so their flows are
/// converted with the exchange rates attached to the first valuation on or
/// after each flow date. Filtered valuations remain in their market currency.
fn fetch_external_cash_flows(
    db: &Database,
    start_exclusive: NaiveDate,
    end_inclusive: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<Vec<(NaiveDate, f64)>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start_exclusive.format("%Y-%m-%d").to_string();
    let end_str = end_inclusive.format("%Y-%m-%d").to_string();
    let mut sql = String::from(
        "SELECT DATE(t.traded_at), t.transaction_type, t.total_amount,
                t.commission, t.currency,
                (SELECT d.exchange_rates
                   FROM daily_portfolio_values d
                  WHERE d.date >= DATE(t.traded_at) AND d.date <= ?2
                  ORDER BY d.date ASC LIMIT 1)
           FROM transactions t
          WHERE DATE(t.traded_at) > ?1 AND DATE(t.traded_at) <= ?2
            AND ((UPPER(t.symbol) LIKE '$CASH-%'
                  AND t.transaction_type IN ('BUY', 'SELL'))
                 OR t.transaction_type = 'OPEN')",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(start_str), Box::new(end_str)];
    if let Some(ref account_id) = filter.account_id {
        sql.push_str(&format!(" AND t.account_id = ?{}", params.len() + 1));
        params.push(Box::new(account_id.clone()));
    }
    if let Some(ref market) = filter.market {
        sql.push_str(&format!(" AND t.market = ?{}", params.len() + 1));
        params.push(Box::new(market.clone()));
    }
    sql.push_str(" ORDER BY DATE(t.traded_at) ASC");

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(|param| param.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut grouped = std::collections::BTreeMap::<NaiveDate, f64>::new();
    for (date_str, transaction_type, total_amount, commission, currency, rates_json) in rows {
        let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
            .map_err(|e| format!("bad cash-flow date '{}': {}", date_str, e))?;
        let signed_amount = match transaction_type.as_str() {
            "BUY" => total_amount + commission,
            "SELL" => -(total_amount + commission),
            "OPEN" => total_amount + commission,
            _ => continue,
        };
        let amount = if filter.is_active() || currency == "USD" {
            signed_amount
        } else {
            let context = format!("{} cash flow on {}", currency, date_str);
            let rates = parse_required_exchange_rates(rates_json.as_deref(), &context)?;
            crate::services::exchange_rate_service::convert_currency(
                signed_amount,
                &currency,
                "USD",
                &rates,
            )
        };
        *grouped.entry(date).or_insert(0.0) += amount;
    }

    Ok(grouped.into_iter().collect())
}

struct PerformanceCalculation {
    daily_values: Vec<(NaiveDate, f64, f64)>,
    baseline: Option<(NaiveDate, f64)>,
    external_cash_flows: Vec<(NaiveDate, f64)>,
    return_series: Vec<ReturnDataPoint>,
}

#[cfg(test)]
thread_local! {
    static PERFORMANCE_LOAD_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
fn reset_performance_load_count() {
    PERFORMANCE_LOAD_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
fn performance_load_count() -> usize {
    PERFORMANCE_LOAD_COUNT.with(std::cell::Cell::get)
}

impl PerformanceCalculation {
    fn load(
        db: &Database,
        start_date: NaiveDate,
        end_date: NaiveDate,
        filter: &PerformanceFilter,
    ) -> Result<Self, String> {
        #[cfg(test)]
        PERFORMANCE_LOAD_COUNT.with(|count| count.set(count.get() + 1));

        let daily_values = fetch_daily_values(db, start_date, end_date, filter)?;
        if daily_values.is_empty() {
            return Ok(Self {
                daily_values,
                baseline: None,
                external_cash_flows: vec![],
                return_series: vec![],
            });
        }

        let baseline = fetch_previous_day_value(db, start_date, filter)?;
        let cash_flow_start = baseline.map(|(date, _)| date).unwrap_or(daily_values[0].0);
        let actual_end = daily_values.last().map(|row| row.0).unwrap_or(end_date);
        let external_cash_flows =
            fetch_external_cash_flows(db, cash_flow_start, actual_end, filter)?;
        let return_series = build_twr_return_series(&daily_values, baseline, &external_cash_flows);

        Ok(Self {
            daily_values,
            baseline,
            external_cash_flows,
            return_series,
        })
    }

    fn start_date(&self) -> Option<NaiveDate> {
        self.baseline
            .map(|(date, _)| date)
            .or_else(|| self.daily_values.first().map(|row| row.0))
    }

    fn end_date(&self) -> Option<NaiveDate> {
        self.daily_values.last().map(|row| row.0)
    }

    fn start_value(&self) -> f64 {
        self.baseline
            .map(|(_, value)| value)
            .or_else(|| self.daily_values.first().map(|row| row.1))
            .unwrap_or(0.0)
    }

    fn end_value(&self) -> f64 {
        self.daily_values.last().map(|row| row.1).unwrap_or(0.0)
    }

    fn total_external_cash_flow(&self) -> f64 {
        self.external_cash_flows
            .iter()
            .map(|(_, amount)| *amount)
            .sum()
    }

    fn total_pnl(&self) -> f64 {
        self.end_value() - self.start_value() - self.total_external_cash_flow()
    }

    fn total_return(&self) -> f64 {
        self.return_series
            .last()
            .map(|point| point.cumulative_return / 100.0)
            .unwrap_or(0.0)
    }

    fn calendar_days(&self) -> i64 {
        match (self.start_date(), self.end_date()) {
            (Some(start), Some(end)) => (end - start).num_days(),
            _ => 0,
        }
    }

    fn daily_returns(&self) -> Vec<f64> {
        let skip = usize::from(self.baseline.is_none());
        self.return_series
            .iter()
            .skip(skip)
            .map(|point| point.daily_return / 100.0)
            .collect()
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Core calculations
// ─────────────────────────────────────────────────────────────────────────────

/// Build a cash-flow-adjusted daily TWR series.
///
/// External contributions are positive and withdrawals are negative. Cash
/// flows are treated as end-of-day flows and therefore removed from the
/// change between the previous valuation and the current valuation before the
/// sub-period return is calculated. Sub-period returns are geometrically
/// linked into `cumulative_return`.
pub fn build_twr_return_series(
    daily_values: &[(NaiveDate, f64, f64)],
    baseline: Option<(NaiveDate, f64)>,
    external_cash_flows: &[(NaiveDate, f64)],
) -> Vec<ReturnDataPoint> {
    if daily_values.is_empty() {
        return vec![];
    }

    let mut result = Vec::with_capacity(daily_values.len());
    let mut previous = baseline;
    let mut growth = 1.0f64;

    for (date, value, _raw_daily_pnl) in daily_values {
        let Some((previous_date, previous_value)) = previous else {
            result.push(ReturnDataPoint {
                date: date.format("%Y-%m-%d").to_string(),
                cumulative_return: 0.0,
                daily_return: 0.0,
                portfolio_value: *value,
                daily_pnl: 0.0,
            });
            previous = Some((*date, *value));
            continue;
        };

        let external_flow = external_cash_flows
            .iter()
            .filter(|(flow_date, _)| *flow_date > previous_date && *flow_date <= *date)
            .map(|(_, amount)| *amount)
            .sum::<f64>();
        let period_pnl = *value - previous_value - external_flow;
        let period_return = if previous_value > 0.0 {
            period_pnl / previous_value
        } else {
            0.0
        };
        growth *= 1.0 + period_return;

        result.push(ReturnDataPoint {
            date: date.format("%Y-%m-%d").to_string(),
            cumulative_return: (growth - 1.0) * 100.0,
            daily_return: period_return * 100.0,
            portfolio_value: *value,
            daily_pnl: period_pnl,
        });
        previous = Some((*date, *value));
    }

    result
}

/// Calculate maximum drawdown from a cash-flow-adjusted return series.
fn calculate_max_drawdown(
    return_series: &[ReturnDataPoint],
    baseline_date: Option<NaiveDate>,
) -> DrawdownAnalysis {
    if return_series.is_empty() {
        return DrawdownAnalysis {
            max_drawdown: 0.0,
            peak_date: String::new(),
            trough_date: String::new(),
            recovery_date: None,
            drawdown_duration: 0,
            recovery_duration: None,
            drawdown_series: vec![],
        };
    }

    // Drawdown measures the decline of investment wealth, not the decline of
    // raw account value.  The latter is distorted by deposits and withdrawals;
    // cumulative TWR is already neutral to those external cash flows.
    let values: Vec<f64> = return_series
        .iter()
        .map(|r| 1.0 + r.cumulative_return / 100.0)
        .collect();
    let dates: Vec<&str> = return_series.iter().map(|r| r.date.as_str()).collect();

    // Cumulative TWR is measured from a wealth index of 1.0 immediately
    // before the first visible observation, so an initial loss is itself a
    // drawdown and must not become the starting peak.
    let mut peak = 1.0f64;
    let mut peak_idx: Option<usize> = None;
    let mut max_drawdown = 0.0f64;
    let mut md_peak_value = peak;
    let mut md_peak_date = baseline_date
        .map(|date| date.format("%Y-%m-%d").to_string())
        .unwrap_or_else(|| dates[0].to_string());
    let mut md_trough_idx = 0usize;

    let mut drawdown_series = Vec::with_capacity(values.len());

    for (i, &v) in values.iter().enumerate() {
        if v > peak {
            peak = v;
            peak_idx = Some(i);
        }
        let dd = if peak > 0.0 { (v - peak) / peak } else { 0.0 };
        drawdown_series.push(DrawdownPoint {
            date: dates[i].to_string(),
            drawdown: dd * 100.0,
        });
        if dd < max_drawdown {
            max_drawdown = dd;
            md_peak_value = peak;
            md_peak_date = peak_idx
                .map(|index| dates[index].to_string())
                .or_else(|| baseline_date.map(|date| date.format("%Y-%m-%d").to_string()))
                .unwrap_or_else(|| dates[0].to_string());
            md_trough_idx = i;
        }
    }

    // Find recovery date: first date after trough where value >= peak at trough time
    let recovery_idx = (max_drawdown < 0.0)
        .then(|| {
            values[md_trough_idx..]
                .iter()
                .position(|&v| v >= md_peak_value)
                .map(|offset| md_trough_idx + offset)
        })
        .flatten();

    let peak_date_str = md_peak_date;
    let trough_date_str = dates[md_trough_idx].to_string();

    let drawdown_duration = if let (Ok(pd), Ok(td)) = (
        NaiveDate::parse_from_str(&peak_date_str, "%Y-%m-%d"),
        NaiveDate::parse_from_str(&trough_date_str, "%Y-%m-%d"),
    ) {
        (td - pd).num_days()
    } else {
        0
    };

    let recovery_date = recovery_idx.map(|ri| dates[ri].to_string());
    let recovery_duration = recovery_date.as_deref().and_then(|rd| {
        let td = NaiveDate::parse_from_str(&trough_date_str, "%Y-%m-%d").ok()?;
        let rdate = NaiveDate::parse_from_str(rd, "%Y-%m-%d").ok()?;
        Some((rdate - td).num_days())
    });

    DrawdownAnalysis {
        max_drawdown: max_drawdown * 100.0,
        peak_date: peak_date_str,
        trough_date: trough_date_str,
        recovery_date,
        drawdown_duration,
        recovery_duration,
        drawdown_series,
    }
}

/// Calculate annualised volatility from daily return percentages.
pub fn calculate_volatility(daily_returns: &[f64]) -> (f64, f64) {
    let n = daily_returns.len();
    if n < 2 {
        return (0.0, 0.0);
    }
    let mean = daily_returns.iter().sum::<f64>() / n as f64;
    let variance = daily_returns
        .iter()
        .map(|r| (r - mean).powi(2))
        .sum::<f64>()
        / (n - 1) as f64;
    let daily_vol = variance.sqrt();
    let annualised_vol = daily_vol * TRADING_DAYS_PER_YEAR.sqrt();
    (daily_vol, annualised_vol)
}

/// Calculate the annualised ex-post Sharpe ratio from periodic daily returns.
/// Returns `None` when fewer than two observations exist or their sample
/// standard deviation is zero, because the ratio is then undefined.
pub fn calculate_sharpe_from_daily_returns(
    daily_returns: &[f64],
    annual_risk_free_rate: f64,
) -> Option<f64> {
    if daily_returns.len() < 2 || annual_risk_free_rate <= -1.0 {
        return None;
    }

    let mean_daily_return = daily_returns.iter().sum::<f64>() / daily_returns.len() as f64;
    let (daily_volatility, _) = calculate_volatility(daily_returns);
    if daily_volatility <= f64::EPSILON {
        return None;
    }

    let daily_risk_free_rate =
        (1.0 + annual_risk_free_rate).powf(1.0 / TRADING_DAYS_PER_YEAR) - 1.0;
    Some(
        (mean_daily_return - daily_risk_free_rate) / daily_volatility
            * TRADING_DAYS_PER_YEAR.sqrt(),
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Public service functions called from commands
// ─────────────────────────────────────────────────────────────────────────────

pub fn get_drawdown_analysis(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<DrawdownAnalysis, String> {
    let calculation = PerformanceCalculation::load(db, start_date, end_date, filter)?;
    Ok(drawdown_analysis_from(&calculation))
}

fn drawdown_analysis_from(calculation: &PerformanceCalculation) -> DrawdownAnalysis {
    calculate_max_drawdown(
        &calculation.return_series,
        calculation.baseline.map(|(date, _)| date),
    )
}

pub fn get_performance_summary(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<PerformanceSummary, String> {
    let calculation = PerformanceCalculation::load(db, start_date, end_date, filter)?;
    Ok(performance_summary_from(&calculation, start_date, end_date))
}

fn performance_summary_from(
    calculation: &PerformanceCalculation,
    requested_start_date: NaiveDate,
    requested_end_date: NaiveDate,
) -> PerformanceSummary {
    if calculation.daily_values.is_empty() {
        return PerformanceSummary {
            start_date: requested_start_date.format("%Y-%m-%d").to_string(),
            end_date: requested_end_date.format("%Y-%m-%d").to_string(),
            start_value: 0.0,
            end_value: 0.0,
            total_return: 0.0,
            annualized_return: 0.0,
            total_pnl: 0.0,
            max_drawdown: 0.0,
            volatility: 0.0,
            sharpe_ratio: None,
            return_series: vec![],
        };
    }

    let start_value = calculation.start_value();
    let end_value = calculation.end_value();
    let total_pnl = calculation.total_pnl();
    let total_return = calculation.total_return();
    let total_return_pct = total_return * 100.0;
    let days = calculation.calendar_days();
    let actual_start_date = calculation.start_date().unwrap();
    let actual_end_date = calculation.end_date().unwrap();
    let annualised = annualise_return(total_return, days);
    let dd_analysis = calculate_max_drawdown(
        &calculation.return_series,
        calculation.baseline.map(|(date, _)| date),
    );
    let daily_returns = calculation.daily_returns();
    let (_daily_vol, ann_vol) = calculate_volatility(&daily_returns);
    let sharpe = calculate_sharpe_from_daily_returns(&daily_returns, RISK_FREE_RATE);

    PerformanceSummary {
        start_date: actual_start_date.format("%Y-%m-%d").to_string(),
        end_date: actual_end_date.format("%Y-%m-%d").to_string(),
        start_value,
        end_value,
        total_return: total_return_pct,
        annualized_return: annualised * 100.0,
        total_pnl,
        max_drawdown: dd_analysis.max_drawdown,
        volatility: ann_vol * 100.0,
        sharpe_ratio: sharpe,
        return_series: calculation.return_series.clone(),
    }
}

pub fn get_risk_metrics(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<RiskMetrics, String> {
    let calculation = PerformanceCalculation::load(db, start_date, end_date, filter)?;
    Ok(risk_metrics_from(&calculation))
}

fn risk_metrics_from(calculation: &PerformanceCalculation) -> RiskMetrics {
    if calculation.daily_values.is_empty() {
        return RiskMetrics {
            daily_volatility: 0.0,
            annualized_volatility: 0.0,
            sharpe_ratio: None,
            risk_free_rate: 4.5,
            max_drawdown: 0.0,
            calmar_ratio: None,
        };
    }

    let total_return = calculation.total_return();
    let days = calculation.calendar_days();
    let annualised = annualise_return(total_return, days);
    let daily_returns = calculation.daily_returns();
    let (daily_vol, ann_vol) = calculate_volatility(&daily_returns);
    let sharpe = calculate_sharpe_from_daily_returns(&daily_returns, RISK_FREE_RATE);
    let dd_analysis = calculate_max_drawdown(
        &calculation.return_series,
        calculation.baseline.map(|(date, _)| date),
    );
    let max_dd = dd_analysis.max_drawdown.abs() / 100.0;
    let calmar = if max_dd > 0.0 {
        Some(annualised / max_dd)
    } else {
        None
    };

    RiskMetrics {
        daily_volatility: daily_vol * 100.0,
        annualized_volatility: ann_vol * 100.0,
        sharpe_ratio: sharpe,
        risk_free_rate: RISK_FREE_RATE * 100.0,
        max_drawdown: dd_analysis.max_drawdown,
        calmar_ratio: calmar,
    }
}

pub fn get_return_attribution(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<ReturnAttribution, String> {
    let calculation = PerformanceCalculation::load(db, start_date, end_date, filter)?;
    return_attribution_from(db, &calculation, filter)
}

fn return_attribution_from(
    db: &Database,
    calculation: &PerformanceCalculation,
    filter: &PerformanceFilter,
) -> Result<ReturnAttribution, String> {
    if calculation.daily_values.is_empty() {
        return Ok(ReturnAttribution {
            total_pnl: 0.0,
            by_market: vec![],
            by_category: vec![],
            by_holding: vec![],
        });
    }
    let actual_start_date = calculation.start_date().unwrap();
    let actual_end_date = calculation.end_date().unwrap();
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = actual_start_date.format("%Y-%m-%d").to_string();
    let end_str = actual_end_date.format("%Y-%m-%d").to_string();
    let normalize_to_usd = !filter.is_active();

    let rates_for_date = |date: &str| -> Result<crate::models::ExchangeRates, String> {
        let json = conn
            .query_row(
                "SELECT exchange_rates FROM daily_portfolio_values WHERE date = ?1",
                rusqlite::params![date],
                |row| row.get::<_, String>(0),
            )
            .map_err(|e| format!("missing exchange rates for valuation {}: {}", date, e))?;
        serde_json::from_str::<crate::models::ExchangeRates>(&json)
            .map_err(|e| format!("invalid exchange rates for valuation {}: {}", date, e))
    };
    let start_rates = normalize_to_usd
        .then(|| rates_for_date(&start_str))
        .transpose()?;
    let end_rates = normalize_to_usd
        .then(|| rates_for_date(&end_str))
        .transpose()?;
    let normalize_value =
        |value: f64, market: &str, rates: Option<&crate::models::ExchangeRates>| -> f64 {
            let Some(rates) = rates else {
                return value;
            };
            let currency = match market {
                "CN" => "CNY",
                "HK" => "HKD",
                _ => "USD",
            };
            crate::services::exchange_rate_service::convert_currency(value, currency, "USD", rates)
        };

    type PositionKey = (String, String, String, String); // account, symbol, market, category
    type AccountSymbol = (String, String);

    // Canonical current metadata keeps one account's position separate from
    // the same ticker in another account, and gives transaction-only positions
    // a stable market/category label.
    let holding_metadata: std::collections::HashMap<AccountSymbol, (String, String, String)> = {
        let mut name_stmt = conn
            .prepare(
                "SELECT h.account_id, h.symbol, h.name, h.market,
                        COALESCE(c.name, '未分类')
                   FROM holdings h
                   LEFT JOIN categories c ON h.category_id = c.id",
            )
            .map_err(|e| e.to_string())?;
        let rows = name_stmt
            .query_map([], |row| {
                Ok((
                    (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                    (
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ),
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows.into_iter().collect()
    };

    let position_key = |account_id: &str, symbol: &str, market: &str, category: &str| {
        let metadata_key = (account_id.to_string(), symbol.to_string());
        let (resolved_market, resolved_category) = holding_metadata
            .get(&metadata_key)
            .map(|(_, holding_market, holding_category)| {
                (holding_market.clone(), holding_category.clone())
            })
            .unwrap_or_else(|| (market.to_string(), category.to_string()));
        (
            account_id.to_string(),
            symbol.to_string(),
            resolved_market,
            resolved_category,
        )
    };

    // Get endpoint snapshots per account-position. Values must be normalized
    // before any cross-market aggregation.
    let mut start_vals: std::collections::HashMap<PositionKey, f64> =
        std::collections::HashMap::new();
    let mut end_vals: std::collections::HashMap<PositionKey, f64> =
        std::collections::HashMap::new();

    {
        // Build start query with filters applied to both subquery and outer query
        let mut sql = String::from(
            "SELECT account_id, symbol, market, COALESCE(category_name, '未分类'),
                    SUM(market_value)
             FROM daily_holding_snapshots
             WHERE date = (
                 SELECT MAX(date) FROM daily_holding_snapshots WHERE date <= ?1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(start_str.clone())];
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push(')');
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push_str(" GROUP BY account_id, symbol, market, category_name");
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, f64>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (account_id, symbol, market, category, val) in rows {
            let key = position_key(&account_id, &symbol, &market, &category);
            let val = normalize_value(val, &market, start_rates.as_ref());
            *start_vals.entry(key).or_insert(0.0) += val;
        }
    }

    {
        // Build end query with filters applied to both subquery and outer query
        let mut sql = String::from(
            "SELECT account_id, symbol, market, COALESCE(category_name, '未分类'),
                    SUM(market_value)
             FROM daily_holding_snapshots
             WHERE date = (
                 SELECT MAX(date) FROM daily_holding_snapshots WHERE date <= ?1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(end_str.clone())];
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push(')');
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push_str(" GROUP BY account_id, symbol, market, category_name");
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, f64>(4)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (account_id, symbol, market, category, val) in rows {
            let key = position_key(&account_id, &symbol, &market, &category);
            let val = normalize_value(val, &market, end_rates.as_ref());
            *end_vals.entry(key).or_insert(0.0) += val;
        }
    }

    // Fetch net cash flows per symbol from transactions during the period.
    // BUY  → positive cash flow (money invested into the holding)
    // SELL → negative cash flow (money withdrawn from the holding)
    let mut net_cash_flows: std::collections::HashMap<PositionKey, f64> =
        std::collections::HashMap::new();
    {
        let mut sql = String::from(
            "SELECT t.account_id, t.symbol, t.market, t.transaction_type,
                    t.total_amount, t.commission, t.currency,
                    (SELECT d.exchange_rates
                       FROM daily_portfolio_values d
                      WHERE d.date >= DATE(t.traded_at) AND d.date <= ?2
                      ORDER BY d.date ASC LIMIT 1)
             FROM transactions t
             WHERE DATE(t.traded_at) > ?1 AND DATE(t.traded_at) <= ?2",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(start_str.clone()), Box::new(end_str.clone())];
        if let Some(ref account_id) = filter.account_id {
            sql.push_str(&format!(" AND t.account_id = ?{}", params.len() + 1));
            params.push(Box::new(account_id.clone()));
        }
        if let Some(ref market) = filter.market {
            sql.push_str(&format!(" AND t.market = ?{}", params.len() + 1));
            params.push(Box::new(market.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (account_id, symbol, market, tx_type, amount, commission, currency, rates_json) in rows
        {
            let native_flow = match tx_type.as_str() {
                "BUY" => amount + commission,
                "SELL" | "PAY" => -(amount - commission),
                "OPEN" => amount + commission,
                _ => continue,
            };
            let flow = if normalize_to_usd && currency != "USD" {
                let context = format!("{} transaction", currency);
                let rates = parse_required_exchange_rates(rates_json.as_deref(), &context)?;
                crate::services::exchange_rate_service::convert_currency(
                    native_flow,
                    &currency,
                    "USD",
                    &rates,
                )
            } else {
                native_flow
            };
            let mut key = position_key(&account_id, &symbol, &market, "未分类");
            if !start_vals.contains_key(&key) && !end_vals.contains_key(&key) {
                if let Some(endpoint_key) = start_vals.keys().chain(end_vals.keys()).find(
                    |(position_account, position_symbol, position_market, _)| {
                        position_account == &account_id
                            && position_symbol == &symbol
                            && position_market == &market
                    },
                ) {
                    key = endpoint_key.clone();
                }
            }
            *net_cash_flows.entry(key).or_insert(0.0) += flow;
        }
    }

    let all_positions: std::collections::HashSet<PositionKey> = start_vals
        .keys()
        .chain(end_vals.keys())
        .chain(net_cash_flows.keys())
        .cloned()
        .collect();

    let mut total_pnl = 0.0f64;
    let mut total_start_val = 0.0f64;
    let mut market_pnl: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut category_pnl: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut holding_pnl: std::collections::HashMap<(String, String), (String, f64, f64)> =
        std::collections::HashMap::new(); // (symbol, market) -> (display name, pnl, start value)

    for position in &all_positions {
        let (account_id, symbol, market, category) = position;
        // Skip cash symbols ($CASH-CNY, $CASH-USD, $CASH-HKD) from attribution.
        // Cash holdings don't have entries in the transactions table, so their
        // PnL = ev − sv reflects the cash flow from buying/selling stocks, NOT
        // actual investment returns. Including them double-counts the trade
        // amounts that are already subtracted from individual stock PnLs.
        if crate::services::quote_service::is_cash_symbol(symbol) {
            continue;
        }

        let sv = start_vals.get(position).copied().unwrap_or(0.0);
        let ev = end_vals.get(position).copied().unwrap_or(0.0);
        // Actual PnL = (end_value - start_value) - net_cash_flow
        // net_cash_flow: positive for buys (money in), negative for sells (money out)
        let cf = net_cash_flows.get(position).copied().unwrap_or(0.0);
        let pnl = ev - sv - cf;

        total_pnl += pnl;
        total_start_val += sv;
        *market_pnl.entry(market.clone()).or_insert(0.0) += pnl;
        *category_pnl.entry(category.clone()).or_insert(0.0) += pnl;
        let display_name = holding_metadata
            .get(&(account_id.clone(), symbol.clone()))
            .map(|(name, _, _)| name)
            .filter(|name| !name.is_empty() && name.as_str() != symbol)
            .map(|name| format!("{} {}", symbol, name))
            .unwrap_or_else(|| symbol.clone());
        holding_pnl
            .entry((symbol.clone(), market.clone()))
            .and_modify(|e| {
                e.1 += pnl;
                e.2 += sv;
            })
            .or_insert((display_name, pnl, sv));
    }

    let make_items = |map: std::collections::HashMap<String, f64>| -> Vec<AttributionItem> {
        let mut items: Vec<AttributionItem> = map
            .into_iter()
            .map(|(name, pnl)| {
                let contribution_percent = if total_pnl != 0.0 {
                    pnl / total_pnl.abs() * 100.0
                } else {
                    0.0
                };
                let weight = if total_start_val != 0.0 {
                    pnl / total_start_val * 100.0
                } else {
                    0.0
                };
                AttributionItem {
                    name,
                    pnl,
                    contribution_percent,
                    weight,
                }
            })
            .collect();
        items.sort_by(|a, b| {
            b.pnl
                .partial_cmp(&a.pnl)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        items
    };

    let market_label = |m: &str| match m {
        "US" => "🇺🇸 美股".to_string(),
        "CN" => "🇨🇳 A股".to_string(),
        "HK" => "🇭🇰 港股".to_string(),
        _ => m.to_string(),
    };
    let by_market = make_items(
        market_pnl
            .into_iter()
            .map(|(k, v)| (market_label(&k), v))
            .collect(),
    );
    let by_category = make_items(category_pnl);

    let mut by_holding: Vec<AttributionItem> = holding_pnl
        .into_iter()
        .map(|((_symbol, _market), (display_name, pnl, sv))| {
            let contribution_percent = if total_pnl != 0.0 {
                pnl / total_pnl.abs() * 100.0
            } else {
                0.0
            };
            let weight = if total_start_val != 0.0 {
                sv / total_start_val * 100.0
            } else {
                0.0
            };
            AttributionItem {
                name: display_name,
                pnl,
                contribution_percent,
                weight,
            }
        })
        .collect();
    by_holding.sort_by(|a, b| {
        b.pnl
            .partial_cmp(&a.pnl)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    Ok(ReturnAttribution {
        total_pnl,
        by_market,
        by_category,
        by_holding,
    })
}

pub fn get_monthly_returns(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<Vec<MonthlyReturn>, String> {
    let calculation = PerformanceCalculation::load(db, start_date, end_date, filter)?;
    Ok(monthly_returns_from(&calculation))
}

fn monthly_returns_from(calculation: &PerformanceCalculation) -> Vec<MonthlyReturn> {
    if calculation.daily_values.is_empty() {
        return vec![];
    }

    // Group the already cash-flow-adjusted daily returns by calendar month.
    // Linking those sub-period returns preserves TWR across month boundaries.
    let mut months: std::collections::BTreeMap<(i32, u32), (NaiveDate, f64, f64)> =
        std::collections::BTreeMap::new();
    for ((date, value, _), point) in calculation
        .daily_values
        .iter()
        .zip(calculation.return_series.iter())
    {
        let key = (date.year(), date.month());
        months
            .entry(key)
            .and_modify(|e| {
                if *date > e.0 {
                    e.0 = *date;
                    e.1 = *value;
                }
                e.2 *= 1.0 + point.daily_return / 100.0;
            })
            .or_insert((*date, *value, 1.0 + point.daily_return / 100.0));
    }

    let mut result = Vec::with_capacity(months.len());
    let mut period_start_date = calculation.start_date().unwrap();
    let mut period_start_value = calculation.start_value();
    for ((year, month), (month_end_date, month_end_value, growth)) in months {
        let external_flow = calculation
            .external_cash_flows
            .iter()
            .filter(|(date, _)| *date > period_start_date && *date <= month_end_date)
            .map(|(_, amount)| *amount)
            .sum::<f64>();

        result.push(MonthlyReturn {
            year,
            month,
            return_rate: (growth - 1.0) * 100.0,
            pnl: month_end_value - period_start_value - external_flow,
            start_value: period_start_value,
            end_value: month_end_value,
        });
        period_start_date = month_end_date;
        period_start_value = month_end_value;
    }

    result
}

pub fn get_holding_performance_ranking(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    sort_by: &str,
    limit: usize,
    filter: &PerformanceFilter,
) -> Result<Vec<HoldingPerformance>, String> {
    let calculation = PerformanceCalculation::load(db, start_date, end_date, filter)?;
    holding_performance_ranking_from(db, &calculation, sort_by, limit, filter)
}

fn holding_performance_ranking_from(
    db: &Database,
    calculation: &PerformanceCalculation,
    sort_by: &str,
    limit: usize,
    filter: &PerformanceFilter,
) -> Result<Vec<HoldingPerformance>, String> {
    if calculation.daily_values.is_empty() {
        return Ok(vec![]);
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = calculation
        .start_date()
        .unwrap()
        .format("%Y-%m-%d")
        .to_string();
    let end_str = calculation
        .end_date()
        .unwrap()
        .format("%Y-%m-%d")
        .to_string();
    let normalize_to_usd = !filter.is_active();

    let rates_for_date = |date: &str| -> Result<crate::models::ExchangeRates, String> {
        let json = conn
            .query_row(
                "SELECT exchange_rates FROM daily_portfolio_values WHERE date = ?1",
                rusqlite::params![date],
                |row| row.get::<_, String>(0),
            )
            .map_err(|e| format!("missing exchange rates for valuation {}: {}", date, e))?;
        serde_json::from_str::<crate::models::ExchangeRates>(&json)
            .map_err(|e| format!("invalid exchange rates for valuation {}: {}", date, e))
    };
    let start_rates = normalize_to_usd
        .then(|| rates_for_date(&start_str))
        .transpose()?;
    let end_rates = normalize_to_usd
        .then(|| rates_for_date(&end_str))
        .transpose()?;
    let normalize_value =
        |value: f64, market: &str, rates: Option<&crate::models::ExchangeRates>| -> f64 {
            let Some(rates) = rates else {
                return value;
            };
            let currency = match market {
                "CN" => "CNY",
                "HK" => "HKD",
                _ => "USD",
            };
            crate::services::exchange_rate_service::convert_currency(value, currency, "USD", rates)
        };

    type AccountSymbol = (String, String);
    type PositionKey = (String, String, String); // account, symbol, market
    let holding_metadata: std::collections::HashMap<AccountSymbol, (String, String, String)> = {
        let mut stmt = conn
            .prepare(
                "SELECT h.account_id, h.symbol, h.name, h.market,
                        COALESCE(c.name, '未分类')
                   FROM holdings h
                   LEFT JOIN categories c ON h.category_id = c.id",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |row| {
                Ok((
                    (row.get::<_, String>(0)?, row.get::<_, String>(1)?),
                    (
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                    ),
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        rows.into_iter().collect()
    };

    let position_key = |account_id: &str, symbol: &str, market: &str| {
        let resolved_market = holding_metadata
            .get(&(account_id.to_string(), symbol.to_string()))
            .map(|(_, holding_market, _)| holding_market.clone())
            .unwrap_or_else(|| market.to_string());
        (account_id.to_string(), symbol.to_string(), resolved_market)
    };

    // Collect per-account-position endpoint values so duplicate tickers are
    // normalized independently before later aggregation.
    struct SnapRow {
        account_id: String,
        symbol: String,
        market: String,
        category_name: String,
        market_value: f64,
    }

    let fetch_snap = |date_param: &str| -> Result<Vec<SnapRow>, String> {
        let mut sql = String::from(
            "SELECT account_id, symbol, market, COALESCE(category_name, '未分类'),
                    SUM(market_value)
             FROM daily_holding_snapshots
             WHERE date = (
                 SELECT MAX(date) FROM daily_holding_snapshots WHERE date <= ?1",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(date_param.to_string())];
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push(')');
        filter.append_where_clauses(&mut sql, &mut params);
        sql.push_str(" GROUP BY account_id, symbol, market, category_name");
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok(SnapRow {
                    account_id: row.get(0)?,
                    symbol: row.get(1)?,
                    market: row.get(2)?,
                    category_name: row.get(3)?,
                    market_value: row.get(4)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    };

    let start_snaps = fetch_snap(&start_str)?;
    let end_snaps = fetch_snap(&end_str)?;

    let mut start_map: std::collections::HashMap<PositionKey, (String, f64)> =
        std::collections::HashMap::new();
    for snapshot in start_snaps {
        let key = position_key(&snapshot.account_id, &snapshot.symbol, &snapshot.market);
        let value = normalize_value(
            snapshot.market_value,
            &snapshot.market,
            start_rates.as_ref(),
        );
        let entry = start_map
            .entry(key)
            .or_insert((snapshot.category_name, 0.0));
        entry.1 += value;
    }

    let mut end_map: std::collections::HashMap<PositionKey, (String, f64)> =
        std::collections::HashMap::new();
    for snapshot in end_snaps {
        let key = position_key(&snapshot.account_id, &snapshot.symbol, &snapshot.market);
        let value = normalize_value(snapshot.market_value, &snapshot.market, end_rates.as_ref());
        let entry = end_map.entry(key).or_insert((snapshot.category_name, 0.0));
        entry.1 += value;
    }

    // Fetch position flows individually because transaction-date FX rates can
    // differ. This also prevents native CNY/HKD/USD values from being compared.
    let mut net_cash_flows: std::collections::HashMap<PositionKey, f64> =
        std::collections::HashMap::new();
    let mut gross_contributions: std::collections::HashMap<PositionKey, f64> =
        std::collections::HashMap::new();
    {
        let mut sql = String::from(
            "SELECT t.account_id, t.symbol, t.market, t.transaction_type,
                    t.total_amount, t.commission, t.currency,
                    (SELECT d.exchange_rates
                       FROM daily_portfolio_values d
                      WHERE d.date >= DATE(t.traded_at) AND d.date <= ?2
                      ORDER BY d.date ASC LIMIT 1)
             FROM transactions t
             WHERE DATE(t.traded_at) > ?1 AND DATE(t.traded_at) <= ?2",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
            vec![Box::new(start_str.clone()), Box::new(end_str.clone())];
        if let Some(ref account_id) = filter.account_id {
            sql.push_str(&format!(" AND t.account_id = ?{}", params.len() + 1));
            params.push(Box::new(account_id.clone()));
        }
        if let Some(ref market) = filter.market {
            sql.push_str(&format!(" AND t.market = ?{}", params.len() + 1));
            params.push(Box::new(market.clone()));
        }
        let param_refs: Vec<&dyn rusqlite::types::ToSql> =
            params.iter().map(|p| p.as_ref()).collect();
        let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(param_refs.as_slice(), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, Option<String>>(7)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (account_id, symbol, market, tx_type, amount, commission, currency, rates_json) in rows
        {
            let native_flow = match tx_type.as_str() {
                "BUY" => amount + commission,
                "SELL" | "PAY" => -(amount - commission),
                "OPEN" => amount + commission,
                _ => continue,
            };
            let flow = if normalize_to_usd && currency != "USD" {
                let context = format!("{} transaction", currency);
                let rates = parse_required_exchange_rates(rates_json.as_deref(), &context)?;
                crate::services::exchange_rate_service::convert_currency(
                    native_flow,
                    &currency,
                    "USD",
                    &rates,
                )
            } else {
                native_flow
            };
            let key = position_key(&account_id, &symbol, &market);
            if flow > 0.0 {
                *gross_contributions.entry(key.clone()).or_insert(0.0) += flow;
            }
            *net_cash_flows.entry(key).or_insert(0.0) += flow;
        }
    }

    let all_positions: std::collections::HashSet<PositionKey> = start_map
        .keys()
        .chain(end_map.keys())
        .chain(net_cash_flows.keys())
        .cloned()
        .collect();

    struct AggregatedPerformance {
        symbol: String,
        name: String,
        market: String,
        category_name: String,
        pnl: f64,
        start_value: f64,
        end_value: f64,
        cost_base: f64,
    }
    let mut aggregated: std::collections::HashMap<(String, String), AggregatedPerformance> =
        std::collections::HashMap::new();
    for position in all_positions
        .into_iter()
        .filter(|(_, symbol, _)| !crate::services::quote_service::is_cash_symbol(symbol))
    {
        let (account_id, symbol, market) = &position;
        let metadata = holding_metadata.get(&(account_id.clone(), symbol.clone()));
        let name = metadata
            .map(|(name, _, _)| name.clone())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| symbol.clone());
        let category = metadata
            .map(|(_, _, category)| category.clone())
            .or_else(|| {
                start_map
                    .get(&position)
                    .or_else(|| end_map.get(&position))
                    .map(|(category, _)| category.clone())
            })
            .unwrap_or_else(|| "未分类".to_string());
        let start_value = start_map
            .get(&position)
            .map(|(_, value)| *value)
            .unwrap_or(0.0);
        let end_value = end_map
            .get(&position)
            .map(|(_, value)| *value)
            .unwrap_or(0.0);
        let flow = net_cash_flows.get(&position).copied().unwrap_or(0.0);
        let pnl = end_value - start_value - flow;
        let cost_base = start_value + gross_contributions.get(&position).copied().unwrap_or(0.0);
        let aggregate_key = (symbol.clone(), market.clone());
        aggregated
            .entry(aggregate_key)
            .and_modify(|entry| {
                entry.pnl += pnl;
                entry.start_value += start_value;
                entry.end_value += end_value;
                entry.cost_base += cost_base;
                if entry.category_name != category {
                    entry.category_name = "多类别".to_string();
                }
            })
            .or_insert(AggregatedPerformance {
                symbol: symbol.clone(),
                name,
                market: market.clone(),
                category_name: category,
                pnl,
                start_value,
                end_value,
                cost_base,
            });
    }

    let mut performances: Vec<HoldingPerformance> = aggregated
        .into_values()
        .map(|entry| HoldingPerformance {
            symbol: entry.symbol,
            name: entry.name,
            market: entry.market,
            category_name: entry.category_name,
            return_rate: if entry.cost_base > 0.0 {
                entry.pnl / entry.cost_base * 100.0
            } else {
                0.0
            },
            pnl: entry.pnl,
            start_value: entry.start_value,
            end_value: entry.end_value,
        })
        .collect();

    // Sort
    if sort_by == "pnl" {
        performances.sort_by(|a, b| {
            b.pnl
                .partial_cmp(&a.pnl)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        performances.sort_by(|a, b| {
            b.return_rate
                .partial_cmp(&a.return_rate)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    Ok(performances.into_iter().take(limit).collect())
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
    let calculation = PerformanceCalculation::load(db, start_date, end_date, filter)?;
    let report = PerformanceReport {
        summary: performance_summary_from(&calculation, start_date, end_date),
        drawdown: drawdown_analysis_from(&calculation),
        attribution: return_attribution_from(db, &calculation, filter)?,
        monthly_returns: monthly_returns_from(&calculation),
        holding_performances: holding_performance_ranking_from(
            db,
            &calculation,
            ranking_sort_by,
            ranking_limit,
            filter,
        )?,
        risk_metrics: risk_metrics_from(&calculation),
    };
    tracing::debug!(
        elapsed_ms = started.elapsed().as_millis(),
        start_date = %start_date,
        end_date = %end_date,
        "built aggregate performance report"
    );
    Ok(report)
}

// ─────────────────────────────────────────────────────────────────────────────
// Benchmark data
// ─────────────────────────────────────────────────────────────────────────────

/// Cache benchmark data in SQLite.
pub fn cache_benchmark_prices(
    db: &Database,
    symbol: &str,
    points: &[BenchmarkDataPoint],
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    for p in points {
        conn.execute(
            "INSERT OR REPLACE INTO benchmark_daily_prices (symbol, date, close_price, change_percent)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![symbol, p.date, p.close_price, p.change_percent],
        )
        .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Read cached benchmark prices from SQLite.
pub fn read_cached_benchmark(
    db: &Database,
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<BenchmarkDataPoint>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start_date.format("%Y-%m-%d").to_string();
    let end_str = end_date.format("%Y-%m-%d").to_string();

    let mut stmt = conn
        .prepare(
            "SELECT date, close_price, change_percent
             FROM benchmark_daily_prices
             WHERE symbol = ?1 AND date BETWEEN ?2 AND ?3
             ORDER BY date ASC",
        )
        .map_err(|e| e.to_string())?;

    let rows = stmt
        .query_map(rusqlite::params![symbol, start_str, end_str], |row| {
            Ok(BenchmarkDataPoint {
                date: row.get(0)?,
                close_price: row.get(1)?,
                change_percent: row.get(2)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(rows)
}

/// Fetch benchmark history from Yahoo Finance and cache it.
pub async fn fetch_benchmark_history(
    db: &Database,
    symbol: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<BenchmarkDataPoint>, String> {
    // Check cache first
    let cached = read_cached_benchmark(db, symbol, start_date, end_date)?;

    // If we have data covering the range, use it
    let days_needed = (end_date - start_date).num_days();
    if (cached.len() as f64) >= days_needed as f64 * CACHE_COVERAGE_THRESHOLD {
        return Ok(cached);
    }

    // Fetch from Yahoo Finance
    let start_ts = start_date
        .and_hms_opt(0, 0, 0)
        .unwrap()
        .and_utc()
        .timestamp();
    let end_ts = end_date
        .and_hms_opt(23, 59, 59)
        .unwrap()
        .and_utc()
        .timestamp();

    let url = format!(
        "https://query1.finance.yahoo.com/v8/finance/chart/{}?period1={}&period2={}&interval=1d",
        symbol, start_ts, end_ts
    );

    let resp = http_client::general_client()
        .get(&url)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    let timestamps = json["chart"]["result"][0]["timestamp"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let closes = json["chart"]["result"][0]["indicators"]["quote"][0]["close"]
        .as_array()
        .cloned()
        .unwrap_or_default();

    let mut points: Vec<BenchmarkDataPoint> = Vec::new();
    let mut prev_close: Option<f64> = None;

    for (ts, cl) in timestamps.iter().zip(closes.iter()) {
        if let (Some(ts_i), Some(cl_f)) = (ts.as_i64(), cl.as_f64()) {
            let date = chrono::DateTime::from_timestamp(ts_i, 0)
                .unwrap_or_default()
                .date_naive();
            let change_pct = prev_close
                .map(|pc| {
                    if pc != 0.0 {
                        (cl_f - pc) / pc * 100.0
                    } else {
                        0.0
                    }
                })
                .unwrap_or(0.0);
            points.push(BenchmarkDataPoint {
                date: date.format("%Y-%m-%d").to_string(),
                close_price: cl_f,
                change_percent: change_pct,
            });
            prev_close = Some(cl_f);
        }
    }

    // Cache the fetched data
    cache_benchmark_prices(db, symbol, &points)?;

    Ok(points)
}

/// Build a return series for the benchmark (cumulative %).
/// When `base_price` is provided, cumulative returns are calculated relative
/// to this price (the closing price on the day before the visible range)
/// instead of the first element in `points`.
pub fn benchmark_to_return_series(
    points: &[BenchmarkDataPoint],
    base_price: Option<f64>,
) -> Vec<ReturnDataPoint> {
    if points.is_empty() {
        return vec![];
    }
    let start_price = base_price.unwrap_or(points[0].close_price);
    let mut prev_price = start_price;
    points
        .iter()
        .map(|p| {
            let daily_return = if prev_price > 0.0 {
                (p.close_price - prev_price) / prev_price * 100.0
            } else {
                0.0
            };
            let cumulative_return = if start_price > 0.0 {
                (p.close_price - start_price) / start_price * 100.0
            } else {
                0.0
            };
            prev_price = p.close_price;
            ReturnDataPoint {
                date: p.date.clone(),
                cumulative_return,
                daily_return,
                portfolio_value: p.close_price,
                daily_pnl: 0.0,
            }
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::performance::parse_date;

    #[test]
    fn previous_value_lookup_distinguishes_missing_from_bad_dates() {
        let db = Database::new(":memory:").unwrap();
        let cutoff = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        assert!(
            fetch_previous_day_value(&db, cutoff, &PerformanceFilter::default())
                .unwrap()
                .is_none()
        );

        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO daily_portfolio_values (date, total_value)
             VALUES ('2025-bad-date', 42)",
            [],
        )
        .unwrap();
        drop(conn);

        let error =
            fetch_previous_day_value(&db, cutoff, &PerformanceFilter::default()).unwrap_err();
        assert!(error.contains("bad date '2025-bad-date'"));
    }

    #[test]
    fn filtered_previous_value_lookup_propagates_bad_dates() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO daily_holding_snapshots
             (date, account_id, symbol, market, shares, avg_cost, close_price, market_value)
             VALUES ('2025-bad-date', 'acct', 'AAPL', 'US', 1, 1, 42, 42)",
            [],
        )
        .unwrap();
        drop(conn);
        let filter = PerformanceFilter {
            market: Some("US".to_string()),
            account_id: None,
        };

        let error =
            fetch_previous_day_value(&db, NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(), &filter)
                .unwrap_err();

        assert!(error.contains("bad date '2025-bad-date'"));
    }

    #[test]
    fn required_rate_parser_distinguishes_missing_from_malformed_json() {
        let missing = parse_required_exchange_rates(None, "cash flow").unwrap_err();
        assert!(missing.contains("missing exchange rates for cash flow"));

        let malformed = parse_required_exchange_rates(Some("not-json"), "cash flow").unwrap_err();
        assert!(malformed.contains("invalid exchange rates for cash flow"));
    }

    fn cash_flow_performance_db() -> Database {
        let db = Database::new(":memory:").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                 VALUES ('acct-us', 'US account', 'US', NULL, '2024-01-01', '2024-01-01')",
                [],
            )
            .unwrap();
            for (date, value) in [
                ("2024-01-01", 100.0),
                ("2024-01-02", 110.0),
                ("2024-01-03", 165.0),
            ] {
                conn.execute(
                    "INSERT INTO daily_portfolio_values
                     (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value,
                      hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
                     VALUES (?1, 0, ?2, 0, ?2, 0, 0, 0, 0,
                      '{\"usd_cny\":7.2,\"usd_hkd\":7.8,\"cny_hkd\":1.0833333333,\"updated_at\":\"2024-01-01\"}',
                      0, 0)",
                    rusqlite::params![date, value],
                )
                .unwrap();
            }
            conn.execute(
                "INSERT INTO transactions
                 (id, holding_id, account_id, symbol, name, market, transaction_type,
                  shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                 VALUES ('cash-in', NULL, 'acct-us', '$CASH-USD', 'Cash', 'US', 'BUY',
                         0, 0, 50, 0, 'USD', '2024-01-03T09:00:00Z', NULL, '2024-01-03')",
                [],
            )
            .unwrap();
        }
        db
    }

    fn sold_holding_performance_db() -> Database {
        let db = Database::new(":memory:").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                 VALUES ('acct-us', 'US account', 'US', NULL, '2024-01-01', '2024-01-01')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                  currency, created_at, updated_at)
                 VALUES ('holding-aapl', 'acct-us', 'AAPL', 'Apple', 'US', NULL, 0, 100,
                         'USD', '2024-01-01', '2024-01-03')",
                [],
            )
            .unwrap();
            for (date, value) in [
                ("2024-01-01", 100.0),
                ("2024-01-02", 104.0),
                ("2024-01-03", 122.0),
            ] {
                conn.execute(
                    "INSERT INTO daily_portfolio_values
                     (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value,
                      hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
                     VALUES (?1, 0, ?2, 0, ?2, 0, 0, 0, 0,
                      '{\"usd_cny\":7.2,\"usd_hkd\":7.8,\"cny_hkd\":1.0833333333,\"updated_at\":\"2024-01-01\"}',
                      0, 0)",
                    rusqlite::params![date, value],
                )
                .unwrap();
            }
            conn.execute(
                "INSERT INTO daily_holding_snapshots
                 (date, account_id, symbol, market, category_name, shares, avg_cost,
                  close_price, market_value)
                 VALUES ('2024-01-01', 'acct-us', 'AAPL', 'US', '成长股', 1, 100, 100, 100)",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO daily_holding_snapshots
                 (date, account_id, symbol, market, category_name, shares, avg_cost,
                  close_price, market_value)
                 VALUES ('2024-01-03', 'acct-us', '$CASH-USD', 'US', NULL, 122, 1, 1, 122)",
                [],
            )
            .unwrap();
            for (id, transaction_type, total_amount, commission) in [
                ("aapl-dividend", "PAY", 5.0, 1.0),
                ("aapl-sell", "SELL", 120.0, 2.0),
            ] {
                conn.execute(
                    "INSERT INTO transactions
                     (id, holding_id, account_id, symbol, name, market, transaction_type,
                      shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                     VALUES (?1, 'holding-aapl', 'acct-us', 'AAPL', 'Apple', 'US', ?2,
                             1, 120, ?3, ?4, 'USD', '2024-01-02T09:00:00Z', NULL, '2024-01-02')",
                    rusqlite::params![id, transaction_type, total_amount, commission],
                )
                .unwrap();
            }
        }
        db
    }

    fn roundtrip_holding_performance_db() -> Database {
        let db = sold_holding_performance_db();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                  currency, created_at, updated_at)
                 VALUES ('holding-msft', 'acct-us', 'MSFT', 'Microsoft', 'US', NULL, 0, 100,
                         'USD', '2024-01-02', '2024-01-02')",
                [],
            )
            .unwrap();
            for (id, transaction_type, total_amount, commission) in [
                ("msft-buy", "BUY", 100.0, 1.0),
                ("msft-sell", "SELL", 120.0, 2.0),
            ] {
                conn.execute(
                    "INSERT INTO transactions
                     (id, holding_id, account_id, symbol, name, market, transaction_type,
                      shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                     VALUES (?1, 'holding-msft', 'acct-us', 'MSFT', 'Microsoft', 'US', ?2,
                             1, 100, ?3, ?4, 'USD', '2024-01-02T10:00:00Z', NULL, '2024-01-02')",
                    rusqlite::params![id, transaction_type, total_amount, commission],
                )
                .unwrap();
            }
        }
        db
    }

    fn multicurrency_attribution_db() -> Database {
        let db = Database::new(":memory:").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                 VALUES ('acct-cn', 'CN account', 'CN', NULL, '2024-01-01', '2024-01-01')",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                  currency, created_at, updated_at)
                 VALUES ('holding-cn', 'acct-cn', '600000', '浦发银行', 'CN', NULL, 100, 7.2,
                         'CNY', '2024-01-01', '2024-01-03')",
                [],
            )
            .unwrap();
            for (date, native_value, usd_value) in [
                ("2024-01-01", 720.0, 100.0),
                ("2024-01-02", 756.0, 105.0),
                ("2024-01-03", 792.0, 110.0),
            ] {
                conn.execute(
                    "INSERT INTO daily_portfolio_values
                     (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value,
                      hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
                     VALUES (?1, 0, ?3, 0, 0, 0, ?2, 0, 0,
                      '{\"usd_cny\":7.2,\"usd_hkd\":7.8,\"cny_hkd\":1.0833333333,\"updated_at\":\"2024-01-01\"}',
                      0, 0)",
                    rusqlite::params![date, native_value, usd_value],
                )
                .unwrap();
                conn.execute(
                    "INSERT INTO daily_holding_snapshots
                     (date, account_id, symbol, market, category_name, shares, avg_cost,
                      close_price, market_value)
                     VALUES (?1, 'acct-cn', '600000', 'CN', '分红股', 100, 7.2,
                             ?2 / 100.0, ?2)",
                    rusqlite::params![date, native_value],
                )
                .unwrap();
            }
        }
        db
    }

    fn duplicate_symbol_multicurrency_db() -> Database {
        let db = Database::new(":memory:").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            for (id, name, market) in [
                ("acct-us", "US account", "US"),
                ("acct-cn", "CN account", "CN"),
            ] {
                conn.execute(
                    "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
                     VALUES (?1, ?2, ?3, NULL, '2024-01-01', '2024-01-03')",
                    rusqlite::params![id, name, market],
                )
                .unwrap();
            }
            for (id, name) in [("cat-growth", "成长股"), ("cat-value", "价值股")] {
                conn.execute(
                    "INSERT INTO categories
                     (id, name, color, icon, is_system, sort_order, created_at)
                     VALUES (?1, ?2, '#000000', '', 0, 0, '2024-01-01')",
                    rusqlite::params![id, name],
                )
                .unwrap();
            }
            for (id, account_id, name, market, category_id, currency, shares, avg_cost) in [
                (
                    "holding-us",
                    "acct-us",
                    "Duplicate US",
                    "US",
                    "cat-growth",
                    "USD",
                    1.0,
                    100.0,
                ),
                (
                    "holding-cn",
                    "acct-cn",
                    "Duplicate CN",
                    "CN",
                    "cat-value",
                    "CNY",
                    100.0,
                    7.2,
                ),
            ] {
                conn.execute(
                    "INSERT INTO holdings
                     (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                      currency, created_at, updated_at)
                     VALUES (?1, ?2, 'DUP', ?3, ?4, ?5, ?7, ?8, ?6,
                             '2024-01-01', '2024-01-03')",
                    rusqlite::params![
                        id,
                        account_id,
                        name,
                        market,
                        category_id,
                        currency,
                        shares,
                        avg_cost
                    ],
                )
                .unwrap();
            }
            for (date, us_value, cn_value, total_value) in [
                ("2024-01-01", 100.0, 720.0, 200.0),
                ("2024-01-02", 105.0, 756.0, 210.0),
                ("2024-01-03", 110.0, 792.0, 220.0),
            ] {
                conn.execute(
                    "INSERT INTO daily_portfolio_values
                     (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value,
                      hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
                     VALUES (?1, 0, ?4, 0, ?2, 0, ?3, 0, 0,
                      '{\"usd_cny\":7.2,\"usd_hkd\":7.8,\"cny_hkd\":1.0833333333,\"updated_at\":\"2024-01-01\"}',
                      0, 0)",
                    rusqlite::params![date, us_value, cn_value, total_value],
                )
                .unwrap();
                for (account_id, market, category, shares, avg_cost, close_price, market_value) in [
                    ("acct-us", "US", "成长股", 1.0, 100.0, us_value, us_value),
                    (
                        "acct-cn",
                        "CN",
                        "价值股",
                        100.0,
                        7.2,
                        cn_value / 100.0,
                        cn_value,
                    ),
                ] {
                    conn.execute(
                        "INSERT INTO daily_holding_snapshots
                         (date, account_id, symbol, market, category_name, shares, avg_cost,
                          close_price, market_value)
                         VALUES (?1, ?2, 'DUP', ?3, ?4, ?5, ?6, ?7, ?8)",
                        rusqlite::params![
                            date,
                            account_id,
                            market,
                            category,
                            shares,
                            avg_cost,
                            close_price,
                            market_value
                        ],
                    )
                    .unwrap();
                }
            }
        }
        db
    }

    fn flat_performance_db() -> Database {
        let db = Database::new(":memory:").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            for date in ["2024-01-01", "2024-01-02", "2024-01-03"] {
                conn.execute(
                    "INSERT INTO daily_portfolio_values
                     (date, total_cost, total_value, us_cost, us_value, cn_cost, cn_value,
                      hk_cost, hk_value, exchange_rates, daily_pnl, cumulative_pnl)
                     VALUES (?1, 0, 100, 0, 100, 0, 0, 0, 0,
                      '{\"usd_cny\":7.2,\"usd_hkd\":7.8,\"cny_hkd\":1.0833333333,\"updated_at\":\"2024-01-01\"}',
                      0, 0)",
                    rusqlite::params![date],
                )
                .unwrap();
            }
        }
        db
    }

    #[test]
    fn test_annualise_return() {
        let ar = annualise_return(0.10, 365);
        assert!((ar - 0.10).abs() < 1e-6);

        let ar2 = annualise_return(0.0, 365);
        assert_eq!(ar2, 0.0);
    }

    #[test]
    fn test_volatility() {
        let returns = vec![1.0, -1.0, 2.0, -2.0, 0.5];
        let (dv, av) = calculate_volatility(&returns);
        assert!(dv > 0.0);
        assert!((av - dv * 252.0_f64.sqrt()).abs() < 1e-9);
    }

    #[test]
    fn test_max_drawdown() {
        let series: Vec<ReturnDataPoint> = vec![
            ReturnDataPoint {
                date: "2024-01-01".to_string(),
                cumulative_return: 0.0,
                daily_return: 0.0,
                portfolio_value: 100.0,
                daily_pnl: 0.0,
            },
            ReturnDataPoint {
                date: "2024-01-02".to_string(),
                cumulative_return: 10.0,
                daily_return: 10.0,
                portfolio_value: 110.0,
                daily_pnl: 10.0,
            },
            ReturnDataPoint {
                date: "2024-01-03".to_string(),
                cumulative_return: -5.0,
                daily_return: -15.0,
                portfolio_value: 95.0,
                daily_pnl: -15.0,
            },
            ReturnDataPoint {
                date: "2024-01-04".to_string(),
                cumulative_return: 5.0,
                daily_return: 10.0,
                portfolio_value: 105.0,
                daily_pnl: 10.0,
            },
        ];
        let dd = calculate_max_drawdown(&series, None);
        // Peak = 110 on day 2, trough = 95 on day 3 → MDD = (95-110)/110 ≈ -13.6%
        assert!(dd.max_drawdown < 0.0);
        assert!((dd.max_drawdown - (-13.636_363_636)).abs() < 0.001);
        assert_eq!(dd.peak_date, "2024-01-02");
        assert_eq!(dd.trough_date, "2024-01-03");
    }

    #[test]
    fn test_twr_series_uses_baseline_and_neutralizes_external_cash_flow() {
        let daily = vec![
            (parse_date("2024-01-02").unwrap(), 110.0, 10.0),
            (parse_date("2024-01-03").unwrap(), 165.0, 55.0),
        ];
        let baseline = Some((parse_date("2024-01-01").unwrap(), 100.0));
        let external_cash_flows = vec![(parse_date("2024-01-03").unwrap(), 50.0)];

        let series = build_twr_return_series(&daily, baseline, &external_cash_flows);

        assert_eq!(series.len(), 2);
        // First visible day must retain the return from the previous valuation.
        assert!((series[0].daily_return - 10.0).abs() < 1e-9);
        // The 50 contribution is not investment performance: (165 - 110 - 50) / 110.
        assert!((series[1].daily_return - 4.545_454_545).abs() < 1e-9);
        // Daily sub-period returns are geometrically linked: 1.10 * 1.0454545 - 1 = 15%.
        assert!((series[1].cumulative_return - 15.0).abs() < 1e-9);
        assert!((series[1].daily_pnl - 5.0).abs() < 1e-9);
    }

    #[test]
    fn test_performance_summary_uses_twr_and_cash_adjusted_pnl() {
        let db = cash_flow_performance_db();

        let summary = get_performance_summary(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            &PerformanceFilter::default(),
        )
        .unwrap();

        assert!((summary.total_return - 15.0).abs() < 1e-9);
        assert!((summary.total_pnl - 15.0).abs() < 1e-9);
        assert_eq!(summary.start_date, "2024-01-01");
        assert_eq!(summary.end_date, "2024-01-03");
        assert!((summary.return_series[0].daily_return - 10.0).abs() < 1e-9);
        assert!((summary.return_series[1].daily_return - 4.545_454_545).abs() < 1e-9);
    }

    #[test]
    fn test_performance_report_matches_individual_results_and_loads_once() {
        let db = sold_holding_performance_db();
        let start = parse_date("2024-01-02").unwrap();
        let end = parse_date("2024-01-03").unwrap();
        let filter = PerformanceFilter::default();

        let expected_summary = get_performance_summary(&db, start, end, &filter).unwrap();
        let expected_drawdown = get_drawdown_analysis(&db, start, end, &filter).unwrap();
        let expected_attribution = get_return_attribution(&db, start, end, &filter).unwrap();
        let expected_monthly_returns = get_monthly_returns(&db, start, end, &filter).unwrap();
        let expected_holding_performances =
            get_holding_performance_ranking(&db, start, end, "pnl", 10_000, &filter).unwrap();
        let expected_risk_metrics = get_risk_metrics(&db, start, end, &filter).unwrap();

        reset_performance_load_count();
        let report = get_performance_report(&db, start, end, "pnl", 10_000, &filter).unwrap();

        assert_eq!(
            serde_json::to_value(report.summary).unwrap(),
            serde_json::to_value(expected_summary).unwrap()
        );
        assert_eq!(
            serde_json::to_value(report.drawdown).unwrap(),
            serde_json::to_value(expected_drawdown).unwrap()
        );
        assert_eq!(
            serde_json::to_value(report.attribution).unwrap(),
            serde_json::to_value(expected_attribution).unwrap()
        );
        assert_eq!(
            serde_json::to_value(report.monthly_returns).unwrap(),
            serde_json::to_value(expected_monthly_returns).unwrap()
        );
        assert_eq!(
            serde_json::to_value(report.holding_performances).unwrap(),
            serde_json::to_value(expected_holding_performances).unwrap()
        );
        assert_eq!(
            serde_json::to_value(report.risk_metrics).unwrap(),
            serde_json::to_value(expected_risk_metrics).unwrap()
        );
        assert_eq!(performance_load_count(), 1);
    }

    #[test]
    fn test_performance_summary_neutralizes_in_kind_open_contribution() {
        let db = cash_flow_performance_db();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE daily_portfolio_values
                    SET total_value = 186, us_value = 186
                  WHERE date = '2024-01-03'",
                [],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO transactions
                 (id, holding_id, account_id, symbol, name, market, transaction_type,
                  shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                 VALUES ('in-kind-open', NULL, 'acct-us', 'SPY', 'SPDR S&P 500 ETF', 'US', 'OPEN',
                         1, 20, 20, 1, 'USD', '2024-01-03T10:00:00Z', NULL, '2024-01-03')",
                [],
            )
            .unwrap();
        }

        let summary = get_performance_summary(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            &PerformanceFilter::default(),
        )
        .unwrap();

        // The additional USD 20 is an externally contributed position, not a
        // USD 20 investment gain. Performance remains 10% followed by 4.545%.
        assert!((summary.total_return - 15.0).abs() < 1e-9);
        assert!((summary.total_pnl - 15.0).abs() < 1e-9);
    }

    #[test]
    fn test_performance_summary_neutralizes_cash_withdrawal_and_fee() {
        let db = cash_flow_performance_db();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE transactions
                    SET transaction_type = 'SELL', commission = 1
                  WHERE id = 'cash-in'",
                [],
            )
            .unwrap();
            conn.execute(
                "UPDATE daily_portfolio_values
                    SET total_value = 64, us_value = 64
                  WHERE date = '2024-01-03'",
                [],
            )
            .unwrap();
        }

        let summary = get_performance_summary(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            &PerformanceFilter::default(),
        )
        .unwrap();

        // Withdrawing USD 50 plus its USD 1 fee reduces account value by 51;
        // the remaining USD 5 change is performance, not a 41.8% loss.
        assert!((summary.return_series[1].daily_return - 4.545_454_545).abs() < 1e-9);
        assert!((summary.total_return - 15.0).abs() < 1e-9);
        assert!((summary.total_pnl - 15.0).abs() < 1e-9);
    }

    #[test]
    fn test_monthly_return_geometrically_links_daily_twr_and_adjusts_pnl() {
        let db = cash_flow_performance_db();

        let months = get_monthly_returns(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            &PerformanceFilter::default(),
        )
        .unwrap();

        assert_eq!(months.len(), 1);
        assert!((months[0].return_rate - 15.0).abs() < 1e-9);
        assert!((months[0].pnl - 15.0).abs() < 1e-9);
        assert!((months[0].start_value - 100.0).abs() < 1e-9);
        assert!((months[0].end_value - 165.0).abs() < 1e-9);
    }

    #[test]
    fn test_attribution_includes_net_dividend_fees_and_fully_sold_holding() {
        let db = sold_holding_performance_db();

        let attribution = get_return_attribution(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            &PerformanceFilter::default(),
        )
        .unwrap();

        // 0 ending value - 100 starting value + 118 net sale + 4 net dividend.
        assert!((attribution.total_pnl - 22.0).abs() < 1e-9);
        assert_eq!(attribution.by_holding.len(), 1);
        assert_eq!(attribution.by_holding[0].name, "AAPL Apple");
        assert!((attribution.by_holding[0].pnl - 22.0).abs() < 1e-9);
    }

    #[test]
    fn test_ranking_keeps_fully_sold_holding_and_includes_income_and_fees() {
        let db = sold_holding_performance_db();

        let ranking = get_holding_performance_ranking(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            "return_rate",
            10,
            &PerformanceFilter::default(),
        )
        .unwrap();

        assert_eq!(ranking.len(), 1);
        assert_eq!(ranking[0].symbol, "AAPL");
        assert!((ranking[0].pnl - 22.0).abs() < 1e-9);
        assert!((ranking[0].return_rate - 22.0).abs() < 1e-9);
    }

    #[test]
    fn test_attribution_includes_holding_bought_and_sold_inside_period() {
        let db = roundtrip_holding_performance_db();

        let attribution = get_return_attribution(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            &PerformanceFilter::default(),
        )
        .unwrap();

        let microsoft = attribution
            .by_holding
            .iter()
            .find(|item| item.name == "MSFT Microsoft")
            .expect("round-trip holding must remain in attribution");
        // 118 net sale proceeds - 101 purchase outlay.
        assert!((microsoft.pnl - 17.0).abs() < 1e-9);
        assert!((attribution.total_pnl - 39.0).abs() < 1e-9);
        assert_eq!(attribution.by_market.len(), 1);
        assert_eq!(attribution.by_market[0].name, "🇺🇸 美股");
        assert!((attribution.by_market[0].pnl - 39.0).abs() < 1e-9);
    }

    #[test]
    fn test_unfiltered_attribution_aligns_baseline_and_normalizes_currency_to_usd() {
        let db = multicurrency_attribution_db();

        let attribution = get_return_attribution(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            &PerformanceFilter::default(),
        )
        .unwrap();

        // The selected period starts from the prior valuation (Jan 1), matching
        // the summary. CNY 72 gain / 7.2 = USD 10, not a mixed-currency 72.
        assert!((attribution.total_pnl - 10.0).abs() < 1e-9);
        assert!((attribution.by_market[0].pnl - 10.0).abs() < 1e-9);
        assert!((attribution.by_holding[0].pnl - 10.0).abs() < 1e-9);
    }

    #[test]
    fn test_attribution_keeps_duplicate_symbols_separate_until_after_currency_normalization() {
        let db = duplicate_symbol_multicurrency_db();

        let attribution = get_return_attribution(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            &PerformanceFilter::default(),
        )
        .unwrap();

        assert!((attribution.total_pnl - 20.0).abs() < 1e-9);
        assert_eq!(attribution.by_market.len(), 2);
        assert!(attribution
            .by_market
            .iter()
            .all(|item| (item.pnl - 10.0).abs() < 1e-9));
        assert_eq!(attribution.by_category.len(), 2);
        assert!(attribution
            .by_category
            .iter()
            .all(|item| (item.pnl - 10.0).abs() < 1e-9));
    }

    #[test]
    fn test_ranking_uses_same_previous_valuation_baseline_as_summary() {
        let db = multicurrency_attribution_db();

        let ranking = get_holding_performance_ranking(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            "return_rate",
            10,
            &PerformanceFilter::default(),
        )
        .unwrap();

        assert_eq!(ranking.len(), 1);
        // Unfiltered ranking normalizes CNY 72 / 7.2 to USD 10 before
        // comparing positions from different markets.
        assert!((ranking[0].pnl - 10.0).abs() < 1e-9);
        assert!((ranking[0].return_rate - 10.0).abs() < 1e-9);
    }

    #[test]
    fn test_unfiltered_ranking_normalizes_duplicate_symbols_before_sorting() {
        let db = duplicate_symbol_multicurrency_db();

        let ranking = get_holding_performance_ranking(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            "pnl",
            10,
            &PerformanceFilter::default(),
        )
        .unwrap();

        assert_eq!(ranking.len(), 2);
        assert!(ranking.iter().all(|item| (item.pnl - 10.0).abs() < 1e-9));
        assert!(ranking
            .iter()
            .all(|item| (item.return_rate - 10.0).abs() < 1e-9));
    }

    #[test]
    fn test_undefined_risk_ratios_are_not_reported_as_zero() {
        let db = flat_performance_db();

        let metrics = get_risk_metrics(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            &PerformanceFilter::default(),
        )
        .unwrap();

        assert_eq!(metrics.sharpe_ratio, None);
        assert_eq!(metrics.calmar_ratio, None);
    }

    #[test]
    fn test_ranking_labels_roundtrip_holding_from_current_metadata() {
        let db = roundtrip_holding_performance_db();

        let ranking = get_holding_performance_ranking(
            &db,
            parse_date("2024-01-02").unwrap(),
            parse_date("2024-01-03").unwrap(),
            "return_rate",
            10,
            &PerformanceFilter::default(),
        )
        .unwrap();

        let microsoft = ranking
            .iter()
            .find(|item| item.symbol == "MSFT")
            .expect("round-trip holding must remain in ranking");
        assert_eq!(microsoft.name, "Microsoft");
        assert_eq!(microsoft.market, "US");
        assert_eq!(microsoft.category_name, "未分类");
        assert!((microsoft.pnl - 17.0).abs() < 1e-9);
        assert!((microsoft.return_rate - (17.0 / 101.0 * 100.0)).abs() < 1e-9);
    }

    #[test]
    fn test_benchmark_to_return_series_no_base() {
        use crate::models::performance::BenchmarkDataPoint;
        let points = vec![
            BenchmarkDataPoint {
                date: "2024-01-01".into(),
                close_price: 100.0,
                change_percent: 0.0,
            },
            BenchmarkDataPoint {
                date: "2024-01-02".into(),
                close_price: 105.0,
                change_percent: 5.0,
            },
            BenchmarkDataPoint {
                date: "2024-01-03".into(),
                close_price: 103.0,
                change_percent: -1.9,
            },
        ];
        let series = benchmark_to_return_series(&points, None);
        assert_eq!(series.len(), 3);
        // Without base_price the first point is the baseline → 0%
        assert!((series[0].cumulative_return - 0.0).abs() < 1e-6);
        assert!((series[1].cumulative_return - 5.0).abs() < 1e-6);
        assert!((series[2].cumulative_return - 3.0).abs() < 1e-6);
    }

    #[test]
    fn test_benchmark_to_return_series_with_base() {
        use crate::models::performance::BenchmarkDataPoint;
        // Previous day close was 95 → first visible day already shows a return
        let points = vec![
            BenchmarkDataPoint {
                date: "2024-01-02".into(),
                close_price: 100.0,
                change_percent: 5.26,
            },
            BenchmarkDataPoint {
                date: "2024-01-03".into(),
                close_price: 105.0,
                change_percent: 5.0,
            },
        ];
        let series = benchmark_to_return_series(&points, Some(95.0));
        assert_eq!(series.len(), 2);
        // cumulative: (100 - 95) / 95 * 100 ≈ 5.263%
        assert!((series[0].cumulative_return - 5.263_157_894).abs() < 0.001);
        // cumulative: (105 - 95) / 95 * 100 ≈ 10.526%
        assert!((series[1].cumulative_return - 10.526_315_789).abs() < 0.001);
        // daily: (100 - 95) / 95 * 100 ≈ 5.263%
        assert!((series[0].daily_return - 5.263_157_894).abs() < 0.001);
        // daily: (105 - 100) / 100 * 100 = 5%
        assert!((series[1].daily_return - 5.0).abs() < 1e-6);
    }

    // ─────────────────────────────────────────────────────────────────────────
    // Additional performance calculation tests
    // ─────────────────────────────────────────────────────────────────────────

    #[test]
    fn test_annualise_return_half_year() {
        // 5% return in 182.5 days → annualised ≈ (1.05)^2 - 1 ≈ 10.25%
        let ar = annualise_return(0.05, 183);
        // (1.05)^(365/183) - 1 ≈ 0.10189
        assert!(ar > 0.10 && ar < 0.11);
    }

    #[test]
    fn test_annualise_return_zero_days() {
        assert_eq!(annualise_return(0.10, 0), 0.0);
        assert_eq!(annualise_return(0.10, -5), 0.0);
    }

    #[test]
    fn test_volatility_constant_returns() {
        // All same returns → variance = 0
        let returns = vec![1.0, 1.0, 1.0, 1.0, 1.0];
        let (dv, av) = calculate_volatility(&returns);
        assert!((dv - 0.0).abs() < 1e-9);
        assert!((av - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_volatility_single_return() {
        let (dv, av) = calculate_volatility(&[5.0]);
        assert_eq!(dv, 0.0);
        assert_eq!(av, 0.0);
    }

    #[test]
    fn test_volatility_empty() {
        let (dv, av) = calculate_volatility(&[]);
        assert_eq!(dv, 0.0);
        assert_eq!(av, 0.0);
    }

    #[test]
    fn test_sharpe_uses_mean_daily_excess_return_and_sample_volatility() {
        let returns = [0.01, 0.02, -0.01];

        let sharpe = calculate_sharpe_from_daily_returns(&returns, 0.0).unwrap();

        // mean = 0.0066667, sample stdev = 0.0152753;
        // annualised Sharpe = mean / stdev * sqrt(252).
        assert!((sharpe - 6.928_203_230).abs() < 1e-9);
    }

    #[test]
    fn test_sharpe_is_undefined_without_return_variability() {
        assert_eq!(
            calculate_sharpe_from_daily_returns(&[0.01, 0.01, 0.01], 0.0),
            None
        );
    }

    #[test]
    fn test_max_drawdown_no_drawdown() {
        // Monotonically increasing portfolio
        let series: Vec<ReturnDataPoint> = vec![
            ReturnDataPoint {
                date: "2024-01-01".into(),
                cumulative_return: 0.0,
                daily_return: 0.0,
                portfolio_value: 100.0,
                daily_pnl: 0.0,
            },
            ReturnDataPoint {
                date: "2024-01-02".into(),
                cumulative_return: 5.0,
                daily_return: 5.0,
                portfolio_value: 105.0,
                daily_pnl: 5.0,
            },
            ReturnDataPoint {
                date: "2024-01-03".into(),
                cumulative_return: 10.0,
                daily_return: 5.0,
                portfolio_value: 110.0,
                daily_pnl: 5.0,
            },
        ];
        let dd = calculate_max_drawdown(&series, None);
        assert!((dd.max_drawdown - 0.0).abs() < 1e-9);
    }

    #[test]
    fn test_max_drawdown_empty() {
        let dd = calculate_max_drawdown(&[], None);
        assert!((dd.max_drawdown - 0.0).abs() < 1e-9);
        assert_eq!(dd.drawdown_duration, 0);
    }

    #[test]
    fn test_max_drawdown_with_recovery() {
        let series: Vec<ReturnDataPoint> = vec![
            ReturnDataPoint {
                date: "2024-01-01".into(),
                cumulative_return: 0.0,
                daily_return: 0.0,
                portfolio_value: 100.0,
                daily_pnl: 0.0,
            },
            ReturnDataPoint {
                date: "2024-01-02".into(),
                cumulative_return: 20.0,
                daily_return: 20.0,
                portfolio_value: 120.0,
                daily_pnl: 20.0,
            },
            ReturnDataPoint {
                date: "2024-01-03".into(),
                cumulative_return: 0.0,
                daily_return: -16.67,
                portfolio_value: 100.0,
                daily_pnl: -20.0,
            },
            ReturnDataPoint {
                date: "2024-01-04".into(),
                cumulative_return: 25.0,
                daily_return: 25.0,
                portfolio_value: 125.0,
                daily_pnl: 25.0,
            },
        ];
        let dd = calculate_max_drawdown(&series, None);
        // Peak=120, trough=100 → (100-120)/120 = -16.67%
        assert!((dd.max_drawdown - (-16.666_666_667)).abs() < 0.01);
        assert_eq!(dd.peak_date, "2024-01-02");
        assert_eq!(dd.trough_date, "2024-01-03");
        // Recovery at day 4 when value=125 >= peak of 120
        assert_eq!(dd.recovery_date, Some("2024-01-04".to_string()));
    }

    #[test]
    fn test_max_drawdown_uses_twr_wealth_instead_of_raw_portfolio_value() {
        let series = vec![
            ReturnDataPoint {
                date: "2024-01-01".into(),
                cumulative_return: 0.0,
                daily_return: 0.0,
                portfolio_value: 100.0,
                daily_pnl: 0.0,
            },
            ReturnDataPoint {
                date: "2024-01-02".into(),
                cumulative_return: 0.0,
                daily_return: 0.0,
                portfolio_value: 200.0,
                daily_pnl: 0.0,
            },
            ReturnDataPoint {
                date: "2024-01-03".into(),
                cumulative_return: 0.0,
                daily_return: 0.0,
                portfolio_value: 100.0,
                daily_pnl: 0.0,
            },
        ];

        let drawdown = calculate_max_drawdown(&series, None);

        // The raw value halved because of a withdrawal, but investment wealth
        // stayed flat after neutralising the external cash flows.
        assert!((drawdown.max_drawdown - 0.0).abs() < 1e-9);
        assert!(drawdown
            .drawdown_series
            .iter()
            .all(|point| point.drawdown.abs() < 1e-9));
    }

    #[test]
    fn test_max_drawdown_includes_loss_from_baseline_to_first_visible_day() {
        let series = vec![
            ReturnDataPoint {
                date: "2024-01-02".into(),
                cumulative_return: -10.0,
                daily_return: -10.0,
                portfolio_value: 90.0,
                daily_pnl: -10.0,
            },
            ReturnDataPoint {
                date: "2024-01-03".into(),
                cumulative_return: -5.0,
                daily_return: 5.555_555_556,
                portfolio_value: 95.0,
                daily_pnl: 5.0,
            },
        ];

        let drawdown = calculate_max_drawdown(&series, Some(parse_date("2024-01-01").unwrap()));

        assert!((drawdown.max_drawdown - (-10.0)).abs() < 1e-9);
        assert!((drawdown.drawdown_series[0].drawdown - (-10.0)).abs() < 1e-9);
        assert_eq!(drawdown.peak_date, "2024-01-01");
        assert_eq!(drawdown.trough_date, "2024-01-02");
        assert_eq!(drawdown.drawdown_duration, 1);
        assert_eq!(drawdown.recovery_date, None);
    }
}
