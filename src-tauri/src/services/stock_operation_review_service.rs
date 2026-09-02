use crate::db::Database;
use crate::models::performance::BenchmarkDataPoint;
use crate::models::stock_operation_review::{
    StockOperationDataQuality, StockOperationEffect, StockOperationFieldIssue,
    StockOperationReviewQuery, StockOperationReviewReport,
};
use crate::models::ExchangeRates;
use crate::models::Transaction;
use crate::services::exchange_rate_service::convert_currency;
use crate::services::stock_operation_builder::{
    build_raw_stock_operations, normalize_stock_market, normalize_stock_symbol,
};
use crate::services::stock_operation_market_data::{
    default_benchmark_symbol, load_stock_price_series, upsert_stock_closes, DailyMarketPoint,
};
use crate::services::stock_operation_review_calculator::{
    calculate_directional_excess, calculate_endpoint_effect, summarize_actions,
    summarize_securities, EndpointEffectInput,
};
use crate::services::{
    performance_service, quote_provider_service, quote_service, snapshot_service,
};
use chrono::Utc;
use chrono::{Datelike, NaiveDate, Weekday};
use rusqlite::params;
use std::collections::{BTreeMap, HashMap};

const BENCHMARK_MAX_AGE_DAYS: i64 = 7;

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedBenchmarkWindow {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub return_value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedNav {
    pub date: NaiveDate,
    pub nav_base: f64,
    pub trade_fx_to_base: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ResolvedFx {
    pub date: NaiveDate,
    pub rate: f64,
}

pub fn validate_query(query: &StockOperationReviewQuery) -> Result<(), String> {
    if query.start_date > query.end_date {
        return Err("开始日期不能晚于结束日期。".to_string());
    }
    if !matches!(query.base_currency.as_str(), "USD" | "CNY" | "HKD") {
        return Err("基准币种必须是 USD、CNY 或 HKD。".to_string());
    }
    if query
        .market
        .as_deref()
        .is_some_and(|market| !matches!(market, "US" | "CN" | "HK"))
    {
        return Err("市场必须是 US、CN 或 HK。".to_string());
    }
    Ok(())
}

pub(crate) fn benchmark_symbol_for_market(market: &str) -> Option<&'static str> {
    normalize_stock_market(market)
        .as_deref()
        .and_then(default_benchmark_symbol)
}

pub(crate) fn project_action_seeds(
    transactions: &[Transaction],
    account_names: &HashMap<String, String>,
    query: &StockOperationReviewQuery,
) -> Vec<StockOperationEffect> {
    build_raw_stock_operations(transactions)
        .into_iter()
        .filter(|action| {
            action.trade_date >= query.start_date && action.trade_date <= query.end_date
        })
        .filter(|action| {
            query
                .account_id
                .as_ref()
                .is_none_or(|id| id == &action.account_id)
        })
        .filter(|action| {
            query.market.as_ref().is_none_or(|market| {
                normalize_stock_market(market) == normalize_stock_market(&action.market)
            })
        })
        .map(|action| StockOperationEffect {
            action_id: action.action_id,
            transaction_ids: action.transaction_ids,
            account_id: action.account_id.clone(),
            account_name: account_names
                .get(&action.account_id)
                .cloned()
                .unwrap_or_else(|| action.account_id.clone()),
            symbol: action.symbol,
            name: action.name,
            market: action.market.clone(),
            action_type: action.action_type,
            trade_date: action.trade_date,
            quantity: action.quantity,
            trade_price: action.trade_price,
            trade_notional_local: action.trade_notional_local,
            trade_notional_base: None,
            fee_local: action.fee_local,
            fee_base: None,
            currency: action.currency,
            shares_before: action.shares_before,
            shares_after: action.shares_after,
            prior_nav_date: None,
            prior_nav_base: None,
            weight_before: None,
            weight_after: None,
            weight_change: None,
            operation_size_ratio: None,
            evaluation_date: None,
            end_price: None,
            price_effect_local: None,
            price_effect_base: None,
            price_effect_percent: None,
            benchmark_symbol: benchmark_symbol_for_market(&action.market).map(str::to_string),
            benchmark_start_date: None,
            benchmark_end_date: None,
            benchmark_return: None,
            directional_excess_return: None,
            fact_labels: Vec::new(),
            issues: Vec::new(),
        })
        .collect()
}

pub(crate) fn security_history_key(symbol: &str, market: &str) -> (String, String) {
    (
        normalize_stock_market(market).unwrap_or_else(|| market.trim().to_ascii_uppercase()),
        normalize_stock_symbol(symbol).unwrap_or_else(|| symbol.trim().to_ascii_uppercase()),
    )
}

pub(crate) fn resolve_stock_endpoint(
    points: &[DailyMarketPoint],
    action_date: NaiveDate,
    report_end: NaiveDate,
) -> Option<(NaiveDate, f64)> {
    points
        .iter()
        .filter(|point| {
            point.date >= action_date
                && point.date <= report_end
                && point.close.is_finite()
                && point.close > 0.0
        })
        .max_by_key(|point| point.date)
        .map(|point| (point.date, point.close))
}

fn latest_benchmark_on_or_before(
    points: &[BenchmarkDataPoint],
    date: NaiveDate,
) -> Option<(NaiveDate, f64)> {
    points
        .iter()
        .filter_map(|point| {
            let point_date = NaiveDate::parse_from_str(&point.date, "%Y-%m-%d").ok()?;
            (point_date <= date && point.close_price.is_finite() && point.close_price > 0.0)
                .then_some((point_date, point.close_price))
        })
        .max_by_key(|(point_date, _)| *point_date)
}

pub(crate) fn resolve_benchmark_window(
    points: &[BenchmarkDataPoint],
    action_date: NaiveDate,
    evaluation_date: NaiveDate,
) -> Option<ResolvedBenchmarkWindow> {
    let (start_date, start_price) = latest_benchmark_on_or_before(points, action_date)?;
    let (end_date, end_price) = latest_benchmark_on_or_before(points, evaluation_date)?;
    let start_age = (action_date - start_date).num_days();
    let end_age = (evaluation_date - end_date).num_days();
    if !(0..=BENCHMARK_MAX_AGE_DAYS).contains(&start_age)
        || !(0..=BENCHMARK_MAX_AGE_DAYS).contains(&end_age)
        || end_date < start_date
    {
        return None;
    }
    let return_value = end_price / start_price - 1.0;
    return_value.is_finite().then_some(ResolvedBenchmarkWindow {
        start_date,
        end_date,
        return_value,
    })
}

fn parse_rates(value: &str) -> Option<ExchangeRates> {
    serde_json::from_str(value).ok()
}

fn fx_rate(currency: &str, base_currency: &str, rates: Option<&ExchangeRates>) -> Option<f64> {
    if currency == base_currency {
        return Some(1.0);
    }
    let value = convert_currency(1.0, currency, base_currency, rates?);
    (value.is_finite() && value > 0.0).then_some(value)
}

fn snapshot_currency<'a>(symbol: &'a str, market: &'a str) -> Option<&'a str> {
    if let Some(currency) = symbol.strip_prefix("$CASH-") {
        return matches!(currency, "USD" | "CNY" | "HKD").then_some(currency);
    }
    match market {
        "US" => Some("USD"),
        "CN" => Some("CNY"),
        "HK" => Some("HKD"),
        _ => None,
    }
}

pub(crate) fn resolve_fx_on_or_before(
    db: &Database,
    currency: &str,
    base_currency: &str,
    date: NaiveDate,
    max_age_days: i64,
) -> Result<Option<ResolvedFx>, String> {
    if currency == base_currency {
        return Ok(Some(ResolvedFx { date, rate: 1.0 }));
    }
    let start = date
        .checked_sub_signed(chrono::Duration::days(max_age_days.max(0)))
        .unwrap_or(date);
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT date, exchange_rates FROM daily_portfolio_values
             WHERE date BETWEEN ?1 AND ?2 ORDER BY date DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![
                start.format("%Y-%m-%d").to_string(),
                date.format("%Y-%m-%d").to_string()
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (row_date, rates_json) = row.map_err(|error| error.to_string())?;
        let Some(row_date) = NaiveDate::parse_from_str(&row_date, "%Y-%m-%d").ok() else {
            continue;
        };
        let rates = parse_rates(&rates_json);
        if let Some(rate) = fx_rate(currency, base_currency, rates.as_ref()) {
            return Ok(Some(ResolvedFx {
                date: row_date,
                rate,
            }));
        }
    }
    Ok(None)
}

pub(crate) fn resolve_prior_nav(
    db: &Database,
    account_id: Option<&str>,
    action_date: NaiveDate,
    trade_currency: &str,
    base_currency: &str,
) -> Result<Option<ResolvedNav>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    if account_id.is_none() {
        let mut statement = conn
            .prepare(
                "SELECT date, total_value, exchange_rates FROM daily_portfolio_values
                 WHERE date < ?1 ORDER BY date DESC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([action_date.format("%Y-%m-%d").to_string()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (row_date, total_value_usd, rates_json) = row.map_err(|error| error.to_string())?;
            let Some(row_date) = NaiveDate::parse_from_str(&row_date, "%Y-%m-%d").ok() else {
                continue;
            };
            let rates = parse_rates(&rates_json);
            let Some(usd_to_base) = fx_rate("USD", base_currency, rates.as_ref()) else {
                continue;
            };
            let nav_base = total_value_usd * usd_to_base;
            if !nav_base.is_finite() || nav_base <= 0.0 {
                continue;
            }
            return Ok(Some(ResolvedNav {
                date: row_date,
                nav_base,
                trade_fx_to_base: fx_rate(trade_currency, base_currency, rates.as_ref()),
            }));
        }
        return Ok(None);
    }

    let mut statement = conn
        .prepare(
            "SELECT snapshots.date, snapshots.symbol, snapshots.market,
                    snapshots.market_value, portfolio.exchange_rates
             FROM daily_holding_snapshots AS snapshots
             LEFT JOIN daily_portfolio_values AS portfolio ON portfolio.date = snapshots.date
             WHERE snapshots.account_id = ?1 AND snapshots.date < ?2
             ORDER BY snapshots.date DESC, snapshots.symbol ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![account_id, action_date.format("%Y-%m-%d").to_string()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?;
    type SnapshotRow = (String, String, f64);
    let mut by_date = BTreeMap::<NaiveDate, (Vec<SnapshotRow>, Option<String>)>::new();
    for row in rows {
        let (row_date, symbol, market, market_value, rates_json) =
            row.map_err(|error| error.to_string())?;
        let Some(row_date) = NaiveDate::parse_from_str(&row_date, "%Y-%m-%d").ok() else {
            continue;
        };
        let entry = by_date.entry(row_date).or_default();
        entry.0.push((symbol, market, market_value));
        if entry.1.is_none() {
            entry.1 = rates_json;
        }
    }
    for (row_date, (rows, rates_json)) in by_date.into_iter().rev() {
        if !rows
            .iter()
            .any(|(symbol, _, _)| symbol.starts_with("$CASH-"))
        {
            continue;
        }
        let rates = rates_json.as_deref().and_then(parse_rates);
        let converted = rows
            .iter()
            .map(|(symbol, market, market_value)| {
                let currency = snapshot_currency(symbol, market)?;
                let rate = fx_rate(currency, base_currency, rates.as_ref())?;
                let value = market_value * rate;
                value.is_finite().then_some(value)
            })
            .collect::<Option<Vec<_>>>();
        let Some(nav_base) = converted.map(|values| values.into_iter().sum::<f64>()) else {
            continue;
        };
        if !nav_base.is_finite() || nav_base <= 0.0 {
            continue;
        }
        return Ok(Some(ResolvedNav {
            date: row_date,
            nav_base,
            trade_fx_to_base: fx_rate(trade_currency, base_currency, rates.as_ref()),
        }));
    }
    Ok(None)
}

fn add_issue(action: &mut StockOperationEffect, code: &str, field: &str, message: &str) {
    action.issues.push(StockOperationFieldIssue {
        code: code.to_string(),
        field: field.to_string(),
        message: message.to_string(),
    });
}

fn add_outcome_labels(action: &mut StockOperationEffect) {
    if let Some(end_price) = action.end_price {
        let label = match action.action_type.as_str() {
            "open" | "add" if end_price > action.trade_price => "买入后上涨",
            "open" | "add" if end_price < action.trade_price => "买入后下跌",
            "open" | "add" => "买入后持平",
            "reduce" | "close" if end_price < action.trade_price => "卖出后下跌",
            "reduce" | "close" if end_price > action.trade_price => "卖出后继续上涨",
            "reduce" | "close" => "卖出后持平",
            _ => "",
        };
        if !label.is_empty() {
            action.fact_labels.push(label.to_string());
        }
    }
    if let Some(excess) = action.directional_excess_return {
        let label = match action.action_type.as_str() {
            "open" | "add" if excess >= 0.0 => "买入后跑赢基准",
            "open" | "add" => "买入后跑输基准",
            "reduce" | "close" if excess >= 0.0 => "卖出方向跑赢基准",
            "reduce" | "close" => "卖出方向跑输基准",
            _ => "",
        };
        if !label.is_empty() {
            action.fact_labels.push(label.to_string());
        }
    }
    if let Some(weight_change) = action.weight_change {
        if weight_change >= 0.05 {
            action.fact_labels.push("大幅提高仓位".to_string());
        } else if weight_change <= -0.05 {
            action.fact_labels.push("大幅降低仓位".to_string());
        }
    }
}

fn quality_for_actions(actions: &[StockOperationEffect]) -> StockOperationDataQuality {
    let missing_end_price_count = actions
        .iter()
        .filter(|action| action.end_price.is_none())
        .count();
    let missing_benchmark_count = actions
        .iter()
        .filter(|action| action.directional_excess_return.is_none())
        .count();
    let missing_fx_count = actions
        .iter()
        .filter(|action| {
            action.trade_notional_base.is_none()
                || (action.price_effect_local.is_some() && action.price_effect_base.is_none())
        })
        .count();
    let missing_weight_count = actions
        .iter()
        .filter(|action| action.weight_change.is_none())
        .count();
    let mut notes = Vec::new();
    if missing_end_price_count > 0 {
        notes.push(format!(
            "{missing_end_price_count} 项操作缺少期末行情，仅隐藏价格效果字段。"
        ));
    }
    if missing_benchmark_count > 0 {
        notes.push(format!(
            "{missing_benchmark_count} 项操作缺少基准端点，仅隐藏相对基准字段。"
        ));
    }
    if missing_fx_count > 0 {
        notes.push(format!(
            "{missing_fx_count} 项操作缺少汇率，仅隐藏基准币种金额字段。"
        ));
    }
    if missing_weight_count > 0 {
        notes.push(format!(
            "{missing_weight_count} 项操作缺少含现金的操作前总资产，仅隐藏权重估算字段。"
        ));
    }
    StockOperationDataQuality {
        action_count: actions.len(),
        missing_end_price_count,
        missing_benchmark_count,
        missing_fx_count,
        missing_weight_count,
        notes,
    }
}

pub(crate) fn assemble_report(
    db: &Database,
    query: StockOperationReviewQuery,
    mut actions: Vec<StockOperationEffect>,
    stock_histories: &HashMap<(String, String), Vec<DailyMarketPoint>>,
    benchmark_histories: &HashMap<String, Vec<BenchmarkDataPoint>>,
) -> Result<StockOperationReviewReport, String> {
    for action in &mut actions {
        let trade_fx = resolve_fx_on_or_before(
            db,
            &action.currency,
            &query.base_currency,
            action.trade_date,
            BENCHMARK_MAX_AGE_DAYS,
        )?;
        if let Some(trade_fx) = trade_fx {
            action.trade_notional_base = Some(action.trade_notional_local.abs() * trade_fx.rate);
            action.fee_base = Some(action.fee_local * trade_fx.rate);
        } else {
            add_issue(
                action,
                "missing_trade_fx",
                "trade_notional_base",
                "操作日附近缺少汇率，无法换算成交金额和费用。",
            );
        }

        let nav = resolve_prior_nav(
            db,
            query.account_id.as_deref(),
            action.trade_date,
            &action.currency,
            &query.base_currency,
        )?;
        if let Some(nav) = nav {
            action.prior_nav_date = Some(nav.date);
            action.prior_nav_base = Some(nav.nav_base);
            if let Some(trade_fx) = nav.trade_fx_to_base {
                let value_before = action.shares_before * action.trade_price * trade_fx;
                let value_after = action.shares_after * action.trade_price * trade_fx;
                action.weight_before = Some(value_before / nav.nav_base);
                action.weight_after = Some(value_after / nav.nav_base);
                action.weight_change = Some((value_after - value_before) / nav.nav_base);
                action.operation_size_ratio =
                    Some(action.trade_notional_local.abs() * trade_fx / nav.nav_base);
            }
        }
        if action.weight_change.is_none() {
            action.fact_labels.push("权重数据不足".to_string());
            add_issue(
                action,
                "missing_weight",
                "weight_change",
                "缺少含现金的操作前总资产或同日汇率，无法估算仓位变化。",
            );
        }

        let stock_points = stock_histories
            .get(&security_history_key(&action.symbol, &action.market))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if let Some((evaluation_date, end_price)) =
            resolve_stock_endpoint(stock_points, action.trade_date, query.end_date)
        {
            action.evaluation_date = Some(evaluation_date);
            action.end_price = Some(end_price);
            let output = calculate_endpoint_effect(&EndpointEffectInput {
                action_type: action.action_type.clone(),
                quantity: action.quantity,
                trade_price: action.trade_price,
                trade_notional_local: action.trade_notional_local,
                end_price: Some(end_price),
                fee_local: action.fee_local,
            });
            action.price_effect_local = output.price_effect_local;
            action.price_effect_percent = output.price_effect_percent;
            let endpoint_fx = resolve_fx_on_or_before(
                db,
                &action.currency,
                &query.base_currency,
                evaluation_date,
                BENCHMARK_MAX_AGE_DAYS,
            )?;
            if let (Some(effect), Some(endpoint_fx)) = (action.price_effect_local, endpoint_fx) {
                action.price_effect_base = Some(effect * endpoint_fx.rate);
            } else if action.price_effect_local.is_some() {
                add_issue(
                    action,
                    "missing_endpoint_fx",
                    "price_effect_base",
                    "评价日附近缺少汇率，保留本币价格效果。",
                );
            }

            if let Some(benchmark_symbol) = action.benchmark_symbol.as_ref() {
                if let Some(window) = benchmark_histories
                    .get(benchmark_symbol)
                    .and_then(|points| {
                        resolve_benchmark_window(points, action.trade_date, evaluation_date)
                    })
                {
                    action.benchmark_start_date = Some(window.start_date);
                    action.benchmark_end_date = Some(window.end_date);
                    action.benchmark_return = Some(window.return_value);
                    let stock_return = end_price / action.trade_price - 1.0;
                    action.directional_excess_return = calculate_directional_excess(
                        &action.action_type,
                        stock_return,
                        window.return_value,
                    );
                }
            }
        } else {
            action.fact_labels.push("期末行情不足".to_string());
            add_issue(
                action,
                "missing_end_price",
                "end_price",
                "操作日至复盘期末没有可用的真实收盘价。",
            );
        }
        if action.directional_excess_return.is_none() {
            action.fact_labels.push("基准数据不足".to_string());
            add_issue(
                action,
                "missing_benchmark",
                "directional_excess_return",
                "自动市场基准缺少七日内可用端点。",
            );
        }
        add_outcome_labels(action);
    }

    let summary = summarize_actions(&actions);
    let securities = summarize_securities(&actions);
    let data_quality = quality_for_actions(&actions);
    Ok(StockOperationReviewReport {
        query,
        summary,
        securities,
        actions,
        data_quality,
        generated_at: Utc::now().to_rfc3339(),
        algorithm_version: "stock-operation-review-lite-v1".to_string(),
    })
}

pub fn scope_report_to_symbol(
    mut report: StockOperationReviewReport,
    symbol: &str,
) -> StockOperationReviewReport {
    report
        .actions
        .retain(|action| normalize_stock_symbol(&action.symbol) == normalize_stock_symbol(symbol));
    report.summary = summarize_actions(&report.actions);
    report.securities = summarize_securities(&report.actions);
    report.data_quality = quality_for_actions(&report.actions);
    report
}

fn load_transactions_through(
    db: &Database,
    end_date: NaiveDate,
) -> Result<Vec<Transaction>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT id, holding_id, account_id, symbol, name, market, transaction_type,
                    shares, price, total_amount, commission, currency, traded_at, notes, created_at
             FROM transactions WHERE substr(traded_at, 1, 10) <= ?1
             ORDER BY traded_at ASC, created_at ASC, id ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([end_date.format("%Y-%m-%d").to_string()], |row| {
            Ok(Transaction {
                id: row.get(0)?,
                holding_id: row.get(1)?,
                account_id: row.get(2)?,
                symbol: row.get(3)?,
                name: row.get(4)?,
                market: row.get(5)?,
                transaction_type: row.get(6)?,
                shares: row.get(7)?,
                price: row.get(8)?,
                total_amount: row.get(9)?,
                commission: row.get(10)?,
                currency: row.get(11)?,
                traded_at: row.get(12)?,
                notes: row.get(13)?,
                created_at: row.get(14)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn load_account_names(
    db: &Database,
    selected_account_id: Option<&str>,
) -> Result<HashMap<String, String>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare("SELECT id, name FROM accounts ORDER BY id ASC")
        .map_err(|error| error.to_string())?;
    let names = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|error| error.to_string())?;
    if let Some(account_id) = selected_account_id {
        if !names.contains_key(account_id) {
            return Err(format!("账户 {account_id} 不存在。"));
        }
    }
    Ok(names)
}

fn provider_for_market<'a>(
    config: &'a crate::models::quote_provider::QuoteProviderConfig,
    market: &str,
) -> &'a str {
    match market {
        "CN" => &config.cn_provider,
        "HK" => &config.hk_provider,
        _ => &config.us_provider,
    }
}

fn stock_history_request_range(
    cached: &[DailyMarketPoint],
    action_start: NaiveDate,
    report_end: NaiveDate,
    latest_closed_market_date: NaiveDate,
) -> Option<(NaiveDate, NaiveDate)> {
    let mut fetch_end = report_end.min(latest_closed_market_date);
    while matches!(fetch_end.weekday(), Weekday::Sat | Weekday::Sun) {
        fetch_end = fetch_end.pred_opt()?;
    }
    if action_start > fetch_end
        || cached
            .iter()
            .any(|point| point.date == fetch_end && point.close.is_finite() && point.close > 0.0)
    {
        return None;
    }
    Some((action_start, fetch_end))
}

async fn load_stock_histories(
    db: &Database,
    quote_state: Option<&quote_service::QuoteServiceState>,
    actions: &[StockOperationEffect],
    report_end: NaiveDate,
    refresh: bool,
) -> Result<HashMap<(String, String), Vec<DailyMarketPoint>>, String> {
    let mut requests = BTreeMap::<(String, String), (String, String, NaiveDate)>::new();
    for action in actions {
        let key = security_history_key(&action.symbol, &action.market);
        requests
            .entry(key)
            .and_modify(|request| request.2 = request.2.min(action.trade_date))
            .or_insert_with(|| {
                (
                    action.symbol.clone(),
                    action.market.clone(),
                    action.trade_date,
                )
            });
    }
    let config = quote_provider_service::get_quote_provider_config(db)?;
    let latest_closed_market_date = snapshot_service::last_closed_market_date();
    let mut histories = HashMap::new();
    for (key, (symbol, market, start_date)) in requests {
        let points = if refresh {
            let cached = load_stock_price_series(db, &symbol, &market, start_date, report_end)?;
            if let Some((fetch_start, fetch_end)) = stock_history_request_range(
                &cached,
                start_date,
                report_end,
                latest_closed_market_date,
            ) {
                let provider = provider_for_market(&config, &market);
                let quote_state = quote_state.ok_or_else(|| {
                    "quote service state is required when refreshing stock history".to_string()
                })?;
                if let Ok(prices) = quote_service::fetch_stock_history(
                    quote_state,
                    &symbol,
                    &market,
                    fetch_start,
                    fetch_end,
                    provider,
                )
                .await
                {
                    upsert_stock_closes(db, &symbol, &market, provider, &prices)?;
                }
            }
            load_stock_price_series(db, &symbol, &market, start_date, report_end)?
        } else {
            load_stock_price_series(db, &symbol, &market, start_date, report_end)?
        };
        histories.insert(key, points);
    }
    Ok(histories)
}

async fn load_benchmark_histories(
    db: &Database,
    actions: &[StockOperationEffect],
    report_end: NaiveDate,
    refresh: bool,
) -> Result<HashMap<String, Vec<BenchmarkDataPoint>>, String> {
    let mut requests = BTreeMap::<String, NaiveDate>::new();
    for action in actions {
        if let Some(symbol) = &action.benchmark_symbol {
            requests
                .entry(symbol.clone())
                .and_modify(|date| *date = (*date).min(action.trade_date))
                .or_insert(action.trade_date);
        }
    }
    let mut histories = HashMap::new();
    for (symbol, earliest_action_date) in requests {
        let start_date = earliest_action_date
            .checked_sub_signed(chrono::Duration::days(BENCHMARK_MAX_AGE_DAYS))
            .unwrap_or(earliest_action_date);
        let cached =
            performance_service::read_cached_benchmark(db, &symbol, start_date, report_end)?;
        let points = if refresh {
            performance_service::fetch_benchmark_history(db, &symbol, start_date, report_end)
                .await
                .unwrap_or(cached)
        } else {
            cached
        };
        histories.insert(symbol, points);
    }
    Ok(histories)
}

pub(crate) async fn get_stock_operation_review_with_refresh(
    db: &Database,
    quote_state: Option<&quote_service::QuoteServiceState>,
    query: StockOperationReviewQuery,
    refresh_market_data: bool,
) -> Result<StockOperationReviewReport, String> {
    validate_query(&query)?;
    let account_names = load_account_names(db, query.account_id.as_deref())?;
    let transactions = load_transactions_through(db, query.end_date)?;
    let actions = project_action_seeds(&transactions, &account_names, &query);
    let stock_histories = load_stock_histories(
        db,
        quote_state,
        &actions,
        query.end_date,
        refresh_market_data,
    )
    .await?;
    let benchmark_histories =
        load_benchmark_histories(db, &actions, query.end_date, refresh_market_data).await?;
    assemble_report(db, query, actions, &stock_histories, &benchmark_histories)
}

pub async fn get_stock_operation_review(
    db: &Database,
    quote_state: &quote_service::QuoteServiceState,
    query: StockOperationReviewQuery,
) -> Result<StockOperationReviewReport, String> {
    get_stock_operation_review_with_refresh(db, Some(quote_state), query, true).await
}

#[cfg(test)]
mod tests {
    use super::{
        assemble_report, benchmark_symbol_for_market, get_stock_operation_review_with_refresh,
        project_action_seeds, resolve_benchmark_window, resolve_fx_on_or_before, resolve_prior_nav,
        resolve_stock_endpoint, scope_report_to_symbol, security_history_key,
        stock_history_request_range, validate_query,
    };
    use crate::db::Database;
    use crate::models::performance::BenchmarkDataPoint;
    use crate::models::stock_operation_review::StockOperationReviewQuery;
    use crate::models::ExchangeRates;
    use crate::models::Transaction;
    use crate::services::stock_operation_market_data::DailyMarketPoint;
    use chrono::NaiveDate;
    use std::collections::HashMap;

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn query() -> StockOperationReviewQuery {
        StockOperationReviewQuery {
            start_date: date("2026-07-01"),
            end_date: date("2026-07-31"),
            account_id: None,
            market: None,
            base_currency: "USD".to_string(),
        }
    }

    fn transaction(
        id: &str,
        transaction_type: &str,
        shares: f64,
        price: f64,
        traded_at: &str,
    ) -> Transaction {
        Transaction {
            id: id.to_string(),
            holding_id: None,
            account_id: "account-1".to_string(),
            symbol: "AAPL".to_string(),
            name: "Apple".to_string(),
            market: "US".to_string(),
            transaction_type: transaction_type.to_string(),
            shares,
            price,
            total_amount: shares * price,
            commission: 1.0,
            currency: "USD".to_string(),
            traded_at: traded_at.to_string(),
            notes: None,
            created_at: traded_at.to_string(),
        }
    }

    fn close(date: &str, value: f64) -> DailyMarketPoint {
        DailyMarketPoint {
            date: self::date(date),
            close: value,
        }
    }

    fn benchmark(date: &str, value: f64) -> BenchmarkDataPoint {
        BenchmarkDataPoint {
            date: date.to_string(),
            close_price: value,
            change_percent: 0.0,
        }
    }

    fn rates_json(date: &str) -> String {
        serde_json::to_string(&ExchangeRates {
            usd_cny: 7.0,
            usd_hkd: 7.8,
            cny_hkd: 1.1142857142857143,
            updated_at: format!("{date}T00:00:00Z"),
        })
        .unwrap()
    }

    fn insert_portfolio_day(db: &Database, day: &str, total_value_usd: f64) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO daily_portfolio_values (date, total_value, exchange_rates) VALUES (?1, ?2, ?3)",
                rusqlite::params![day, total_value_usd, rates_json(day)],
            )
            .unwrap();
    }

    fn insert_snapshot(
        db: &Database,
        day: &str,
        account_id: &str,
        symbol: &str,
        market: &str,
        market_value: f64,
    ) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO daily_holding_snapshots
                 (date, account_id, symbol, market, market_value)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                rusqlite::params![day, account_id, symbol, market, market_value],
            )
            .unwrap();
    }

    fn insert_account(db: &Database, account_id: &str, market: &str) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
                 VALUES (?1, ?2, ?3, '2026-01-01', '2026-01-01')",
                rusqlite::params![account_id, format!("账户-{account_id}"), market],
            )
            .unwrap();
    }

    fn seeded_operation_db() -> Database {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at)
             VALUES ('account-1', '主账户', 'US', '2026-01-01', '2026-01-01')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO transactions
                (id, account_id, symbol, name, market, transaction_type, shares,
                 price, total_amount, commission, currency, traded_at, created_at)
             VALUES
                ('buy', 'account-1', 'AAPL', 'Apple', 'US', 'BUY', 10,
                 100, 1000, 1, 'USD', '2026-07-03T10:00:00Z', '2026-07-03T10:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO stock_daily_prices
                (symbol, market, date, close, source, updated_at)
             VALUES ('AAPL', 'US', '2026-07-03', 100, 'test', '2026-07-03T10:00:00Z')",
            [],
        )
        .unwrap();
        drop(conn);
        db
    }

    #[test]
    fn query_validation_rejects_invalid_ranges_markets_and_currencies() {
        assert!(validate_query(&query()).is_ok());

        let mut invalid = query();
        invalid.start_date = date("2026-08-01");
        assert!(validate_query(&invalid).unwrap_err().contains("开始日期"));

        let mut invalid = query();
        invalid.market = Some("JP".to_string());
        assert!(validate_query(&invalid).unwrap_err().contains("市场"));

        let mut invalid = query();
        invalid.base_currency = "EUR".to_string();
        assert!(validate_query(&invalid).unwrap_err().contains("基准币种"));
    }

    #[test]
    fn complete_history_replay_sets_shares_but_only_range_actions_are_returned() {
        let transactions = vec![
            transaction("opening", "BUY", 100.0, 10.0, "2026-06-01T10:00:00Z"),
            transaction("add-1", "BUY", 20.0, 11.0, "2026-07-02T10:00:00Z"),
            transaction("add-2", "BUY", 30.0, 12.0, "2026-07-02T11:00:00Z"),
            transaction("reduce", "SELL", 50.0, 13.0, "2026-07-10T10:00:00Z"),
            transaction("close", "SELL", 100.0, 14.0, "2026-07-20T10:00:00Z"),
        ];
        let names = HashMap::from([("account-1".to_string(), "主账户".to_string())]);
        let actions = project_action_seeds(&transactions, &names, &query());
        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].action_type, "add");
        assert_eq!(actions[0].quantity, 50.0);
        assert!((actions[0].trade_price - 11.6).abs() < 1e-12);
        assert_eq!(actions[0].trade_notional_local, 580.0);
        assert_eq!(actions[0].fee_local, 2.0);
        assert_eq!(actions[0].shares_before, 100.0);
        assert_eq!(actions[0].shares_after, 150.0);
        assert_eq!(actions[1].action_type, "reduce");
        assert_eq!(actions[1].shares_before, 150.0);
        assert_eq!(actions[1].shares_after, 100.0);
        assert_eq!(actions[2].action_type, "close");
        assert_eq!(actions[2].shares_before, 100.0);
        assert_eq!(actions[2].shares_after, 0.0);
        assert_eq!(actions[0].account_name, "主账户");
        assert_eq!(actions[0].name, "Apple");
    }

    #[test]
    fn action_projection_excludes_non_trade_rows_and_applies_filters_after_replay() {
        let mut cash = transaction("cash", "BUY", 1_000.0, 1.0, "2026-06-01T09:00:00Z");
        cash.symbol = "$CASH-USD".to_string();
        let mut pay = transaction("pay", "PAY", 1.0, 5.0, "2026-07-03T09:00:00Z");
        pay.symbol = "AAPL".to_string();
        let mut cn = transaction("cn", "BUY", 10.0, 20.0, "2026-07-04T09:00:00Z");
        cn.symbol = "600000".to_string();
        cn.name = "浦发银行".to_string();
        cn.market = "CN".to_string();
        cn.currency = "CNY".to_string();
        let transactions = vec![
            cash,
            transaction("opening", "BUY", 100.0, 10.0, "2026-06-01T10:00:00Z"),
            transaction("add", "BUY", 20.0, 11.0, "2026-07-02T10:00:00Z"),
            pay,
            cn,
        ];
        let mut filtered = query();
        filtered.market = Some("CN".to_string());
        let actions = project_action_seeds(&transactions, &HashMap::new(), &filtered);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].symbol, "600000");
        assert_eq!(actions[0].action_type, "open");
    }

    #[test]
    fn stock_endpoint_uses_last_real_close_between_action_and_report_end() {
        let points = vec![
            close("2026-06-30", 9.0),
            close("2026-07-02", 10.0),
            close("2026-07-31", 12.0),
            close("2026-08-03", 13.0),
        ];
        assert_eq!(
            resolve_stock_endpoint(&points, date("2026-07-01"), date("2026-08-02")),
            Some((date("2026-07-31"), 12.0))
        );
    }

    #[test]
    fn newly_listed_stock_uses_first_available_post_action_history() {
        let points = vec![close("2026-07-02", 20.0), close("2026-07-31", 24.0)];
        assert_eq!(
            resolve_stock_endpoint(&points, date("2026-06-30"), date("2026-07-31")),
            Some((date("2026-07-31"), 24.0))
        );
        assert_eq!(
            resolve_stock_endpoint(&points, date("2026-08-01"), date("2026-08-30")),
            None
        );
    }

    #[test]
    fn stock_history_request_skips_a_weekend_tail_already_covered_by_friday_close() {
        let cached = vec![close("2026-08-27", 100.0), close("2026-08-28", 101.0)];

        assert_eq!(
            stock_history_request_range(
                &cached,
                date("2026-07-01"),
                date("2026-08-30"),
                date("2026-08-30"),
            ),
            None
        );
    }

    #[test]
    fn stock_history_request_uses_the_full_performance_window_not_a_trailing_gap() {
        let cached = vec![close("2026-08-27", 100.0)];

        assert_eq!(
            stock_history_request_range(
                &cached,
                date("2026-07-01"),
                date("2026-08-30"),
                date("2026-08-30"),
            ),
            Some((date("2026-07-01"), date("2026-08-28")))
        );
    }

    #[test]
    fn benchmark_window_uses_on_or_before_points_with_seven_day_limit() {
        let points = vec![
            benchmark("2026-06-30", 100.0),
            benchmark("2026-07-01", 101.0),
            benchmark("2026-07-31", 110.0),
        ];
        let resolved =
            resolve_benchmark_window(&points, date("2026-07-02"), date("2026-08-02")).unwrap();
        assert_eq!(resolved.start_date, date("2026-07-01"));
        assert_eq!(resolved.end_date, date("2026-07-31"));
        assert!((resolved.return_value - (110.0 / 101.0 - 1.0)).abs() < 1e-12);

        assert!(
            resolve_benchmark_window(&points, date("2026-07-20"), date("2026-08-20")).is_none()
        );
    }

    #[test]
    fn automatic_benchmarks_are_fixed_by_market() {
        assert_eq!(benchmark_symbol_for_market("CN"), Some("000300.SS"));
        assert_eq!(benchmark_symbol_for_market("HK"), Some("^HSI"));
        assert_eq!(benchmark_symbol_for_market("US"), Some("^GSPC"));
        assert_eq!(benchmark_symbol_for_market("JP"), None);
    }

    #[test]
    fn all_account_prior_nav_uses_global_total_and_snapshot_fx() {
        let db = Database::new(":memory:").unwrap();
        insert_portfolio_day(&db, "2026-07-01", 1_000.0);
        let nav = resolve_prior_nav(&db, None, date("2026-07-02"), "USD", "CNY")
            .unwrap()
            .unwrap();
        assert_eq!(nav.date, date("2026-07-01"));
        assert!((nav.nav_base - 7_000.0).abs() < 1e-12);
        assert_eq!(nav.trade_fx_to_base, Some(7.0));
    }

    #[test]
    fn selected_account_nav_includes_cash_and_all_markets() {
        let db = Database::new(":memory:").unwrap();
        insert_portfolio_day(&db, "2026-07-01", 2_000.0);
        insert_snapshot(&db, "2026-07-01", "account-1", "AAPL", "US", 500.0);
        insert_snapshot(&db, "2026-07-01", "account-1", "$CASH-USD", "US", 500.0);
        insert_snapshot(&db, "2026-07-01", "account-1", "600000", "CN", 100.0);
        insert_snapshot(&db, "2026-07-01", "account-2", "MSFT", "US", 9_999.0);

        let nav = resolve_prior_nav(&db, Some("account-1"), date("2026-07-02"), "USD", "CNY")
            .unwrap()
            .unwrap();
        assert!((nav.nav_base - 7_100.0).abs() < 1e-12);
        assert_eq!(nav.trade_fx_to_base, Some(7.0));
    }

    #[test]
    fn selected_account_nav_skips_newer_snapshot_without_cash() {
        let db = Database::new(":memory:").unwrap();
        insert_portfolio_day(&db, "2026-07-01", 2_000.0);
        insert_snapshot(&db, "2026-07-01", "account-1", "AAPL", "US", 500.0);
        insert_snapshot(&db, "2026-07-01", "account-1", "$CASH-USD", "US", 500.0);
        insert_portfolio_day(&db, "2026-07-02", 2_100.0);
        insert_snapshot(&db, "2026-07-02", "account-1", "AAPL", "US", 600.0);

        let nav = resolve_prior_nav(&db, Some("account-1"), date("2026-07-03"), "USD", "USD")
            .unwrap()
            .unwrap();
        assert_eq!(nav.date, date("2026-07-01"));
        assert_eq!(nav.nav_base, 1_000.0);
    }

    #[test]
    fn selected_account_nav_is_missing_when_no_snapshot_has_cash() {
        let db = Database::new(":memory:").unwrap();
        insert_portfolio_day(&db, "2026-07-01", 2_000.0);
        insert_snapshot(&db, "2026-07-01", "account-1", "AAPL", "US", 500.0);
        assert_eq!(
            resolve_prior_nav(&db, Some("account-1"), date("2026-07-02"), "USD", "USD",).unwrap(),
            None
        );
    }

    #[test]
    fn endpoint_fx_uses_recent_prior_snapshot_but_rejects_stale_data() {
        let db = Database::new(":memory:").unwrap();
        insert_portfolio_day(&db, "2026-07-31", 2_000.0);
        let resolved = resolve_fx_on_or_before(&db, "USD", "CNY", date("2026-08-02"), 7)
            .unwrap()
            .unwrap();
        assert_eq!(resolved.date, date("2026-07-31"));
        assert_eq!(resolved.rate, 7.0);
        assert_eq!(
            resolve_fx_on_or_before(&db, "USD", "CNY", date("2026-08-10"), 7,).unwrap(),
            None
        );
        let identity = resolve_fx_on_or_before(&db, "CNY", "CNY", date("2026-08-10"), 7)
            .unwrap()
            .unwrap();
        assert_eq!(identity.date, date("2026-08-10"));
        assert_eq!(identity.rate, 1.0);
    }

    #[test]
    fn assembled_report_calculates_endpoint_benchmark_weight_and_quality() {
        let db = Database::new(":memory:").unwrap();
        insert_portfolio_day(&db, "2026-07-01", 1_000.0);
        let actions = project_action_seeds(
            &[transaction(
                "buy",
                "BUY",
                100.0,
                10.0,
                "2026-07-02T10:00:00Z",
            )],
            &HashMap::new(),
            &query(),
        );
        let stock_histories = HashMap::from([(
            security_history_key("AAPL", "US"),
            vec![close("2026-07-02", 10.0), close("2026-07-31", 12.0)],
        )]);
        let benchmark_histories = HashMap::from([(
            "^GSPC".to_string(),
            vec![
                benchmark("2026-07-02", 100.0),
                benchmark("2026-07-31", 110.0),
            ],
        )]);

        let report = assemble_report(
            &db,
            query(),
            actions,
            &stock_histories,
            &benchmark_histories,
        )
        .unwrap();
        assert_eq!(report.actions.len(), 1);
        let action = &report.actions[0];
        assert_eq!(action.evaluation_date, Some(date("2026-07-31")));
        assert_eq!(action.end_price, Some(12.0));
        assert_eq!(action.price_effect_local, Some(199.0));
        assert_eq!(action.price_effect_base, Some(199.0));
        assert_eq!(action.price_effect_percent, Some(0.199));
        assert!((action.benchmark_return.unwrap() - 0.10).abs() < 1e-12);
        assert!((action.directional_excess_return.unwrap() - 0.10).abs() < 1e-12);
        assert_eq!(action.prior_nav_date, Some(date("2026-07-01")));
        assert_eq!(action.weight_before, Some(0.0));
        assert_eq!(action.weight_after, Some(1.0));
        assert_eq!(action.weight_change, Some(1.0));
        assert_eq!(action.operation_size_ratio, Some(1.0));
        assert!(action.fact_labels.contains(&"买入后上涨".to_string()));
        assert!(action.fact_labels.contains(&"买入后跑赢基准".to_string()));
        assert!(action.fact_labels.contains(&"大幅提高仓位".to_string()));
        assert_eq!(report.summary.total.price_effect_base, Some(199.0));
        assert_eq!(report.securities.len(), 1);
        assert_eq!(report.data_quality.missing_end_price_count, 0);
        assert_eq!(report.data_quality.missing_benchmark_count, 0);
        assert_eq!(report.data_quality.missing_fx_count, 0);
        assert_eq!(report.data_quality.missing_weight_count, 0);
    }

    #[test]
    fn assembled_report_keeps_rows_when_endpoint_benchmark_and_weight_are_missing() {
        let db = Database::new(":memory:").unwrap();
        let actions = project_action_seeds(
            &[transaction(
                "buy",
                "BUY",
                100.0,
                10.0,
                "2026-07-02T10:00:00Z",
            )],
            &HashMap::new(),
            &query(),
        );
        let report =
            assemble_report(&db, query(), actions, &HashMap::new(), &HashMap::new()).unwrap();
        assert_eq!(report.actions.len(), 1);
        let action = &report.actions[0];
        assert_eq!(action.trade_notional_base, Some(1_000.0));
        assert_eq!(action.price_effect_local, None);
        assert_eq!(action.directional_excess_return, None);
        assert_eq!(action.weight_change, None);
        assert!(action
            .issues
            .iter()
            .any(|issue| issue.code == "missing_end_price"));
        assert!(action
            .issues
            .iter()
            .any(|issue| issue.code == "missing_benchmark"));
        assert!(action
            .issues
            .iter()
            .any(|issue| issue.code == "missing_weight"));
        assert_eq!(report.summary.total.missing_effect_count, 1);
        assert_eq!(report.data_quality.missing_end_price_count, 1);
        assert_eq!(report.data_quality.missing_benchmark_count, 1);
        assert_eq!(report.data_quality.missing_fx_count, 0);
        assert_eq!(report.data_quality.missing_weight_count, 1);
        assert!(!report.data_quality.notes.is_empty());
    }

    #[tokio::test]
    async fn orchestration_returns_empty_report_without_refreshing_network_data() {
        let db = Database::new(":memory:").unwrap();
        let report = get_stock_operation_review_with_refresh(&db, None, query(), false)
            .await
            .unwrap();
        assert!(report.actions.is_empty());
        assert!(report.securities.is_empty());
        assert_eq!(report.data_quality.action_count, 0);
        assert_eq!(report.algorithm_version, "stock-operation-review-lite-v1");
    }

    #[tokio::test]
    async fn legacy_override_rows_never_change_raw_operation_results() {
        let db = seeded_operation_db();
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS stock_review_overrides (
                    id TEXT PRIMARY KEY,
                    override_type TEXT NOT NULL,
                    transaction_ids_json TEXT NOT NULL,
                    value_json TEXT NOT NULL,
                    reference_fingerprint_json TEXT NOT NULL,
                    created_at TEXT NOT NULL,
                    updated_at TEXT NOT NULL
                 );
                 DELETE FROM stock_review_overrides;
                 INSERT INTO stock_review_overrides
                    (id, override_type, transaction_ids_json, value_json,
                     reference_fingerprint_json, created_at, updated_at)
                 VALUES (
                    'legacy', 'non_trade', '[\"buy\"]', '{}', '[]', '2026-07-03', '2026-07-03'
                 );",
            )
            .unwrap();

        let report = get_stock_operation_review_with_refresh(&db, None, query(), false)
            .await
            .unwrap();
        assert_eq!(report.actions.len(), 1);
        assert_eq!(report.actions[0].transaction_ids, vec!["buy".to_string()]);
    }

    #[tokio::test]
    async fn orchestration_rejects_unknown_selected_account() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "known", "US");
        let mut selected = query();
        selected.account_id = Some("missing".to_string());
        let error = get_stock_operation_review_with_refresh(&db, None, selected, false)
            .await
            .unwrap_err();
        assert!(error.contains("账户"));
        assert!(error.contains("不存在"));
    }

    #[test]
    fn symbol_scope_recalculates_lightweight_summaries_and_quality() {
        let db = Database::new(":memory:").unwrap();
        let mut second = transaction("second", "BUY", 50.0, 20.0, "2026-07-03T10:00:00Z");
        second.symbol = "MSFT".to_string();
        second.name = "Microsoft".to_string();
        let actions = project_action_seeds(
            &[
                transaction("first", "BUY", 100.0, 10.0, "2026-07-02T10:00:00Z"),
                second,
            ],
            &HashMap::new(),
            &query(),
        );
        let report =
            assemble_report(&db, query(), actions, &HashMap::new(), &HashMap::new()).unwrap();
        let scoped = scope_report_to_symbol(report, " aapl ");
        assert_eq!(scoped.actions.len(), 1);
        assert_eq!(scoped.actions[0].symbol, "AAPL");
        assert_eq!(scoped.securities.len(), 1);
        assert_eq!(scoped.summary.total.action_count, 1);
        assert_eq!(scoped.data_quality.action_count, 1);
        assert_eq!(scoped.data_quality.missing_end_price_count, 1);
    }
}
