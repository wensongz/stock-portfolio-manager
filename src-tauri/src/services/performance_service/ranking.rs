use super::calculation::{parse_required_exchange_rates, PerformanceCalculation};
use super::PerformanceFilter;
use crate::db::Database;
use crate::models::performance::HoldingPerformance;
use chrono::NaiveDate;

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

pub(super) fn holding_performance_ranking_from(
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
        sql = sql.replace(
            "t.total_amount,",
            &format!("{},", super::TRANSFER_VALUE_SQL),
        );
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
                    row.get::<_, Option<f64>>(4)?,
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
            let amount = super::require_flow_value(amount)?;
            let native_flow = match tx_type.as_str() {
                "BUY" => amount + commission,
                "SELL" | "PAY" => -(amount - commission),
                "OPEN" | "STOCK_IN" => amount + commission,
                "STOCK_OUT" => -amount,
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
