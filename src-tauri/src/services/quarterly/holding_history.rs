use super::*;
use crate::services::cash_reconciliation_service::load_cash_ledger;
use rusqlite::Connection;

/// Read the selected snapshot row's actual operations within its full quarter.
/// The snapshot supplies identity; a current holding is not required to exist.
pub fn get_quarterly_holding_history(
    db: &Database,
    snapshot_id: &str,
    holding_snapshot_id: &str,
) -> Result<QuarterlyHoldingHistory, String> {
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .map_err(|error| error.to_string())?;
    let (account_id, symbol, market, explicit_currency, quarter): (
        String,
        String,
        String,
        String,
        String,
    ) = tx
        .query_row(
            "SELECT qhs.account_id,qhs.symbol,qhs.market,qhs.currency,qs.quarter
             FROM quarterly_holding_snapshots qhs
             JOIN quarterly_snapshots qs ON qs.id=qhs.quarterly_snapshot_id
             WHERE qhs.id=?1 AND qhs.quarterly_snapshot_id=?2",
            rusqlite::params![holding_snapshot_id, snapshot_id],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "该持仓不属于所选季度快照，或快照已更新，请刷新后重试".to_string())?;
    let currency = currency::currency_for_holding(&symbol, &market, &explicit_currency)?;
    let (year, quarter_number) = parse_quarter(&quarter)?;
    NaiveDate::from_ymd_opt(year, 1, 1).ok_or_else(|| "季度年份无效".to_string())?;
    let start = quarter_start_date(year, quarter_number);
    let end = quarter_end_date(year, quarter_number);
    let start_date = start.format("%Y-%m-%d").to_string();
    let end_date = end.format("%Y-%m-%d").to_string();
    let normalized_symbol = symbol.to_uppercase();
    let rows = if normalized_symbol.starts_with("$CASH-") {
        if normalized_symbol != format!("$CASH-{currency}") {
            return Err("现金快照代码与币种不一致".into());
        }
        // Replay before filtering so the first in-quarter balance includes
        // earlier money flows and OPEN resets, while later quarters stay hidden.
        load_cash_ledger(&tx, &account_id, &currency)?
            .rows
            .into_iter()
            .filter(|row| row.trade_date >= start && row.trade_date <= end)
            .rev()
            .map(|row| QuarterlyHoldingHistoryRow {
                transaction: row.transaction,
                cash_delta: Some(row.cash_delta),
                running_balance: Some(row.running_balance),
            })
            .collect()
    } else {
        load_stock_history(
            &tx,
            &account_id,
            &symbol,
            &market,
            &currency,
            &start_date,
            &end_date,
        )?
    };
    let history = QuarterlyHoldingHistory {
        holding_snapshot_id: holding_snapshot_id.to_string(),
        snapshot_id: snapshot_id.to_string(),
        account_id,
        symbol,
        currency,
        quarter,
        start_date,
        end_date,
        rows,
    };
    tx.commit().map_err(|error| error.to_string())?;
    Ok(history)
}

fn load_stock_history(
    conn: &Connection,
    account_id: &str,
    symbol: &str,
    market: &str,
    currency: &str,
    start_date: &str,
    end_date: &str,
) -> Result<Vec<QuarterlyHoldingHistoryRow>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id,holding_id,account_id,symbol,name,market,transaction_type,shares,price,
                total_amount,commission,currency,traded_at,notes,created_at
         FROM transactions
         WHERE account_id=?1 AND UPPER(symbol)=UPPER(?2) AND market=?3 AND currency=?4
           AND DATE(traded_at)>=?5 AND DATE(traded_at)<=?6
         ORDER BY JULIANDAY(traded_at) DESC,JULIANDAY(created_at) DESC,id DESC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            rusqlite::params![account_id, symbol, market, currency, start_date, end_date],
            |row| {
                Ok(QuarterlyHoldingHistoryRow {
                    transaction: Transaction {
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
                    },
                    cash_delta: None,
                    running_balance: None,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

#[cfg(test)]
#[path = "holding_history_tests.rs"]
mod tests;
