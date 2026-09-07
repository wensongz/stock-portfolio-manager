use super::{PerformanceFilter, RISK_FREE_RATE, TRADING_DAYS_PER_YEAR};
use crate::db::Database;
use crate::models::performance::*;
use chrono::{Datelike, NaiveDate};
use rusqlite::OptionalExtension;

pub(super) fn parse_required_exchange_rates(
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
pub(super) fn fetch_previous_day_value(
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
                 OR t.transaction_type IN ('OPEN', 'STOCK_IN', 'STOCK_OUT'))",
    );
    sql = sql.replace(
        "t.total_amount,",
        &format!("{},", super::TRANSFER_VALUE_SQL),
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
                row.get::<_, Option<f64>>(2)?,
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
        let total_amount = super::require_flow_value(total_amount)?;
        let signed_amount = match transaction_type.as_str() {
            "BUY" => total_amount + commission,
            "SELL" => -(total_amount + commission),
            "OPEN" | "STOCK_IN" => total_amount + commission,
            "STOCK_OUT" => -total_amount,
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

pub(super) struct PerformanceCalculation {
    pub(super) daily_values: Vec<(NaiveDate, f64, f64)>,
    pub(super) baseline: Option<(NaiveDate, f64)>,
    pub(super) external_cash_flows: Vec<(NaiveDate, f64)>,
    pub(super) return_series: Vec<ReturnDataPoint>,
}

#[cfg(test)]
thread_local! {
    static PERFORMANCE_LOAD_COUNT: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn reset_performance_load_count() {
    PERFORMANCE_LOAD_COUNT.with(|count| count.set(0));
}

#[cfg(test)]
pub(super) fn performance_load_count() -> usize {
    PERFORMANCE_LOAD_COUNT.with(std::cell::Cell::get)
}

impl PerformanceCalculation {
    pub(super) fn load(
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

    pub(super) fn start_date(&self) -> Option<NaiveDate> {
        self.baseline
            .map(|(date, _)| date)
            .or_else(|| self.daily_values.first().map(|row| row.0))
    }

    pub(super) fn end_date(&self) -> Option<NaiveDate> {
        self.daily_values.last().map(|row| row.0)
    }

    pub(super) fn start_value(&self) -> f64 {
        self.baseline
            .map(|(_, value)| value)
            .or_else(|| self.daily_values.first().map(|row| row.1))
            .unwrap_or(0.0)
    }

    pub(super) fn end_value(&self) -> f64 {
        self.daily_values.last().map(|row| row.1).unwrap_or(0.0)
    }

    pub(super) fn total_external_cash_flow(&self) -> f64 {
        self.external_cash_flows
            .iter()
            .map(|(_, amount)| *amount)
            .sum()
    }

    pub(super) fn total_pnl(&self) -> f64 {
        self.end_value() - self.start_value() - self.total_external_cash_flow()
    }

    pub(super) fn total_return(&self) -> f64 {
        self.return_series
            .last()
            .map(|point| point.cumulative_return / 100.0)
            .unwrap_or(0.0)
    }

    pub(super) fn calendar_days(&self) -> i64 {
        match (self.start_date(), self.end_date()) {
            (Some(start), Some(end)) => (end - start).num_days(),
            _ => 0,
        }
    }

    pub(super) fn daily_returns(&self) -> Vec<f64> {
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
pub(super) fn calculate_max_drawdown(
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

pub(super) fn drawdown_analysis_from(calculation: &PerformanceCalculation) -> DrawdownAnalysis {
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

pub(super) fn performance_summary_from(
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

pub(super) fn risk_metrics_from(calculation: &PerformanceCalculation) -> RiskMetrics {
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

pub fn get_monthly_returns(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<Vec<MonthlyReturn>, String> {
    let calculation = PerformanceCalculation::load(db, start_date, end_date, filter)?;
    Ok(monthly_returns_from(&calculation))
}

pub(super) fn monthly_returns_from(calculation: &PerformanceCalculation) -> Vec<MonthlyReturn> {
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
