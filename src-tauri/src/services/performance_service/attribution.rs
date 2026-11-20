use super::calculation::{
    convert_performance_amount, market_currency, valuation_rates, PerformanceCalculation,
};
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
    let start_rates = valuation_rates(&conn, &start_str)?;
    let end_rates = valuation_rates(&conn, &end_str)?;
    let currency = calculation.currency;

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
        // Read the exact opening valuation date from the shared report window.
        let mut sql = String::from(
            "SELECT account_id, symbol, market, COALESCE(category_name, '未分类'),
                    SUM(market_value)
             FROM daily_holding_snapshots
             WHERE date = ?1 AND UPPER(symbol) NOT LIKE '$CASH-%'",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(start_str.clone())];
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
            let val = convert_performance_amount(
                val,
                market_currency(&market),
                currency,
                start_rates.as_deref(),
                &start_str,
            )?;
            *start_vals.entry(key).or_insert(0.0) += val;
        }
    }

    {
        // A position absent at the final valuation is worth zero.
        let mut sql = String::from(
            "SELECT account_id, symbol, market, COALESCE(category_name, '未分类'),
                    SUM(market_value)
             FROM daily_holding_snapshots
             WHERE date = ?1 AND UPPER(symbol) NOT LIKE '$CASH-%'",
        );
        let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = vec![Box::new(end_str.clone())];
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
            let val = convert_performance_amount(
                val,
                market_currency(&market),
                currency,
                end_rates.as_deref(),
                &end_str,
            )?;
            *end_vals.entry(key).or_insert(0.0) += val;
        }
    }

    // Fetch net cash flows per symbol from transactions during the period.
    // BUY  → positive cash flow (money invested into the holding)
    // SELL → negative cash flow (money withdrawn from the holding)
    let mut net_cash_flows: std::collections::HashMap<PositionKey, f64> =
        std::collections::HashMap::new();
    {
        let mut sql = format!(
            "SELECT t.account_id, t.symbol, t.market, t.transaction_type,
                    t.total_amount, t.commission, t.currency, {},
                    (SELECT d.exchange_rates
                       FROM daily_portfolio_values d
                      WHERE d.date >= DATE(t.traded_at) AND d.date <= ?2
                      ORDER BY d.date ASC LIMIT 1)
             FROM transactions t
             WHERE DATE(t.traded_at) > ?1 AND DATE(t.traded_at) <= ?2
               AND UPPER(t.symbol) NOT LIKE '$CASH-%'",
            super::TRANSFER_VALUE_SQL,
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
                    row.get::<_, Option<f64>>(7)?,
                    row.get::<_, Option<String>>(8)?,
                ))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        for (
            account_id,
            symbol,
            market,
            tx_type,
            amount,
            commission,
            native_currency,
            market_value,
            rates_json,
        ) in rows
        {
            let (amount, native_currency) = super::resolve_transaction_value(
                &tx_type,
                amount,
                &native_currency,
                &market,
                market_value,
            )?;
            let native_flow = match tx_type.as_str() {
                "BUY" => amount + commission,
                "SELL" | "PAY" => -(amount - commission),
                "OPEN" | "STOCK_IN" => amount + commission,
                "STOCK_OUT" => -amount,
                _ => continue,
            };
            let flow = convert_performance_amount(
                native_flow,
                native_currency,
                currency,
                rates_json.as_deref(),
                &format!("{native_currency} transaction for {symbol}"),
            )?;
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

    for (market, pnl) in cash_exchange_pnl(&conn, &start_str, &end_str, filter, currency)? {
        total_pnl += pnl;
        *market_pnl.entry(market.clone()).or_insert(0.0) += pnl;
        *category_pnl.entry("现金类".into()).or_insert(0.0) += pnl;
        let cash_currency = market_currency(&market);
        let symbol = format!("$CASH-{cash_currency}");
        holding_pnl.insert(
            (symbol, market),
            (format!("{cash_currency} 现金汇兑"), pnl, 0.0),
        );
    }
    let pnl_magnitude: f64 = holding_pnl.values().map(|(_, pnl, _)| pnl.abs()).sum();
    let contribution = |pnl| {
        (total_pnl.abs() > f64::EPSILON * pnl_magnitude * 16.0)
            .then(|| pnl / total_pnl.abs() * 100.0)
    };

    let make_items = |map: std::collections::HashMap<String, f64>| -> Vec<AttributionItem> {
        let mut items: Vec<AttributionItem> = map
            .into_iter()
            .map(|(name, pnl)| {
                let weight = if total_start_val != 0.0 {
                    pnl / total_start_val * 100.0
                } else {
                    0.0
                };
                AttributionItem {
                    name,
                    pnl,
                    contribution_percent: contribution(pnl),
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
            let weight = if total_start_val != 0.0 {
                sv / total_start_val * 100.0
            } else {
                0.0
            };
            AttributionItem {
                name: display_name,
                pnl,
                contribution_percent: contribution(pnl),
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

/// With end-of-day flows, FX gains on cash come from the opening native balance
/// and the change in the exchange rate. Deposits, withdrawals and security
/// settlements do not generate cash FX gains at the time they are booked.
fn cash_exchange_pnl(
    conn: &rusqlite::Connection,
    start: &str,
    end: &str,
    filter: &PerformanceFilter,
    target: &str,
) -> Result<std::collections::HashMap<String, f64>, String> {
    use super::calculation::{append_snapshot_filters, SNAPSHOT_MARKET_SQL};

    let mut sql = format!(
        "SELECT h.date, {SNAPSHOT_MARKET_SQL} AS cash_market, SUM(h.market_value), v.exchange_rates
         FROM daily_holding_snapshots h
         LEFT JOIN daily_portfolio_values v ON v.date = h.date
         WHERE h.date BETWEEN ?1 AND ?2 AND UPPER(h.symbol) LIKE '$CASH-%'",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> =
        vec![Box::new(start.to_string()), Box::new(end.to_string())];
    append_snapshot_filters(filter, &mut sql, &mut params);
    sql.push_str(" GROUP BY h.date, cash_market ORDER BY h.date");
    let param_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let mut stmt = conn.prepare(&sql).map_err(|error| error.to_string())?;
    let mut rows = stmt
        .query(param_refs.as_slice())
        .map_err(|error| error.to_string())?;
    let mut cash_by_market = std::collections::HashMap::<String, (f64, Option<String>)>::new();
    let mut pnl_by_market = std::collections::HashMap::new();
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        let date: String = row.get(0).map_err(|error| error.to_string())?;
        let market: String = row.get(1).map_err(|error| error.to_string())?;
        let source = market_currency(&market);
        if source == target {
            continue;
        }
        let balance: f64 = row.get(2).map_err(|error| error.to_string())?;
        let rates: Option<String> = row.get(3).map_err(|error| error.to_string())?;
        if let Some((previous_balance, previous_rates)) =
            cash_by_market.insert(market.clone(), (balance, rates.clone()))
        {
            let context = format!("{source} cash valuation through {date}");
            let before = convert_performance_amount(
                previous_balance,
                source,
                target,
                previous_rates.as_deref(),
                &context,
            )?;
            let after = convert_performance_amount(
                previous_balance,
                source,
                target,
                rates.as_deref(),
                &context,
            )?;
            if after != before {
                *pnl_by_market.entry(market).or_insert(0.0) += after - before;
            }
        }
    }
    Ok(pnl_by_market)
}
