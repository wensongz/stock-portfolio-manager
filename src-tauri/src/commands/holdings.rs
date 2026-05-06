use crate::db::Database;
use crate::models::Holding;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub fn create_holding(
    db: State<Database>,
    account_id: String,
    symbol: String,
    name: String,
    market: String,
    category_id: Option<String>,
    shares: f64,
    avg_cost: f64,
    currency: String,
) -> Result<Holding, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();

    conn.execute_batch("BEGIN IMMEDIATE").map_err(|e| e.to_string())?;

    let result = (|| -> Result<(), String> {
        conn.execute(
            "INSERT INTO holdings (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            rusqlite::params![id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, now, now],
        )
        .map_err(|e| e.to_string())?;

        // Insert an initial OPEN transaction to record the position entry
        let txn_id = uuid::Uuid::new_v4().to_string();
        let total_amount = shares * avg_cost;
        conn.execute(
            "INSERT INTO transactions (id, holding_id, account_id, symbol, name, market, transaction_type, shares, price, total_amount, commission, currency, traded_at, notes, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'OPEN', ?7, ?8, ?9, 0.0, ?10, ?11, NULL, ?12)",
            rusqlite::params![
                txn_id, id, account_id, symbol, name, market,
                shares, avg_cost, total_amount, currency, now, now
            ],
        )
        .map_err(|e| e.to_string())?;

        Ok(())
    })();

    match result {
        Ok(()) => {
            conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            return Err(e);
        }
    }

    Ok(Holding {
        id,
        account_id,
        symbol,
        name,
        market,
        category_id,
        shares,
        avg_cost,
        currency,
        created_at: now.clone(),
        updated_at: now,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn get_holdings(
    db: State<Database>,
    account_id: Option<String>,
) -> Result<Vec<Holding>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let holdings = if let Some(aid) = account_id {
        let mut stmt = conn
            .prepare(
                "SELECT id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at
                 FROM holdings WHERE account_id = ?1 ORDER BY market, symbol",
            )
            .map_err(|e| e.to_string())?;
        let result = stmt
            .query_map(rusqlite::params![aid], |row| {
                Ok(Holding {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    symbol: row.get(2)?,
                    name: row.get(3)?,
                    market: row.get(4)?,
                    category_id: row.get(5)?,
                    shares: row.get(6)?,
                    avg_cost: row.get(7)?,
                    currency: row.get(8)?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        result
    } else {
        let mut stmt = conn
            .prepare(
                "SELECT id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at
                 FROM holdings ORDER BY market, symbol",
            )
            .map_err(|e| e.to_string())?;
        let result = stmt
            .query_map([], |row| {
                Ok(Holding {
                    id: row.get(0)?,
                    account_id: row.get(1)?,
                    symbol: row.get(2)?,
                    name: row.get(3)?,
                    market: row.get(4)?,
                    category_id: row.get(5)?,
                    shares: row.get(6)?,
                    avg_cost: row.get(7)?,
                    currency: row.get(8)?,
                    created_at: row.get(9)?,
                    updated_at: row.get(10)?,
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        result
    };
    Ok(holdings)
}

#[tauri::command(rename_all = "camelCase")]
pub fn update_holding(
    db: State<Database>,
    id: String,
    account_id: String,
    symbol: String,
    name: String,
    market: String,
    category_id: Option<String>,
    shares: f64,
    avg_cost: f64,
    currency: String,
) -> Result<Holding, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let now = chrono::Utc::now().to_rfc3339();
    let rows_affected = conn
        .execute(
            "UPDATE holdings SET account_id = ?2, symbol = ?3, name = ?4, market = ?5,
             category_id = ?6, shares = ?7, avg_cost = ?8, currency = ?9, updated_at = ?10
             WHERE id = ?1",
            rusqlite::params![id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, now],
        )
        .map_err(|e| e.to_string())?;
    if rows_affected == 0 {
        return Err(format!("Holding with id {} not found", id));
    }
    let created_at: String = conn
        .query_row(
            "SELECT created_at FROM holdings WHERE id = ?1",
            rusqlite::params![id],
            |row| row.get(0),
        )
        .map_err(|e| e.to_string())?;
    Ok(Holding {
        id,
        account_id,
        symbol,
        name,
        market,
        category_id,
        shares,
        avg_cost,
        currency,
        created_at,
        updated_at: now,
    })
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_holding(db: State<Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM holdings WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;
    Ok(())
}
