use super::*;

/// Compare two quarterly snapshots.
pub fn compare_quarters(
    db: &Database,
    quarter1: &str,
    quarter2: &str,
) -> Result<QuarterComparison, String> {
    let (snap1, h1) = load_snapshot_for_quarter(db, quarter1)?;
    let (snap2, h2) = load_snapshot_for_quarter(db, quarter2)?;

    // Overview
    let value_change = snap2.total_value - snap1.total_value;
    let value_change_percent = if snap1.total_value != 0.0 {
        value_change / snap1.total_value * 100.0
    } else {
        0.0
    };
    let overview = ComparisonOverview {
        q1_total_value: snap1.total_value,
        q2_total_value: snap2.total_value,
        value_change,
        value_change_percent,
        q1_total_cost: snap1.total_cost,
        q2_total_cost: snap2.total_cost,
        q1_pnl: snap1.total_pnl,
        q2_pnl: snap2.total_pnl,
        q1_holding_count: h1
            .iter()
            .map(|h| h.symbol.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len(),
        q2_holding_count: h2
            .iter()
            .map(|h| h.symbol.as_str())
            .collect::<std::collections::HashSet<_>>()
            .len(),
    };

    // By market
    let by_market = compute_market_comparison(&h1, &h2);

    // By category
    let by_category = compute_category_comparison(&h1, &h2);

    // Holding changes
    let holding_changes = compute_holding_changes(&h1, &h2);

    Ok(QuarterComparison {
        quarter1: quarter1.to_string(),
        quarter2: quarter2.to_string(),
        overview,
        by_market,
        by_category,
        holding_changes,
    })
}

fn load_snapshot_for_quarter(
    db: &Database,
    quarter: &str,
) -> Result<(QuarterlySnapshot, Vec<QuarterlyHoldingSnapshot>), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    let snapshot_id: String = conn
        .query_row(
            "SELECT id FROM quarterly_snapshots WHERE quarter = ?1",
            rusqlite::params![quarter],
            |row| row.get(0),
        )
        .map_err(|_| format!("No snapshot found for quarter '{}'", quarter))?;

    let snapshot = conn
        .query_row(
            "SELECT id, quarter, snapshot_date, total_value, total_cost, total_pnl,
                    us_value, us_cost, cn_value, cn_cost, hk_value, hk_cost,
                    exchange_rates, overall_notes, created_at
             FROM quarterly_snapshots WHERE id = ?1",
            rusqlite::params![snapshot_id],
            |row| {
                Ok(QuarterlySnapshot {
                    id: row.get(0)?,
                    quarter: row.get(1)?,
                    snapshot_date: row.get(2)?,
                    total_value: row.get(3)?,
                    total_cost: row.get(4)?,
                    total_pnl: row.get(5)?,
                    us_value: row.get(6)?,
                    us_cost: row.get(7)?,
                    cn_value: row.get(8)?,
                    cn_cost: row.get(9)?,
                    hk_value: row.get(10)?,
                    hk_cost: row.get(11)?,
                    exchange_rates: row.get(12)?,
                    overall_notes: row.get(13)?,
                    created_at: row.get(14)?,
                    holding_count: 0,
                })
            },
        )
        .map_err(|e| e.to_string())?;

    let mut stmt = conn
        .prepare(
            "SELECT id, quarterly_snapshot_id, account_id, account_name, symbol, name, market,
                    category_name, category_color, shares, avg_cost, close_price,
                    market_value, cost_value, pnl, pnl_percent, weight, notes
             FROM quarterly_holding_snapshots
             WHERE quarterly_snapshot_id = ?1",
        )
        .map_err(|e| e.to_string())?;

    let holdings = stmt
        .query_map(rusqlite::params![snapshot_id], |row| {
            Ok(QuarterlyHoldingSnapshot {
                id: row.get(0)?,
                quarterly_snapshot_id: row.get(1)?,
                account_id: row.get(2)?,
                account_name: row.get(3)?,
                symbol: row.get(4)?,
                name: row.get(5)?,
                market: row.get(6)?,
                category_name: row.get(7)?,
                category_color: row.get(8)?,
                shares: row.get(9)?,
                avg_cost: row.get(10)?,
                close_price: row.get(11)?,
                market_value: row.get(12)?,
                cost_value: row.get(13)?,
                pnl: row.get(14)?,
                pnl_percent: row.get(15)?,
                weight: row.get(16)?,
                notes: row.get(17)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok((snapshot, holdings))
}

/// Load only the holdings for a given quarter. Used for computing holding changes.
/// Read holdings from a quarterly snapshot (fast, no recalculation).
pub(super) fn load_holdings_for_quarter_from_snapshot(
    db: &Database,
    quarter: &str,
) -> Result<Vec<QuarterlyHoldingSnapshot>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    let snapshot_id: String = conn
        .query_row(
            "SELECT id FROM quarterly_snapshots WHERE quarter = ?1",
            rusqlite::params![quarter],
            |row| row.get(0),
        )
        .map_err(|_| format!("No snapshot found for quarter '{}'", quarter))?;

    let mut stmt = conn
        .prepare(
            "SELECT id, quarterly_snapshot_id, account_id, account_name, symbol, name, market,
                    category_name, category_color, shares, avg_cost, close_price,
                    market_value, cost_value, pnl, pnl_percent, weight, notes
             FROM quarterly_holding_snapshots
             WHERE quarterly_snapshot_id = ?1",
        )
        .map_err(|e| e.to_string())?;

    let holdings = stmt
        .query_map(rusqlite::params![snapshot_id], |row| {
            Ok(QuarterlyHoldingSnapshot {
                id: row.get(0)?,
                quarterly_snapshot_id: row.get(1)?,
                account_id: row.get(2)?,
                account_name: row.get(3)?,
                symbol: row.get(4)?,
                name: row.get(5)?,
                market: row.get(6)?,
                category_name: row.get(7)?,
                category_color: row.get(8)?,
                shares: row.get(9)?,
                avg_cost: row.get(10)?,
                close_price: row.get(11)?,
                market_value: row.get(12)?,
                cost_value: row.get(13)?,
                pnl: row.get(14)?,
                pnl_percent: row.get(15)?,
                weight: row.get(16)?,
                notes: row.get(17)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(holdings)
}

fn compute_market_comparison(
    h1: &[QuarterlyHoldingSnapshot],
    h2: &[QuarterlyHoldingSnapshot],
) -> Vec<MarketComparison> {
    let markets = ["US", "CN", "HK"];
    markets
        .iter()
        .map(|m| {
            let q1_value: f64 = h1
                .iter()
                .filter(|h| h.market == *m)
                .map(|h| h.market_value)
                .sum();
            let q1_cost: f64 = h1
                .iter()
                .filter(|h| h.market == *m)
                .map(|h| h.cost_value)
                .sum();
            let q2_value: f64 = h2
                .iter()
                .filter(|h| h.market == *m)
                .map(|h| h.market_value)
                .sum();
            let q2_cost: f64 = h2
                .iter()
                .filter(|h| h.market == *m)
                .map(|h| h.cost_value)
                .sum();
            let value_change = q2_value - q1_value;
            let value_change_percent = if q1_value != 0.0 {
                value_change / q1_value * 100.0
            } else {
                0.0
            };
            MarketComparison {
                market: m.to_string(),
                q1_value,
                q2_value,
                value_change,
                value_change_percent,
                q1_cost,
                q2_cost,
                q1_pnl: q1_value - q1_cost,
                q2_pnl: q2_value - q2_cost,
            }
        })
        .collect()
}

fn compute_category_comparison(
    h1: &[QuarterlyHoldingSnapshot],
    h2: &[QuarterlyHoldingSnapshot],
) -> Vec<CategoryComparison> {
    let mut all_categories: Vec<(String, String)> = Vec::new();
    for h in h1.iter().chain(h2.iter()) {
        if !all_categories.iter().any(|(n, _)| n == &h.category_name) {
            all_categories.push((h.category_name.clone(), h.category_color.clone()));
        }
    }

    all_categories
        .into_iter()
        .map(|(cat_name, cat_color)| {
            let q1_value: f64 = h1
                .iter()
                .filter(|h| h.category_name == cat_name)
                .map(|h| h.market_value)
                .sum();
            let q1_cost: f64 = h1
                .iter()
                .filter(|h| h.category_name == cat_name)
                .map(|h| h.cost_value)
                .sum();
            let q2_value: f64 = h2
                .iter()
                .filter(|h| h.category_name == cat_name)
                .map(|h| h.market_value)
                .sum();
            let q2_cost: f64 = h2
                .iter()
                .filter(|h| h.category_name == cat_name)
                .map(|h| h.cost_value)
                .sum();
            let value_change = q2_value - q1_value;
            let value_change_percent = if q1_value != 0.0 {
                value_change / q1_value * 100.0
            } else {
                0.0
            };
            CategoryComparison {
                category_name: cat_name,
                category_color: cat_color,
                q1_value,
                q2_value,
                value_change,
                value_change_percent,
                q1_cost,
                q2_cost,
                q1_pnl: q1_value - q1_cost,
                q2_pnl: q2_value - q2_cost,
            }
        })
        .collect()
}

fn compute_holding_changes(
    h1: &[QuarterlyHoldingSnapshot],
    h2: &[QuarterlyHoldingSnapshot],
) -> HoldingChanges {
    // Aggregate by symbol: sum shares and values across accounts.
    // Skip cash pseudo-symbols ($CASH-USD etc.).
    struct Agg {
        symbol: String, // original-case symbol for display
        name: String,
        market: String,
        category_name: String,
        shares: f64,
        market_value: f64,
        cost_value: f64,
    }
    fn aggregate(holdings: &[QuarterlyHoldingSnapshot]) -> HashMap<String, Agg> {
        let mut map: HashMap<String, Agg> = HashMap::new();
        for h in holdings {
            if h.symbol.starts_with("$CASH-") {
                continue;
            }
            let key = h.symbol.to_uppercase();
            map.entry(key)
                .and_modify(|a| {
                    a.shares += h.shares;
                    a.market_value += h.market_value;
                    a.cost_value += h.cost_value;
                })
                .or_insert_with(|| Agg {
                    symbol: h.symbol.clone(),
                    name: h.name.clone(),
                    market: h.market.clone(),
                    category_name: h.category_name.clone(),
                    shares: h.shares,
                    market_value: h.market_value,
                    cost_value: h.cost_value,
                });
        }
        map
    }

    let map1 = aggregate(h1);
    let map2 = aggregate(h2);

    let mut new_holdings = Vec::new();
    let mut closed_holdings = Vec::new();
    let mut increased = Vec::new();
    let mut decreased = Vec::new();
    let mut unchanged = Vec::new();

    // Holdings in q2
    for (sym, agg2) in &map2 {
        if let Some(agg1) = map1.get(sym) {
            let shares_change = agg2.shares - agg1.shares;
            let value_change = agg2.market_value - agg1.market_value;
            let item = HoldingChangeItem {
                symbol: agg2.symbol.clone(),
                name: agg2.name.clone(),
                market: agg2.market.clone(),
                category_name: agg2.category_name.clone(),
                q1_shares: Some(agg1.shares),
                q2_shares: Some(agg2.shares),
                q1_value: Some(agg1.market_value),
                q2_value: Some(agg2.market_value),
                shares_change,
                value_change,
            };
            if shares_change > 1e-9 {
                increased.push(item);
            } else if shares_change < -1e-9 {
                decreased.push(item);
            } else {
                unchanged.push(item);
            }
        } else {
            new_holdings.push(HoldingChangeItem {
                symbol: agg2.symbol.clone(),
                name: agg2.name.clone(),
                market: agg2.market.clone(),
                category_name: agg2.category_name.clone(),
                q1_shares: None,
                q2_shares: Some(agg2.shares),
                q1_value: None,
                q2_value: Some(agg2.market_value),
                shares_change: agg2.shares,
                value_change: agg2.market_value,
            });
        }
    }

    // Holdings in q1 but not q2 (closed)
    for (sym, agg1) in &map1 {
        if !map2.contains_key(sym) {
            closed_holdings.push(HoldingChangeItem {
                symbol: agg1.symbol.clone(),
                name: agg1.name.clone(),
                market: agg1.market.clone(),
                category_name: agg1.category_name.clone(),
                q1_shares: Some(agg1.shares),
                q2_shares: None,
                q1_value: Some(agg1.market_value),
                q2_value: None,
                shares_change: -agg1.shares,
                value_change: -agg1.market_value,
            });
        }
    }

    // Sort each list: CN → HK → US, then symbol ascending
    fn market_order(m: &str) -> u8 {
        match m {
            "CN" => 1,
            "HK" => 2,
            _ => 3,
        }
    }
    let sort_list = |list: &mut Vec<HoldingChangeItem>| {
        list.sort_by(|a, b| {
            market_order(&a.market)
                .cmp(&market_order(&b.market))
                .then_with(|| a.symbol.cmp(&b.symbol))
        });
    };
    sort_list(&mut new_holdings);
    sort_list(&mut closed_holdings);
    sort_list(&mut increased);
    sort_list(&mut decreased);
    sort_list(&mut unchanged);

    HoldingChanges {
        new_holdings,
        closed_holdings,
        increased,
        decreased,
        unchanged,
    }
}
