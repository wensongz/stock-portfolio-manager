use crate::db::Database;
use crate::models::Transaction;
use tauri::State;

#[tauri::command]
pub fn create_transaction(
    db: State<Database>,
    account_id: String,
    symbol: String,
    name: String,
    market: String,
    transaction_type: String,
    shares: f64,
    price: f64,
    total_amount: f64,
    commission: f64,
    currency: String,
    traded_at: String,
    notes: Option<String>,
) -> Result<Transaction, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    // Find existing holding for this symbol/account
    let holding_id: Option<String> = conn
        .query_row(
            "SELECT id FROM holdings WHERE account_id = ?1 AND symbol = ?2",
            rusqlite::params![account_id, symbol],
            |row| row.get(0),
        )
        .ok();

    // Update holding shares and avg_cost based on transaction type
    if let Some(ref hid) = holding_id {
        let (current_shares, current_avg_cost): (f64, f64) = conn
            .query_row(
                "SELECT shares, avg_cost FROM holdings WHERE id = ?1",
                rusqlite::params![hid],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .map_err(|e| e.to_string())?;

        let (new_shares, new_avg_cost) = if transaction_type == "BUY" {
            let total_shares = current_shares + shares;
            let new_avg = if total_shares > 0.0 {
                (current_shares * current_avg_cost + shares * price) / total_shares
            } else {
                price
            };
            (total_shares, new_avg)
        } else {
            // SELL: shares decrease, avg_cost unchanged
            (current_shares - shares, current_avg_cost)
        };

        let updated_at = chrono::Utc::now().to_rfc3339();
        conn.execute(
            "UPDATE holdings SET shares = ?2, avg_cost = ?3, updated_at = ?4 WHERE id = ?1",
            rusqlite::params![hid, new_shares, new_avg_cost, updated_at],
        )
        .map_err(|e| e.to_string())?;
    }

    conn.execute(
        "INSERT INTO transactions (id, holding_id, account_id, symbol, name, market, transaction_type, shares, price, total_amount, commission, currency, traded_at, notes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            id, holding_id, account_id, symbol, name, market,
            transaction_type, shares, price, total_amount, commission,
            currency, traded_at, notes, now
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(Transaction {
        id,
        holding_id,
        account_id,
        symbol,
        name,
        market,
        transaction_type,
        shares,
        price,
        total_amount,
        commission,
        currency,
        traded_at,
        notes,
        created_at: now,
    })
}

#[tauri::command]
pub fn get_transactions(
    db: State<Database>,
    account_id: Option<String>,
    symbol: Option<String>,
) -> Result<Vec<Transaction>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    let base_query = "SELECT id, holding_id, account_id, symbol, name, market, transaction_type,
                      shares, price, total_amount, commission, currency, traded_at, notes, created_at
                      FROM transactions";

    let transactions = match (account_id, symbol) {
        (Some(aid), Some(sym)) => {
            let query = format!("{} WHERE account_id = ?1 AND symbol = ?2 ORDER BY traded_at DESC", base_query);
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
            let result = stmt.query_map(rusqlite::params![aid, sym], map_transaction)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            result
        }
        (Some(aid), None) => {
            let query = format!("{} WHERE account_id = ?1 ORDER BY traded_at DESC", base_query);
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
            let result = stmt.query_map(rusqlite::params![aid], map_transaction)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            result
        }
        (None, Some(sym)) => {
            let query = format!("{} WHERE symbol = ?1 ORDER BY traded_at DESC", base_query);
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
            let result = stmt.query_map(rusqlite::params![sym], map_transaction)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            result
        }
        (None, None) => {
            let query = format!("{} ORDER BY traded_at DESC", base_query);
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
            let result = stmt.query_map([], map_transaction)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            result
        }
    };

    Ok(transactions)
}

fn map_transaction(row: &rusqlite::Row<'_>) -> rusqlite::Result<Transaction> {
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
}

#[tauri::command]
pub fn delete_transaction(db: State<Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute(
        "DELETE FROM transactions WHERE id = ?1",
        rusqlite::params![id],
    )
    .map_err(|e| e.to_string())?;
    Ok(())
}
