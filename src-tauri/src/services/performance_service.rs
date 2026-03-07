use crate::db::Database;
use chrono::Datelike;

const RISK_FREE_RATE: f64 = 0.045; // 4.5% US 10-year treasury default
const TRADING_DAYS_PER_YEAR: f64 = 252.0;
const CACHE_COVERAGE_THRESHOLD: f64 = 0.5; // require 50% of expected days in cache to skip re-fetch
use crate::models::performance::{
    annualise_return, calculate_twr_from_periods, AttributionItem, BenchmarkDataPoint,
    DrawdownAnalysis, DrawdownPoint, HoldingPerformance, MonthlyReturn, PerformanceSummary,
    ReturnAttribution, ReturnDataPoint, RiskMetrics, SubPeriod,
};
use chrono::NaiveDate;

// ─────────────────────────────────────────────────────────────────────────────
// Internal DB helpers
// ─────────────────────────────────────────────────────────────────────────────

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

/// Fetch transaction dates (for TWR sub-period boundaries) in the date range.
fn fetch_transaction_dates(
    db: &Database,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<NaiveDate>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start.format("%Y-%m-%d").to_string();
    let end_str = end.format("%Y-%m-%d").to_string();

    let mut stmt = conn
        .prepare(
            "SELECT DISTINCT DATE(traded_at) as d
             FROM transactions
             WHERE DATE(traded_at) BETWEEN ?1 AND ?2
             ORDER BY d ASC",
        )
        .map_err(|e| e.to_string())?;

    let dates = stmt
        .query_map(rusqlite::params![start_str, end_str], |row| {
            let ds: String = row.get(0)?;
            Ok(ds)
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    dates
        .into_iter()
        .map(|ds| {
            NaiveDate::parse_from_str(&ds, "%Y-%m-%d")
                .map_err(|e| format!("bad date '{}': {}", ds, e))
        })
        .collect()
}

// ─────────────────────────────────────────────────────────────────────────────
// Core calculations
// ─────────────────────────────────────────────────────────────────────────────

/// Build a Vec<ReturnDataPoint> from the daily portfolio values.
pub fn build_return_series(
    daily_values: &[(NaiveDate, f64, f64)],
) -> Vec<ReturnDataPoint> {
    if daily_values.is_empty() {
        return vec![];
    }

    let start_value = daily_values[0].1;
    let mut prev_value = start_value;
    let mut result = Vec::with_capacity(daily_values.len());

    for (date, value, dpnl) in daily_values {
        let daily_return = if prev_value > 0.0 {
            (value - prev_value) / prev_value
        } else {
            0.0
        };
        let cumulative_return = if start_value > 0.0 {
            (value - start_value) / start_value
        } else {
            0.0
        };
        result.push(ReturnDataPoint {
            date: date.format("%Y-%m-%d").to_string(),
            cumulative_return: cumulative_return * 100.0,
            daily_return: daily_return * 100.0,
            portfolio_value: *value,
            daily_pnl: *dpnl,
        });
        prev_value = *value;
    }
    result
}

/// Calculate maximum drawdown from a return series.
pub fn calculate_max_drawdown(return_series: &[ReturnDataPoint]) -> DrawdownAnalysis {
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

    let values: Vec<f64> = return_series.iter().map(|r| r.portfolio_value).collect();
    let dates: Vec<&str> = return_series.iter().map(|r| r.date.as_str()).collect();

    let mut peak = values[0];
    let mut peak_idx = 0usize;
    let mut max_drawdown = 0.0f64;
    let mut md_peak_idx = 0usize;
    let mut md_trough_idx = 0usize;

    let mut drawdown_series = Vec::with_capacity(values.len());

    for (i, &v) in values.iter().enumerate() {
        if v > peak {
            peak = v;
            peak_idx = i;
        }
        let dd = if peak > 0.0 { (v - peak) / peak } else { 0.0 };
        drawdown_series.push(DrawdownPoint {
            date: dates[i].to_string(),
            drawdown: dd * 100.0,
        });
        if dd < max_drawdown {
            max_drawdown = dd;
            md_peak_idx = peak_idx;
            md_trough_idx = i;
        }
    }

    // Find recovery date: first date after trough where value >= peak at trough time
    let peak_value_at_md = values[md_peak_idx];
    let recovery_idx = values[md_trough_idx..]
        .iter()
        .position(|&v| v >= peak_value_at_md)
        .map(|offset| md_trough_idx + offset);

    let peak_date_str = dates[md_peak_idx].to_string();
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
    let variance = daily_returns.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1) as f64;
    let daily_vol = variance.sqrt();
    let annualised_vol = daily_vol * TRADING_DAYS_PER_YEAR.sqrt();
    (daily_vol, annualised_vol)
}

/// Calculate Sharpe ratio.
pub fn calculate_sharpe(annualised_return: f64, risk_free_rate: f64, annualised_vol: f64) -> f64 {
    if annualised_vol == 0.0 {
        return 0.0;
    }
    (annualised_return - risk_free_rate) / annualised_vol
}

// ─────────────────────────────────────────────────────────────────────────────
// Public service functions called from commands
// ─────────────────────────────────────────────────────────────────────────────

pub fn get_return_series(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<Vec<ReturnDataPoint>, String> {
    let daily = fetch_daily_values(db, start_date, end_date)?;
    Ok(build_return_series(&daily))
}

pub fn get_performance_summary(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<PerformanceSummary, String> {
    let daily = fetch_daily_values(db, start_date, end_date)?;
    if daily.is_empty() {
        return Ok(PerformanceSummary {
            start_date: start_date.format("%Y-%m-%d").to_string(),
            end_date: end_date.format("%Y-%m-%d").to_string(),
            start_value: 0.0,
            end_value: 0.0,
            total_return: 0.0,
            annualized_return: 0.0,
            total_pnl: 0.0,
            max_drawdown: 0.0,
            volatility: 0.0,
            sharpe_ratio: 0.0,
        });
    }

    let start_value = daily[0].1;
    let end_value = daily.last().unwrap().1;

    // TWR using transaction-date sub-periods
    let tx_dates = fetch_transaction_dates(db, start_date, end_date)?;
    let twr = compute_twr(&daily, &tx_dates, start_value);
    let days = (end_date - start_date).num_days();
    let annualised = annualise_return(twr, days);
    let total_pnl = end_value - start_value;

    let return_series = build_return_series(&daily);
    let dd_analysis = calculate_max_drawdown(&return_series);

    let daily_returns: Vec<f64> = return_series.iter().map(|r| r.daily_return).collect();
    let (_daily_vol, ann_vol) = calculate_volatility(&daily_returns);
    let sharpe = calculate_sharpe(annualised, RISK_FREE_RATE, ann_vol);

    Ok(PerformanceSummary {
        start_date: start_date.format("%Y-%m-%d").to_string(),
        end_date: end_date.format("%Y-%m-%d").to_string(),
        start_value,
        end_value,
        total_return: twr * 100.0,
        annualized_return: annualised * 100.0,
        total_pnl,
        max_drawdown: dd_analysis.max_drawdown,
        volatility: ann_vol * 100.0,
        sharpe_ratio: sharpe,
    })
}

/// Compute TWR from daily values and known cash-flow dates.
fn compute_twr(
    daily: &[(NaiveDate, f64, f64)],
    tx_dates: &[NaiveDate],
    _start_value: f64,
) -> f64 {
    if daily.is_empty() {
        return 0.0;
    }

    // Build boundary dates: start of each sub-period is a transaction date
    let mut boundaries: std::collections::HashSet<NaiveDate> =
        tx_dates.iter().cloned().collect();
    boundaries.insert(daily[0].0);
    boundaries.insert(daily.last().unwrap().0);

    let mut sorted_boundaries: Vec<NaiveDate> = boundaries.into_iter().collect();
    sorted_boundaries.sort();

    // Build a date->value map for quick look-up
    let value_map: std::collections::HashMap<NaiveDate, f64> =
        daily.iter().map(|(d, v, _)| (*d, *v)).collect();

    let mut periods: Vec<SubPeriod> = Vec::new();

    for window in sorted_boundaries.windows(2) {
        let period_start = window[0];
        let period_end = window[1];

        let sv = value_map.get(&period_start).copied().unwrap_or(0.0);
        let ev = value_map.get(&period_end).copied().unwrap_or(0.0);

        if sv > 0.0 {
            periods.push(SubPeriod {
                start_value: sv,
                end_value: ev,
                cash_flow: 0.0, // simplified: treat snapshot values as post-CF
            });
        }
    }

    if periods.is_empty() {
        // Fallback: simple return
        let sv = daily[0].1;
        let ev = daily.last().unwrap().1;
        if sv > 0.0 {
            return (ev - sv) / sv;
        }
        return 0.0;
    }

    calculate_twr_from_periods(&periods)
}

pub fn get_risk_metrics(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<RiskMetrics, String> {
    let daily = fetch_daily_values(db, start_date, end_date)?;
    if daily.is_empty() {
        return Ok(RiskMetrics {
            daily_volatility: 0.0,
            annualized_volatility: 0.0,
            sharpe_ratio: 0.0,
            risk_free_rate: 4.5,
            max_drawdown: 0.0,
            calmar_ratio: 0.0,
        });
    }

    let tx_dates = fetch_transaction_dates(db, start_date, end_date)?;
    let start_value = daily[0].1;
    let twr = compute_twr(&daily, &tx_dates, start_value);
    let days = (end_date - start_date).num_days();
    let annualised = annualise_return(twr, days);

    let return_series = build_return_series(&daily);
    let daily_returns: Vec<f64> = return_series.iter().map(|r| r.daily_return).collect();
    let (daily_vol, ann_vol) = calculate_volatility(&daily_returns);

    let sharpe = calculate_sharpe(annualised, RISK_FREE_RATE, ann_vol);

    let dd_analysis = calculate_max_drawdown(&return_series);
    let max_dd = dd_analysis.max_drawdown.abs() / 100.0;
    let calmar = if max_dd > 0.0 { annualised / max_dd } else { 0.0 };

    Ok(RiskMetrics {
        daily_volatility: daily_vol * 100.0,
        annualized_volatility: ann_vol * 100.0,
        sharpe_ratio: sharpe,
        risk_free_rate: RISK_FREE_RATE * 100.0,
        max_drawdown: dd_analysis.max_drawdown,
        calmar_ratio: calmar,
    })
}

pub fn get_return_attribution(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<ReturnAttribution, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start_date.format("%Y-%m-%d").to_string();
    let end_str = end_date.format("%Y-%m-%d").to_string();

    // Get start and end snapshots aggregated by symbol
    let mut start_vals: std::collections::HashMap<String, (String, String, f64)> =
        std::collections::HashMap::new();
    let mut end_vals: std::collections::HashMap<String, f64> =
        std::collections::HashMap::new();

    {
        let mut stmt = conn
            .prepare(
                "SELECT symbol, market, COALESCE(category_name, '未分类'), market_value
                 FROM daily_holding_snapshots
                 WHERE date = (
                     SELECT MAX(date) FROM daily_holding_snapshots WHERE date <= ?1
                 )",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![start_str], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (sym, mkt, cat, val) in rows {
            start_vals.insert(sym, (mkt, cat, val));
        }
    }

    {
        let mut stmt = conn
            .prepare(
                "SELECT symbol, market_value
                 FROM daily_holding_snapshots
                 WHERE date = (
                     SELECT MAX(date) FROM daily_holding_snapshots WHERE date <= ?1
                 )",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![end_str], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (sym, val) in rows {
            end_vals.insert(sym, val);
        }
    }

    let all_symbols: std::collections::HashSet<String> = start_vals
        .keys()
        .chain(end_vals.keys())
        .cloned()
        .collect();

    let mut total_pnl = 0.0f64;
    let mut total_start_val = 0.0f64;
    let mut market_pnl: std::collections::HashMap<String, f64> =
        std::collections::HashMap::new();
    let mut category_pnl: std::collections::HashMap<String, f64> =
        std::collections::HashMap::new();
    let mut holding_pnl: std::collections::HashMap<String, (String, String, f64, f64, f64)> =
        std::collections::HashMap::new();

    for sym in &all_symbols {
        let (market, cat, sv) = start_vals
            .get(sym)
            .map(|(m, c, v)| (m.clone(), c.clone(), *v))
            .unwrap_or_else(|| ("Unknown".to_string(), "未分类".to_string(), 0.0));
        let ev = end_vals.get(sym).copied().unwrap_or(0.0);
        let pnl = ev - sv;

        total_pnl += pnl;
        total_start_val += sv;
        *market_pnl.entry(market.clone()).or_insert(0.0) += pnl;
        *category_pnl.entry(cat.clone()).or_insert(0.0) += pnl;
        holding_pnl
            .entry(sym.clone())
            .and_modify(|e| {
                e.2 += pnl;
                e.3 += sv;
                e.4 += ev;
            })
            .or_insert((market, cat, pnl, sv, ev));
    }

    let make_items =
        |map: std::collections::HashMap<String, f64>| -> Vec<AttributionItem> {
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
            items.sort_by(|a, b| b.pnl.partial_cmp(&a.pnl).unwrap_or(std::cmp::Ordering::Equal));
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
        .map(|(sym, (_mkt, _cat, pnl, sv, _ev))| {
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
                name: sym,
                pnl,
                contribution_percent,
                weight,
            }
        })
        .collect();
    by_holding.sort_by(|a, b| b.pnl.partial_cmp(&a.pnl).unwrap_or(std::cmp::Ordering::Equal));

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
) -> Result<Vec<MonthlyReturn>, String> {
    let daily = fetch_daily_values(db, start_date, end_date)?;
    if daily.is_empty() {
        return Ok(vec![]);
    }

    // Group by year-month
    let mut months: std::collections::BTreeMap<(i32, u32), (NaiveDate, f64, NaiveDate, f64)> =
        std::collections::BTreeMap::new();

    for (date, value, _dpnl) in &daily {
        let key = (date.year(), date.month());
        months
            .entry(key)
            .and_modify(|e| {
                if *date > e.2 {
                    e.2 = *date;
                    e.3 = *value;
                }
            })
            .or_insert((*date, *value, *date, *value));
    }

    // Build a sorted list of month-start values
    let keys: Vec<(i32, u32)> = months.keys().cloned().collect();
    let mut result = Vec::new();

    for (i, &key) in keys.iter().enumerate() {
        let (_, _, end_d, end_v) = months[&key];
        // start value is either the last day of the prior month or the first day of this month
        let start_v = if i == 0 {
            // Use the first data point of this month as start
            let (_, first_v, _, _) = months[&key];
            first_v
        } else {
            let prev_key = keys[i - 1];
            let (_, _, _, prev_end_v) = months[&prev_key];
            prev_end_v
        };

        let pnl = end_v - start_v;
        let return_rate = if start_v > 0.0 {
            (end_v - start_v) / start_v * 100.0
        } else {
            0.0
        };

        result.push(MonthlyReturn {
            year: end_d.year(),
            month: end_d.month(),
            return_rate,
            pnl,
            start_value: start_v,
            end_value: end_v,
        });
    }

    Ok(result)
}

pub fn get_holding_performance_ranking(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    sort_by: &str,
    limit: usize,
) -> Result<Vec<HoldingPerformance>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let start_str = start_date.format("%Y-%m-%d").to_string();
    let end_str = end_date.format("%Y-%m-%d").to_string();

    // Collect per-symbol start and end values from snapshots
    struct SnapRow {
        symbol: String,
        market: String,
        category_name: String,
        market_value: f64,
    }

    let fetch_snap = |date_param: &str| -> Result<Vec<SnapRow>, String> {
        let mut stmt = conn
            .prepare(
                "SELECT symbol, market, COALESCE(category_name, '未分类'), SUM(market_value)
                 FROM daily_holding_snapshots
                 WHERE date = (
                     SELECT MAX(date) FROM daily_holding_snapshots WHERE date <= ?1
                 )
                 GROUP BY symbol",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(rusqlite::params![date_param], |row| {
                Ok(SnapRow {
                    symbol: row.get(0)?,
                    market: row.get(1)?,
                    category_name: row.get(2)?,
                    market_value: row.get(3)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    };

    let start_snaps = fetch_snap(&start_str)?;
    let end_snaps = fetch_snap(&end_str)?;

    let mut start_map: std::collections::HashMap<String, (String, String, f64)> =
        std::collections::HashMap::new();
    for s in start_snaps {
        start_map.insert(s.symbol, (s.market, s.category_name, s.market_value));
    }

    let mut performances: Vec<HoldingPerformance> = end_snaps
        .into_iter()
        .map(|e| {
            let (market, cat, sv) = start_map
                .get(&e.symbol)
                .cloned()
                .unwrap_or_else(|| (e.market.clone(), e.category_name.clone(), 0.0));
            let ev = e.market_value;
            let pnl = ev - sv;
            let return_rate = if sv > 0.0 { pnl / sv * 100.0 } else { 0.0 };
            HoldingPerformance {
                symbol: e.symbol,
                name: String::new(), // will be filled below
                market,
                category_name: cat,
                return_rate,
                pnl,
                start_value: sv,
                end_value: ev,
            }
        })
        .collect();

    // Enrich with holding names
    {
        let mut name_stmt = conn
            .prepare("SELECT symbol, name FROM holdings")
            .map_err(|e| e.to_string())?;
        let name_map: std::collections::HashMap<String, String> = name_stmt
            .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?
            .into_iter()
            .collect();
        for p in &mut performances {
            if let Some(name) = name_map.get(&p.symbol) {
                p.name = name.clone();
            }
            if p.name.is_empty() {
                p.name = p.symbol.clone();
            }
        }
    }

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

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())?;

    let resp = client
        .get(&url)
        .header("User-Agent", "Mozilla/5.0")
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
                .map(|pc| if pc != 0.0 { (cl_f - pc) / pc * 100.0 } else { 0.0 })
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
pub fn benchmark_to_return_series(points: &[BenchmarkDataPoint]) -> Vec<ReturnDataPoint> {
    if points.is_empty() {
        return vec![];
    }
    let start_price = points[0].close_price;
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
    fn test_twr_calculation() {
        let periods = vec![
            SubPeriod { start_value: 100.0, end_value: 110.0, cash_flow: 0.0 },
            SubPeriod { start_value: 110.0, end_value: 99.0, cash_flow: 0.0 },
        ];
        let twr = calculate_twr_from_periods(&periods);
        // (110/100 - 1) = 0.1, (99/110 - 1) = -0.1, TWR = 1.1 * 0.9 - 1 = -0.01
        assert!((twr - (-0.01)).abs() < 1e-9);
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
            ReturnDataPoint { date: "2024-01-01".to_string(), cumulative_return: 0.0, daily_return: 0.0, portfolio_value: 100.0, daily_pnl: 0.0 },
            ReturnDataPoint { date: "2024-01-02".to_string(), cumulative_return: 10.0, daily_return: 10.0, portfolio_value: 110.0, daily_pnl: 10.0 },
            ReturnDataPoint { date: "2024-01-03".to_string(), cumulative_return: -5.0, daily_return: -15.0, portfolio_value: 95.0, daily_pnl: -15.0 },
            ReturnDataPoint { date: "2024-01-04".to_string(), cumulative_return: 5.0, daily_return: 10.0, portfolio_value: 105.0, daily_pnl: 10.0 },
        ];
        let dd = calculate_max_drawdown(&series);
        // Peak = 110 on day 2, trough = 95 on day 3 → MDD = (95-110)/110 ≈ -13.6%
        assert!(dd.max_drawdown < 0.0);
        assert!((dd.max_drawdown - (-13.636_363_636)).abs() < 0.001);
        assert_eq!(dd.peak_date, "2024-01-02");
        assert_eq!(dd.trough_date, "2024-01-03");
    }

    #[test]
    fn test_build_return_series() {
        let daily = vec![
            (parse_date("2024-01-01").unwrap(), 100.0, 0.0),
            (parse_date("2024-01-02").unwrap(), 105.0, 5.0),
            (parse_date("2024-01-03").unwrap(), 103.0, -2.0),
        ];
        let series = build_return_series(&daily);
        assert_eq!(series.len(), 3);
        assert!((series[1].cumulative_return - 5.0).abs() < 1e-6);
        assert!((series[2].cumulative_return - 3.0).abs() < 1e-6);
    }
}
