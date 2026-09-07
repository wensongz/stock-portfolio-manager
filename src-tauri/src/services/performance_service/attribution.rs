use super::calculation::{parse_required_exchange_rates, PerformanceCalculation};
use super::PerformanceFilter;
use crate::db::Database;
use crate::models::performance::{AttributionItem, ReturnAttribution};
use chrono::NaiveDate;

pub fn get_return_attribution(
    db: &Database,
    start_date: NaiveDate,
    end_date: NaiveDate,
    filter: &PerformanceFilter,
) -> Result<ReturnAttribution, String> {
    let calculation = PerformanceCalculation::load(db, start_date, end_date, filter)?;
    return_attribution_from(db, &calculation, filter)
}

pub(super) fn return_attribution_from(
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
