use super::csv::parse_expiry_to_sortable;
use crate::db::Database;
use crate::models::option::{OptionContract, OptionRecord};
use crate::services::option_matching::{match_options_fifo, MatchRecord, SplitRecord};

pub(super) fn load_matching_inputs(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<(Vec<MatchRecord>, Vec<SplitRecord>), String> {
    let records = {
        let mut statement = conn
            .prepare(
                "SELECT id, option_symbol, underlying, expiry_date, strike_price,
                        option_type, action, code, quantity, traded_at
                 FROM option_records WHERE account_id = ?1",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([account_id], |row| {
                Ok(MatchRecord {
                    id: row.get(0)?,
                    option_symbol: row.get(1)?,
                    underlying: row.get(2)?,
                    expiry_date: row.get(3)?,
                    strike_price: row.get(4)?,
                    option_type: row.get(5)?,
                    action: row.get(6)?,
                    code: row.get(7)?,
                    quantity: row.get(8)?,
                    traded_at: row.get(9)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        rows
    };
    let splits = load_split_records(conn)?;
    Ok((records, splits))
}

fn load_split_records(conn: &rusqlite::Connection) -> Result<Vec<SplitRecord>, String> {
    let mut statement = conn
        .prepare("SELECT stock_code, split_date, ratio_from, ratio_to FROM stock_splits")
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok(SplitRecord {
                stock_code: row.get(0)?,
                split_date: row.get(1)?,
                ratio_from: row.get(2)?,
                ratio_to: row.get(3)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

/// Internal helper without the tauri::State wrapper (testable directly).
/// See `import_options_csv` for the CSV format.
#[cfg(test)]
pub(super) fn recompute_option_statuses(db: &Database, account_id: &str) -> Result<(), String> {
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    recompute_option_statuses_in(&transaction, account_id)?;
    transaction.commit().map_err(|e| e.to_string())
}

pub(super) fn recompute_option_statuses_in(
    conn: &rusqlite::Connection,
    account_id: &str,
) -> Result<(), String> {
    let (records, splits) = load_matching_inputs(conn, account_id)?;
    let result = match_options_fifo(&records, &splits);
    let close_codes: std::collections::HashMap<_, _> = records
        .iter()
        .map(|record| (record.id.as_str(), record.code.as_str()))
        .collect();

    conn.execute(
        "UPDATE option_records SET contract_status = 'active' WHERE account_id = ?1",
        rusqlite::params![account_id],
    )
    .map_err(|e| e.to_string())?;

    for open in records
        .iter()
        .filter(|record| record.action == "SELL" && record.code.starts_with('O'))
    {
        if result.remaining_open.get(&open.id).copied().unwrap_or(0) != 0 {
            continue;
        }
        let Some(completing_close) = result
            .allocations
            .iter()
            .rev()
            .find(|allocation| allocation.open_id == open.id)
        else {
            continue;
        };
        let status = match close_codes.get(completing_close.close_id.as_str()).copied() {
            Some("A;C") => "assigned",
            Some("C") | Some("C;P") => "closed",
            _ => "expired",
        };
        conn.execute(
            "UPDATE option_records SET contract_status = ?1 WHERE id = ?2",
            rusqlite::params![status, open.id],
        )
        .map_err(|e| e.to_string())?;
    }

    Ok(())
}

/// Get all option contracts for an account, paired by option_symbol
#[tauri::command(rename_all = "camelCase")]
pub fn get_option_contracts_inner(
    db: &Database,
    account_id: &str,
) -> Result<Vec<OptionContract>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT id, account_id, option_symbol, underlying, expiry_date, strike_price,
                    option_type, action, code, quantity, price, amount, commission, fee,
                    traded_at, settled_at, created_at, contract_status
             FROM option_records WHERE account_id = ?1
             ORDER BY option_symbol, traded_at",
        )
        .map_err(|e| e.to_string())?;

    let records: Vec<OptionRecord> = stmt
        .query_map(rusqlite::params![account_id], |row| {
            Ok(OptionRecord {
                id: row.get(0)?,
                account_id: row.get(1)?,
                option_symbol: row.get(2)?,
                underlying: row.get(3)?,
                expiry_date: row.get(4)?,
                strike_price: row.get(5)?,
                option_type: row.get(6)?,
                action: row.get(7)?,
                code: row.get(8)?,
                quantity: row.get(9)?,
                price: row.get(10)?,
                amount: row.get(11)?,
                commission: row.get(12)?,
                fee: row.get(13)?,
                traded_at: row.get(14)?,
                settled_at: row.get(15)?,
                created_at: row.get(16)?,
                contract_status: row.get(17)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let match_records: Vec<_> = records
        .iter()
        .map(|record| MatchRecord {
            id: record.id.clone(),
            option_symbol: record.option_symbol.clone(),
            underlying: record.underlying.clone(),
            expiry_date: record.expiry_date.clone(),
            strike_price: record.strike_price,
            option_type: record.option_type.clone(),
            action: record.action.clone(),
            code: record.code.clone(),
            quantity: record.quantity,
            traded_at: record.traded_at.clone(),
        })
        .collect();
    let result = match_options_fifo(&match_records, &load_split_records(&conn)?);
    let records_by_id: std::collections::HashMap<_, _> = records
        .iter()
        .map(|record| (record.id.as_str(), record))
        .collect();
    let mut contracts: Vec<OptionContract> = records
        .iter()
        .filter(|record| record.action == "SELL" && record.code.starts_with('O'))
        .map(|open| {
            let is_complete = result.remaining_open.get(&open.id).copied().unwrap_or(0) == 0;
            let completing_close = is_complete
                .then(|| {
                    result
                        .allocations
                        .iter()
                        .rev()
                        .find(|allocation| allocation.open_id == open.id)
                        .and_then(|allocation| records_by_id.get(allocation.close_id.as_str()))
                })
                .flatten();
            let status = completing_close
                .map(|close| match close.code.as_str() {
                    "A;C" => "assigned",
                    "C" | "C;P" => "closed",
                    _ => "expired",
                })
                .unwrap_or("active")
                .to_string();
            OptionContract {
                id: open.id.clone(),
                option_symbol: open.option_symbol.clone(),
                underlying: open.underlying.clone(),
                expiry_date: open.expiry_date.clone(),
                strike_price: open.strike_price,
                option_type: open.option_type.clone(),
                contracts: open.quantity,
                open_price: open.price,
                open_amount: open.amount,
                commission: open.commission,
                traded_at: open.traded_at.clone(),
                close_price: completing_close.map(|record| record.price),
                close_code: completing_close.map(|record| record.code.clone()),
                status,
                account_id: open.account_id.clone(),
            }
        })
        .collect();

    contracts.sort_by(|a, b| {
        a.underlying
            .cmp(&b.underlying)
            .then_with(|| {
                parse_expiry_to_sortable(&a.expiry_date)
                    .cmp(&parse_expiry_to_sortable(&b.expiry_date))
            })
            .then_with(|| {
                a.strike_price
                    .partial_cmp(&b.strike_price)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    Ok(contracts)
}
