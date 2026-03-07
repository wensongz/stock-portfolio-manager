use crate::db::Database;
use crate::models::transaction::{CreateTransactionRequest, Transaction};
use uuid::Uuid;

/// Create a transaction and auto-update the associated holding.
/// For BUY: creates or updates holding with weighted average cost.
/// For SELL: reduces holding shares (avg_cost unchanged).
pub fn create_transaction(db: &Database, req: CreateTransactionRequest) -> Result<Transaction, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let tx_id = Uuid::new_v4().to_string();
    let now = chrono::Utc::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let commission = req.commission.unwrap_or(0.0);
    let total_amount = req.shares * req.price;
    let notes = req.notes.clone().unwrap_or_default();

    // Find or create holding
    let holding_id = find_or_create_holding(
        &conn,
        &req.account_id,
        &req.symbol,
        &req.name,
        &req.market,
        &req.currency,
        &now,
    )?;

    // Update holding based on transaction type
    update_holding_from_transaction(&conn, &holding_id, &req.tx_type, req.shares, req.price, &now)?;

    conn.execute(
        "INSERT INTO transactions (id, holding_id, account_id, symbol, name, market, type, shares, price, total_amount, commission, currency, traded_at, notes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            tx_id, holding_id, req.account_id, req.symbol, req.name, req.market,
            req.tx_type, req.shares, req.price, total_amount, commission,
            req.currency, req.traded_at, notes, now,
        ],
    )
    .map_err(|e| e.to_string())?;

    Ok(Transaction {
        id: tx_id,
        holding_id: Some(holding_id),
        account_id: req.account_id,
        symbol: req.symbol,
        name: req.name,
        market: req.market,
        tx_type: req.tx_type,
        shares: req.shares,
        price: req.price,
        total_amount,
        commission,
        currency: req.currency,
        traded_at: req.traded_at,
        notes,
        created_at: now,
    })
}

fn find_or_create_holding(
    conn: &rusqlite::Connection,
    account_id: &str,
    symbol: &str,
    name: &str,
    market: &str,
    currency: &str,
    now: &str,
) -> Result<String, String> {
    // Try to find existing holding
    let result = conn.query_row(
        "SELECT id FROM holdings WHERE account_id = ?1 AND symbol = ?2",
        rusqlite::params![account_id, symbol],
        |row| row.get::<_, String>(0),
    );

    match result {
        Ok(id) => Ok(id),
        Err(rusqlite::Error::QueryReturnedNoRows) => {
            // Create a new holding with 0 shares (will be updated by the transaction)
            let id = Uuid::new_v4().to_string();
            conn.execute(
                "INSERT INTO holdings (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL, 0, 0, ?6, ?7, ?8)",
                rusqlite::params![id, account_id, symbol, name, market, currency, now, now],
            )
            .map_err(|e| e.to_string())?;
            Ok(id)
        }
        Err(e) => Err(e.to_string()),
    }
}

fn update_holding_from_transaction(
    conn: &rusqlite::Connection,
    holding_id: &str,
    tx_type: &str,
    tx_shares: f64,
    tx_price: f64,
    now: &str,
) -> Result<(), String> {
    let (current_shares, current_avg_cost): (f64, f64) = conn
        .query_row(
            "SELECT shares, avg_cost FROM holdings WHERE id = ?1",
            rusqlite::params![holding_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|e| e.to_string())?;

    let (new_shares, new_avg_cost) = match tx_type {
        "BUY" => {
            let total_cost = current_shares * current_avg_cost + tx_shares * tx_price;
            let new_shares = current_shares + tx_shares;
            let new_avg_cost = if new_shares > 0.0 {
                total_cost / new_shares
            } else {
                0.0
            };
            (new_shares, new_avg_cost)
        }
        "SELL" => {
            let new_shares = current_shares - tx_shares;
            if new_shares < 0.0 {
                return Err("Insufficient shares for sell transaction".to_string());
            }
            // Avg cost doesn't change on sell
            (new_shares, current_avg_cost)
        }
        _ => return Err(format!("Invalid transaction type: {}", tx_type)),
    };

    conn.execute(
        "UPDATE holdings SET shares = ?1, avg_cost = ?2, updated_at = ?3 WHERE id = ?4",
        rusqlite::params![new_shares, new_avg_cost, now, holding_id],
    )
    .map_err(|e| e.to_string())?;

    Ok(())
}

pub fn list_transactions(
    db: &Database,
    account_id: Option<&str>,
    symbol: Option<&str>,
) -> Result<Vec<Transaction>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;

    let mut sql = String::from(
        "SELECT id, holding_id, account_id, symbol, name, market, type, shares, price, total_amount, commission, currency, traded_at, notes, created_at
         FROM transactions WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
    let mut param_idx = 1;

    if let Some(aid) = account_id {
        sql.push_str(&format!(" AND account_id = ?{}", param_idx));
        params.push(Box::new(aid.to_string()));
        param_idx += 1;
    }

    if let Some(sym) = symbol {
        sql.push_str(&format!(" AND symbol = ?{}", param_idx));
        params.push(Box::new(sym.to_string()));
        // param_idx is not read after this, but keeping for consistency
        let _ = param_idx + 1;
    }

    sql.push_str(" ORDER BY traded_at DESC, created_at DESC");

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let params_refs: Vec<&dyn rusqlite::types::ToSql> = params.iter().map(|p| p.as_ref()).collect();

    let transactions = stmt
        .query_map(params_refs.as_slice(), |row| {
            Ok(Transaction {
                id: row.get(0)?,
                holding_id: row.get(1)?,
                account_id: row.get(2)?,
                symbol: row.get(3)?,
                name: row.get(4)?,
                market: row.get(5)?,
                tx_type: row.get(6)?,
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
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;

    Ok(transactions)
}

pub fn delete_transaction(db: &Database, id: &str) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let changes = conn
        .execute("DELETE FROM transactions WHERE id = ?1", rusqlite::params![id])
        .map_err(|e| e.to_string())?;

    if changes == 0 {
        return Err("Transaction not found".to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::account::CreateAccountRequest;
    use crate::services::account_service;
    use crate::services::holding_service;

    fn setup_db_with_account() -> (Database, String) {
        let db = Database::new_in_memory().unwrap();
        let account = account_service::create_account(
            &db,
            CreateAccountRequest {
                name: "Test Account".to_string(),
                market: "US".to_string(),
                description: None,
            },
        )
        .unwrap();
        (db, account.id)
    }

    #[test]
    fn test_buy_transaction_creates_holding() {
        let (db, account_id) = setup_db_with_account();

        let tx = create_transaction(
            &db,
            CreateTransactionRequest {
                account_id: account_id.clone(),
                symbol: "AAPL".to_string(),
                name: "Apple Inc.".to_string(),
                market: "US".to_string(),
                tx_type: "BUY".to_string(),
                shares: 100.0,
                price: 150.0,
                commission: Some(5.0),
                currency: "USD".to_string(),
                traded_at: "2024-01-15 10:00:00".to_string(),
                notes: None,
            },
        )
        .unwrap();

        assert_eq!(tx.total_amount, 15000.0);
        assert!(tx.holding_id.is_some());

        // Check that holding was created
        let holdings = holding_service::list_holdings(&db, Some(&account_id)).unwrap();
        assert_eq!(holdings.len(), 1);
        assert_eq!(holdings[0].shares, 100.0);
        assert_eq!(holdings[0].avg_cost, 150.0);
    }

    #[test]
    fn test_buy_updates_weighted_avg_cost() {
        let (db, account_id) = setup_db_with_account();

        // First buy: 100 shares at $150
        create_transaction(
            &db,
            CreateTransactionRequest {
                account_id: account_id.clone(),
                symbol: "AAPL".to_string(),
                name: "Apple Inc.".to_string(),
                market: "US".to_string(),
                tx_type: "BUY".to_string(),
                shares: 100.0,
                price: 150.0,
                commission: None,
                currency: "USD".to_string(),
                traded_at: "2024-01-15 10:00:00".to_string(),
                notes: None,
            },
        )
        .unwrap();

        // Second buy: 50 shares at $180
        create_transaction(
            &db,
            CreateTransactionRequest {
                account_id: account_id.clone(),
                symbol: "AAPL".to_string(),
                name: "Apple Inc.".to_string(),
                market: "US".to_string(),
                tx_type: "BUY".to_string(),
                shares: 50.0,
                price: 180.0,
                commission: None,
                currency: "USD".to_string(),
                traded_at: "2024-02-01 10:00:00".to_string(),
                notes: None,
            },
        )
        .unwrap();

        let holdings = holding_service::list_holdings(&db, Some(&account_id)).unwrap();
        assert_eq!(holdings.len(), 1);
        assert_eq!(holdings[0].shares, 150.0);
        // Weighted avg: (100*150 + 50*180) / 150 = 24000/150 = 160
        assert!((holdings[0].avg_cost - 160.0).abs() < 0.01);
    }

    #[test]
    fn test_sell_reduces_shares_keeps_avg_cost() {
        let (db, account_id) = setup_db_with_account();

        // Buy 100 shares at $150
        create_transaction(
            &db,
            CreateTransactionRequest {
                account_id: account_id.clone(),
                symbol: "AAPL".to_string(),
                name: "Apple Inc.".to_string(),
                market: "US".to_string(),
                tx_type: "BUY".to_string(),
                shares: 100.0,
                price: 150.0,
                commission: None,
                currency: "USD".to_string(),
                traded_at: "2024-01-15 10:00:00".to_string(),
                notes: None,
            },
        )
        .unwrap();

        // Sell 30 shares at $170
        create_transaction(
            &db,
            CreateTransactionRequest {
                account_id: account_id.clone(),
                symbol: "AAPL".to_string(),
                name: "Apple Inc.".to_string(),
                market: "US".to_string(),
                tx_type: "SELL".to_string(),
                shares: 30.0,
                price: 170.0,
                commission: None,
                currency: "USD".to_string(),
                traded_at: "2024-02-01 10:00:00".to_string(),
                notes: None,
            },
        )
        .unwrap();

        let holdings = holding_service::list_holdings(&db, Some(&account_id)).unwrap();
        assert_eq!(holdings[0].shares, 70.0);
        assert_eq!(holdings[0].avg_cost, 150.0); // Avg cost unchanged on sell
    }

    #[test]
    fn test_sell_insufficient_shares() {
        let (db, account_id) = setup_db_with_account();

        // Buy 10 shares
        create_transaction(
            &db,
            CreateTransactionRequest {
                account_id: account_id.clone(),
                symbol: "AAPL".to_string(),
                name: "Apple Inc.".to_string(),
                market: "US".to_string(),
                tx_type: "BUY".to_string(),
                shares: 10.0,
                price: 150.0,
                commission: None,
                currency: "USD".to_string(),
                traded_at: "2024-01-15 10:00:00".to_string(),
                notes: None,
            },
        )
        .unwrap();

        // Try to sell 20 shares - should fail
        let result = create_transaction(
            &db,
            CreateTransactionRequest {
                account_id,
                symbol: "AAPL".to_string(),
                name: "Apple Inc.".to_string(),
                market: "US".to_string(),
                tx_type: "SELL".to_string(),
                shares: 20.0,
                price: 170.0,
                commission: None,
                currency: "USD".to_string(),
                traded_at: "2024-02-01 10:00:00".to_string(),
                notes: None,
            },
        );

        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Insufficient shares"));
    }

    #[test]
    fn test_list_transactions() {
        let (db, account_id) = setup_db_with_account();

        create_transaction(
            &db,
            CreateTransactionRequest {
                account_id: account_id.clone(),
                symbol: "AAPL".to_string(),
                name: "Apple Inc.".to_string(),
                market: "US".to_string(),
                tx_type: "BUY".to_string(),
                shares: 100.0,
                price: 150.0,
                commission: None,
                currency: "USD".to_string(),
                traded_at: "2024-01-15 10:00:00".to_string(),
                notes: None,
            },
        )
        .unwrap();

        let transactions = list_transactions(&db, Some(&account_id), None).unwrap();
        assert_eq!(transactions.len(), 1);

        let transactions = list_transactions(&db, None, Some("AAPL")).unwrap();
        assert_eq!(transactions.len(), 1);
    }
}
