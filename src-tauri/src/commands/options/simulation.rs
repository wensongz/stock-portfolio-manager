use super::contracts::get_option_contracts_inner;
use crate::db::Database;
use crate::models::option::{
    CallContractSimulation, OptionContract, PutContractSimulation, SellCallSimulation,
    SellPutSimulation,
};

#[derive(Debug, serde::Deserialize)]
pub struct StockPriceInput {
    pub symbol: String,
    pub price: f64,
}

fn load_share_lots(db: &Database) -> Result<std::collections::HashMap<String, i64>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare("SELECT stock_code, shares_per_contract FROM option_share_lots")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|e| e.to_string())?;
    let mut lots = std::collections::HashMap::new();
    for row in rows {
        let (code, shares) = row.map_err(|e| e.to_string())?;
        lots.insert(code.to_uppercase(), shares);
    }
    Ok(lots)
}

pub(super) fn simulate_sell_put_inner(
    db: &Database,
    account_id: &str,
    stock_prices: Vec<StockPriceInput>,
) -> Result<Vec<SellPutSimulation>, String> {
    let contracts = get_option_contracts_inner(db, account_id)?;

    let share_lots = load_share_lots(db)?;

    let get_shares = |underlying: &str| -> f64 {
        share_lots
            .get(&underlying.to_uppercase())
            .copied()
            .unwrap_or(100) as f64
    };

    let active_puts: Vec<&OptionContract> = contracts
        .iter()
        .filter(|c| c.status == "active" && c.option_type == "P")
        .collect();

    // Group by underlying
    let mut grouped: std::collections::HashMap<String, Vec<&OptionContract>> =
        std::collections::HashMap::new();
    for contract in &active_puts {
        grouped
            .entry(contract.underlying.clone())
            .or_default()
            .push(contract);
    }

    let price_map: std::collections::HashMap<String, f64> = stock_prices
        .into_iter()
        .map(|sp| (sp.symbol.to_uppercase(), sp.price))
        .collect();

    let mut results: Vec<SellPutSimulation> = Vec::new();

    for (underlying, puts) in &grouped {
        let stock_price = price_map.get(&underlying.to_uppercase()).copied();
        let shares_per_contract = get_shares(underlying);

        let mut sim_contracts: Vec<PutContractSimulation> = Vec::new();
        let mut total_cash = 0.0;

        for put in puts {
            let would_be_assigned = match stock_price {
                Some(price) => price < put.strike_price,
                None => false,
            };
            let cash_needed = if would_be_assigned {
                put.strike_price * put.contracts.abs() as f64 * shares_per_contract
            } else {
                0.0
            };
            total_cash += cash_needed;

            sim_contracts.push(PutContractSimulation {
                option_symbol: put.option_symbol.clone(),
                strike_price: put.strike_price,
                contracts: put.contracts,
                would_be_assigned,
                cash_needed,
            });
        }

        results.push(SellPutSimulation {
            underlying: underlying.clone(),
            contracts: sim_contracts,
            total_cash_needed: total_cash,
        });
    }

    results.sort_by(|a, b| a.underlying.cmp(&b.underlying));
    Ok(results)
}

/// Simulate sell call assignments given stock prices
pub(super) fn simulate_sell_call_inner(
    db: &Database,
    account_id: &str,
    stock_prices: Vec<StockPriceInput>,
) -> Result<Vec<SellCallSimulation>, String> {
    let contracts = get_option_contracts_inner(db, account_id)?;

    let share_lots = load_share_lots(db)?;

    let get_shares = |underlying: &str| -> i64 {
        share_lots
            .get(&underlying.to_uppercase())
            .copied()
            .unwrap_or(100)
    };

    let active_calls: Vec<&OptionContract> = contracts
        .iter()
        .filter(|c| c.status == "active" && c.option_type == "C")
        .collect();

    let mut grouped: std::collections::HashMap<String, Vec<&OptionContract>> =
        std::collections::HashMap::new();
    for contract in &active_calls {
        grouped
            .entry(contract.underlying.clone())
            .or_default()
            .push(contract);
    }

    let price_map: std::collections::HashMap<String, f64> = stock_prices
        .into_iter()
        .map(|sp| (sp.symbol.to_uppercase(), sp.price))
        .collect();

    let mut results: Vec<SellCallSimulation> = Vec::new();

    for (underlying, calls) in &grouped {
        let stock_price = price_map.get(&underlying.to_uppercase()).copied();
        let shares_per_contract = get_shares(underlying);

        let mut sim_contracts: Vec<CallContractSimulation> = Vec::new();
        let mut total_shares: i64 = 0;

        for call in calls {
            let would_be_assigned = match stock_price {
                Some(price) => price > call.strike_price,
                None => false,
            };
            let shares_needed = if would_be_assigned {
                call.contracts.abs() * shares_per_contract
            } else {
                0
            };
            total_shares += shares_needed;

            sim_contracts.push(CallContractSimulation {
                option_symbol: call.option_symbol.clone(),
                strike_price: call.strike_price,
                contracts: call.contracts,
                would_be_assigned,
                shares_needed,
            });
        }

        results.push(SellCallSimulation {
            underlying: underlying.clone(),
            contracts: sim_contracts,
            total_shares_needed: total_shares,
        });
    }

    results.sort_by(|a, b| a.underlying.cmp(&b.underlying));
    Ok(results)
}
