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

/// Legacy cash snapshots could inherit the account's market instead of the
/// market corresponding to their settlement currency.
pub(super) const SNAPSHOT_MARKET_SQL: &str = "
    CASE UPPER(symbol)
      WHEN '$CASH-USD' THEN 'US'
      WHEN '$CASH-CNY' THEN 'CN'
      WHEN '$CASH-HKD' THEN 'HK'
      ELSE market
    END";

pub(super) fn append_snapshot_filters(
    filter: &PerformanceFilter,
    sql: &mut String,
    params: &mut Vec<Box<dyn rusqlite::types::ToSql>>,
) {
    if let Some(ref market) = filter.market {
        sql.push_str(&format!(
            " AND ({SNAPSHOT_MARKET_SQL}) = ?{}",
            params.len() + 1
        ));
        params.push(Box::new(market.clone()));
    }
    if let Some(ref account_id) = filter.account_id {
        sql.push_str(&format!(" AND account_id = ?{}", params.len() + 1));
        params.push(Box::new(account_id.clone()));
    }
}

pub(super) fn market_currency(market: &str) -> &'static str {
    match market {
        "CN" => "CNY",
        "HK" => "HKD",
        _ => "USD",
    }
}

fn report_currency(db: &Database, filter: &PerformanceFilter) -> Result<&'static str, String> {
    if let Some(ref market) = filter.market {
        return Ok(market_currency(market));
    }
    let Some(ref account_id) = filter.account_id else {
        return Ok("USD");
    };
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let market = conn
        .query_row(
            "SELECT market FROM accounts WHERE id = ?1",
            [account_id],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(market_currency(market.as_deref().unwrap_or("US")))
}

pub(super) fn convert_performance_amount(
    value: f64,
    source: &str,
    target: &str,
    rates_json: Option<&str>,
    context: &str,
) -> Result<f64, String> {
    if value == 0.0 || source == target {
        return Ok(value);
    }
    let rates = parse_required_exchange_rates(rates_json, context)?;
    Ok(crate::services::exchange_rate_service::convert_currency(
        value, source, target, &rates,
    ))
}

pub(super) fn valuation_rates(
    conn: &rusqlite::Connection,
    date: &str,
) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT exchange_rates FROM daily_portfolio_values WHERE date = ?1",
        [date],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| error.to_string())
}

/// Fetch daily portfolio values (total_value, daily_pnl) for the date range.
fn fetch_daily_values(
    db: &Database,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<(NaiveDate, f64, f64)>, String> {
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

/// Use the portfolio valuation calendar after this scope's first snapshot.
/// A valuation with no matching positions then represents a zero balance.
fn fetch_filtered_daily_values(
    db: &Database,
    start: NaiveDate,
    end: NaiveDate,
    filter: &PerformanceFilter,
    currency: &str,
    include_end: bool,
) -> Result<Vec<(NaiveDate, f64, f64)>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let (comparison, limit) = if include_end {
        ("<=", -1_i64)
    } else {
        ("<", 1_i64)
    };
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
        Box::new(start.to_string()),
        Box::new(end.to_string()),
        Box::new(limit),
    ];
    let mut predicate = String::new();
    append_snapshot_filters(filter, &mut predicate, &mut params);
    let sql = format!(
        "WITH positions AS (
            SELECT date, {SNAPSHOT_MARKET_SQL} AS position_market, SUM(market_value) AS value
            FROM daily_holding_snapshots WHERE date {comparison} ?2 {predicate}
            GROUP BY date, position_market
        ), valuation_dates AS (
            SELECT date FROM (
                SELECT date FROM positions WHERE date >= ?1
                UNION
                SELECT date FROM daily_portfolio_values
                WHERE date >= ?1 AND date {comparison} ?2 AND date >= (SELECT MIN(date) FROM positions)
            ) ORDER BY date DESC LIMIT ?3
        )
        SELECT d.date, h.position_market, COALESCE(h.value, 0), v.exchange_rates
        FROM valuation_dates d LEFT JOIN positions h ON h.date = d.date
        LEFT JOIN daily_portfolio_values v ON v.date = d.date
        ORDER BY d.date ASC, h.position_market ASC"
    );

    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let mut rows = stmt
        .query(param_refs.as_slice())
        .map_err(|error| error.to_string())?;
    let mut result: Vec<(NaiveDate, f64, f64)> = Vec::new();
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let date_str: String = row.get(0).map_err(|error| error.to_string())?;
        let market: Option<String> = row.get(1).map_err(|error| error.to_string())?;
        let value: f64 = row.get(2).map_err(|error| error.to_string())?;
        let rates: Option<String> = row.get(3).map_err(|error| error.to_string())?;
        let date = NaiveDate::parse_from_str(&date_str, "%Y-%m-%d")
            .map_err(|error| format!("bad date '{date_str}': {error}"))?;
        let value = convert_performance_amount(
            value,
            market_currency(market.as_deref().unwrap_or("US")),
            currency,
            rates.as_deref(),
            &format!("valuation on {date}"),
        )?;
        if let Some(last) = result.last_mut().filter(|last| last.0 == date) {
            last.1 += value;
        } else {
            result.push((date, value, 0.0));
        }
    }
    Ok(result)
}

/// Fetch the latest closing valuation strictly before the visible date range.
pub(super) fn fetch_previous_day_value(
    db: &Database,
    date: NaiveDate,
    filter: &PerformanceFilter,
    currency: &str,
) -> Result<Option<(NaiveDate, f64)>, String> {
    if filter.is_active() {
        return Ok(
            fetch_filtered_daily_values(db, NaiveDate::MIN, date, filter, currency, false)?
                .into_iter()
                .next()
                .map(|(date, value, _)| (date, value)),
        );
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let date_str = date.to_string();
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
/// Cash OPEN records reset a ledger balance. Stock OPEN records are in-kind
/// contributions. A market slice treats stock settlement in another market's
/// currency as an external flow between those slices. Use the first valuation
/// on or after the flow date for FX.
fn fetch_external_cash_flows(
    db: &Database,
    start_exclusive: NaiveDate,
    end_inclusive: NaiveDate,
    filter: &PerformanceFilter,
    report_currency: &str,
) -> Result<Vec<(NaiveDate, f64)>, String> {
    use crate::services::cash_reconciliation_service::{load_cash_ledger, CashLedger};
    use std::collections::hash_map::Entry;
    use std::collections::{BTreeMap, HashMap};

    struct ExternalFlow {
        date: String,
        id: String,
        account_id: String,
        is_cash: bool,
        market: String,
        transaction_type: String,
        recorded_amount: f64,
        commission: f64,
        currency: String,
        market_value: Option<f64>,
        rates_json: Option<String>,
    }

    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start_exclusive.format("%Y-%m-%d").to_string();
    let end_str = end_inclusive.format("%Y-%m-%d").to_string();
    let cash_symbol = "UPPER(t.symbol) LIKE '$CASH-%'";
    let mut sql = format!(
        "SELECT DATE(t.traded_at), t.id, t.account_id, {cash_symbol},
                t.market, t.transaction_type, t.total_amount, t.commission, t.currency, {},
                (SELECT d.exchange_rates
                   FROM daily_portfolio_values d
                  WHERE d.date >= DATE(t.traded_at) AND d.date <= ?2
                  ORDER BY d.date ASC LIMIT 1)
           FROM transactions t
          WHERE DATE(t.traded_at) > ?1 AND DATE(t.traded_at) <= ?2",
        super::TRANSFER_VALUE_SQL,
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(start_str), Box::new(end_str)];
    if let Some(ref account_id) = filter.account_id {
        sql.push_str(&format!(" AND t.account_id = ?{}", params.len() + 1));
        params.push(Box::new(account_id.clone()));
    }
    let external_transaction = format!(
        "(({cash_symbol} AND t.transaction_type IN ('BUY', 'SELL'))
          OR t.transaction_type IN ('OPEN', 'STOCK_IN', 'STOCK_OUT'))"
    );
    if let Some(ref market) = filter.market {
        let market_param = params.len() + 1;
        let currency_param = market_param + 1;
        sql.push_str(&format!(
            " AND (({external_transaction}
                    AND CASE WHEN {cash_symbol} THEN t.currency = ?{currency_param}
                             ELSE t.market = ?{market_param} END)
                   OR (NOT ({cash_symbol})
                       AND t.transaction_type IN ('BUY', 'SELL', 'PAY')
                       AND ((t.market = ?{market_param} AND t.currency <> ?{currency_param})
                            OR (t.market <> ?{market_param} AND t.currency = ?{currency_param}))))"
        ));
        params.push(Box::new(market.clone()));
        params.push(Box::new(report_currency.to_string()));
    } else {
        sql.push_str(" AND ");
        sql.push_str(&external_transaction);
    }
    sql.push_str(" ORDER BY DATE(t.traded_at) ASC");

    let param_refs: Vec<&dyn rusqlite::types::ToSql> =
        params.iter().map(|param| param.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(param_refs.as_slice(), |row| {
            Ok(ExternalFlow {
                date: row.get(0)?,
                id: row.get(1)?,
                account_id: row.get(2)?,
                is_cash: row.get(3)?,
                market: row.get(4)?,
                transaction_type: row.get(5)?,
                recorded_amount: row.get(6)?,
                commission: row.get(7)?,
                currency: row.get(8)?,
                market_value: row.get(9)?,
                rates_json: row.get(10)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    let mut grouped = BTreeMap::<NaiveDate, f64>::new();
    let mut cash_ledgers = HashMap::<(String, String), CashLedger>::new();
    for flow in rows {
        let date = NaiveDate::parse_from_str(&flow.date, "%Y-%m-%d")
            .map_err(|e| format!("bad cash-flow date '{}': {}", flow.date, e))?;
        let (signed_amount, currency) = if flow.is_cash && flow.transaction_type == "OPEN" {
            let key = (flow.account_id.clone(), flow.currency.clone());
            let ledger = match cash_ledgers.entry(key) {
                Entry::Occupied(entry) => entry.into_mut(),
                Entry::Vacant(entry) => {
                    entry.insert(load_cash_ledger(&conn, &flow.account_id, &flow.currency)?)
                }
            };
            let delta = ledger
                .rows
                .iter()
                .find(|row| row.transaction.id == flow.id)
                .map(|row| row.cash_delta)
                .ok_or_else(|| format!("现金流水缺少期初记录：{}", flow.id))?;
            (delta, flow.currency.as_str())
        } else {
            let (total_amount, currency) = super::resolve_transaction_value(
                &flow.transaction_type,
                flow.recorded_amount,
                &flow.currency,
                &flow.market,
                flow.market_value,
            )?;
            let signed_amount = match flow.transaction_type.as_str() {
                "BUY" | "OPEN" | "STOCK_IN" => total_amount + flow.commission,
                "SELL" if flow.is_cash => -(total_amount + flow.commission),
                "SELL" | "PAY" => -(total_amount - flow.commission),
                "STOCK_OUT" => -total_amount,
                _ => continue,
            };
            let is_settlement_market = !flow.is_cash
                && filter
                    .market
                    .as_deref()
                    .is_some_and(|market| market != flow.market);
            let amount = if is_settlement_market {
                -signed_amount
            } else {
                signed_amount
            };
            (amount, currency)
        };
        let amount = convert_performance_amount(
            signed_amount,
            currency,
            report_currency,
            flow.rates_json.as_deref(),
            &format!("{currency} cash flow on {}", flow.date),
        )?;
        *grouped.entry(date).or_insert(0.0) += amount;
    }

    Ok(grouped.into_iter().collect())
}

fn dates_without_market_closes(
    db: &Database,
    start: NaiveDate,
    end: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<std::collections::HashSet<String>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut sql =
        String::from("SELECT date FROM daily_holding_snapshots WHERE date BETWEEN ?1 AND ?2");
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(start.to_string()), Box::new(end.to_string())];
    append_snapshot_filters(filter, &mut sql, &mut params);
    sql.push_str(
        " GROUP BY date HAVING MAX(has_market_close) = 0
                   AND COUNT(has_market_close) = COUNT(*)",
    );
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let dates = stmt
        .query_map(param_refs.as_slice(), |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<_>>()
        .map_err(|error| error.to_string())?;
    Ok(dates)
}

pub(super) struct PerformanceCalculation {
    pub(super) currency: &'static str,
    pub(super) daily_values: Vec<(NaiveDate, f64, f64)>,
    pub(super) baseline: Option<(NaiveDate, f64)>,
    pub(super) external_cash_flows: Vec<(NaiveDate, f64)>,
    pub(super) return_series: Vec<ReturnDataPoint>,
    dates_without_market_closes: std::collections::HashSet<String>,
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

        let currency = report_currency(db, filter)?;
        let daily_values = if filter.is_active() {
            fetch_filtered_daily_values(db, start_date, end_date, filter, currency, true)?
        } else {
            fetch_daily_values(db, start_date, end_date)?
        };
        if daily_values.is_empty() {
            return Ok(Self {
                currency,
                daily_values,
                baseline: None,
                external_cash_flows: vec![],
                return_series: vec![],
                dates_without_market_closes: Default::default(),
            });
        }

        let baseline = fetch_previous_day_value(db, start_date, filter, currency)?;
        let cash_flow_start = baseline.map(|(date, _)| date).unwrap_or(daily_values[0].0);
        let actual_end = daily_values.last().map(|row| row.0).unwrap_or(end_date);
        let external_cash_flows =
            fetch_external_cash_flows(db, cash_flow_start, actual_end, filter, currency)?;
        let return_series = build_twr_return_series(&daily_values, baseline, &external_cash_flows);
        let dates_without_market_closes =
            dates_without_market_closes(db, start_date, end_date, filter)?;

        Ok(Self {
            currency,
            daily_values,
            baseline,
            external_cash_flows,
            return_series,
            dates_without_market_closes,
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

    fn annualized_return_percent(&self) -> Option<f64> {
        let total_return = self.total_return();
        let days = self.calendar_days();
        if days <= 0 || total_return < -1.0 {
            return None;
        }
        let percent = annualise_return(total_return, days) * 100.0;
        percent.is_finite().then_some(percent)
    }

    pub(super) fn daily_returns(&self) -> Vec<f64> {
        let prior_values = self
            .baseline
            .iter()
            .map(|(_, value)| *value)
            .chain(self.daily_values.iter().map(|(_, value, _)| *value));
        self.return_series
            .iter()
            .skip(usize::from(self.baseline.is_none()))
            .zip(prior_values)
            .filter(|(point, previous_value)| {
                *previous_value > 0.0
                    && (point.daily_return.abs() > 1e-10
                        || !self.dates_without_market_closes.contains(&point.date))
            })
            .map(|(point, _)| point.daily_return / 100.0)
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
    const WEALTH_INDEX_TOLERANCE: f64 = 1e-12;
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
        let dd = ((v - peak) / peak).min(0.0);
        let dd = if dd.abs() <= WEALTH_INDEX_TOLERANCE {
            0.0
        } else {
            dd
        };
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
                .position(|&v| v >= md_peak_value * (1.0 - WEALTH_INDEX_TOLERANCE))
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
            valuation_currency: calculation.currency.to_string(),
            start_date: requested_start_date.format("%Y-%m-%d").to_string(),
            end_date: requested_end_date.format("%Y-%m-%d").to_string(),
            start_value: 0.0,
            end_value: 0.0,
            total_return: 0.0,
            annualized_return: None,
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
    let actual_start_date = calculation.start_date().unwrap();
    let actual_end_date = calculation.end_date().unwrap();
    let dd_analysis = calculate_max_drawdown(
        &calculation.return_series,
        calculation.baseline.map(|(date, _)| date),
    );
    let daily_returns = calculation.daily_returns();
    let (_daily_vol, ann_vol) = calculate_volatility(&daily_returns);
    let sharpe = calculate_sharpe_from_daily_returns(&daily_returns, RISK_FREE_RATE);

    PerformanceSummary {
        valuation_currency: calculation.currency.to_string(),
        start_date: actual_start_date.format("%Y-%m-%d").to_string(),
        end_date: actual_end_date.format("%Y-%m-%d").to_string(),
        start_value,
        end_value,
        total_return: total_return_pct,
        annualized_return: calculation.annualized_return_percent(),
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

    let daily_returns = calculation.daily_returns();
    let (daily_vol, ann_vol) = calculate_volatility(&daily_returns);
    let sharpe = calculate_sharpe_from_daily_returns(&daily_returns, RISK_FREE_RATE);
    let dd_analysis = calculate_max_drawdown(
        &calculation.return_series,
        calculation.baseline.map(|(date, _)| date),
    );
    let max_dd = dd_analysis.max_drawdown.abs();
    let calmar = if max_dd > 0.0 {
        calculation
            .annualized_return_percent()
            .map(|annualized| annualized / max_dd)
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
