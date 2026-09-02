use crate::db::Database;
use crate::models::Transaction;
#[cfg(test)]
pub(crate) use crate::services::portfolio_mutation::{adjust_cash_holding, cash_delta};
use crate::services::portfolio_mutation::{
    create_transaction_in, delete_transaction_in, update_transaction_in, CreateTransactionInput,
};
use crate::services::position_replay::rebuild_all_position_groups;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
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
    let input = CreateTransactionInput {
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
    };
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let created = create_transaction_in(&transaction, &input)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(created)
}

#[tauri::command(rename_all = "camelCase")]
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
            let query = format!(
                "{} WHERE account_id = ?1 AND UPPER(symbol) = UPPER(?2) ORDER BY traded_at DESC",
                base_query
            );
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
            let result = stmt
                .query_map(rusqlite::params![aid, sym], map_transaction)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            result
        }
        (Some(aid), None) => {
            let query = format!(
                "{} WHERE account_id = ?1 ORDER BY traded_at DESC",
                base_query
            );
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
            let result = stmt
                .query_map(rusqlite::params![aid], map_transaction)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            result
        }
        (None, Some(sym)) => {
            let query = format!(
                "{} WHERE UPPER(symbol) = UPPER(?1) ORDER BY traded_at DESC",
                base_query
            );
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
            let result = stmt
                .query_map(rusqlite::params![sym], map_transaction)
                .map_err(|e| e.to_string())?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|e| e.to_string())?;
            result
        }
        (None, None) => {
            let query = format!("{} ORDER BY traded_at DESC", base_query);
            let mut stmt = conn.prepare(&query).map_err(|e| e.to_string())?;
            let result = stmt
                .query_map([], map_transaction)
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

/// Flexible transaction query for the AI tools layer. Filters are all
/// optional and combined with AND; results are newest-first, capped at `limit`
/// (default 50, max 200) so a tool call can't dump the entire history into the
/// model's context.
///
/// `tx_type` is matched case-insensitively against `transaction_type`
/// (BUY/SELL/OPEN/PAY). `days` restricts to the last N days (traded_at >= now
/// minus days). This is a plain `&Database` function (no Tauri State) so it can
/// be called from `ai_tools::execute_tool` directly.
pub fn query_transactions_inner(
    db: &Database,
    account_id: Option<&str>,
    symbol: Option<&str>,
    tx_type: Option<&str>,
    days: Option<i64>,
    limit: Option<usize>,
) -> Result<Vec<Transaction>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut sql = String::from(
        "SELECT id, holding_id, account_id, symbol, name, market, transaction_type,
                shares, price, total_amount, commission, currency, traded_at, notes, created_at
         FROM transactions WHERE 1=1",
    );
    let mut params: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();
    if let Some(aid) = account_id {
        sql.push_str(" AND account_id = ?");
        params.push(Box::new(aid.to_string()));
    }
    if let Some(sym) = symbol {
        sql.push_str(" AND UPPER(symbol) = UPPER(?)");
        params.push(Box::new(sym.to_string()));
    }
    if let Some(t) = tx_type {
        sql.push_str(" AND UPPER(transaction_type) = UPPER(?)");
        params.push(Box::new(t.to_string()));
    }
    if let Some(d) = days {
        sql.push_str(" AND traded_at >= ?");
        let cutoff = chrono::Utc::now() - chrono::Duration::days(d);
        params.push(Box::new(cutoff.to_rfc3339()));
    }
    sql.push_str(" ORDER BY traded_at DESC LIMIT ?");
    let cap = limit.unwrap_or(50).min(200);
    params.push(Box::new(cap as i64));

    let mut stmt = conn.prepare(&sql).map_err(|e| e.to_string())?;
    let param_refs: Vec<&dyn rusqlite::ToSql> = params.iter().map(|p| p.as_ref()).collect();
    let rows = stmt
        .query_map(param_refs.as_slice(), map_transaction)
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub fn update_transaction(
    db: State<Database>,
    id: String,
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
    let input = CreateTransactionInput {
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
    };
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let updated = update_transaction_in(&transaction, &id, &input)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(updated)
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_transaction(db: State<Database>, id: String) -> Result<(), String> {
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    delete_transaction_in(&transaction, &id)?;
    transaction.commit().map_err(|error| error.to_string())
}

/// Rebuild every non-cash holding from chronological transaction history.
/// The caller-facing command owns one transaction so no partial rebuild can persist.
#[tauri::command(rename_all = "camelCase")]
pub fn recalculate_holdings_cost(db: State<Database>) -> Result<(), String> {
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    rebuild_all_position_groups(&transaction)?;
    transaction.commit().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;
    use crate::services::portfolio_mutation::{
        validate_cash_withdrawal, validate_transaction_shares,
    };

    /// Build an in-memory DB with one US account.
    fn db_with_account() -> (Database, String) {
        let db = Database::new(":memory:").expect("failed to create in-memory database");
        let account_id = "acct-test".to_string();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
                 VALUES (?1, ?2, 'US', ?3, ?3)",
                rusqlite::params![account_id, "Test Account", chrono::Utc::now().to_rfc3339()],
            )
            .expect("failed to insert account");
        }
        (db, account_id)
    }

    fn get_cash_shares(conn: &rusqlite::Connection, account_id: &str, currency: &str) -> f64 {
        conn.query_row(
            "SELECT shares FROM holdings WHERE account_id = ?1 AND symbol = ?2",
            rusqlite::params![account_id, format!("$CASH-{}", currency)],
            |row| row.get(0),
        )
        .unwrap_or(0.0)
    }

    #[test]
    fn test_cash_deposit_increases_balance() {
        let (db, account_id) = db_with_account();
        let conn = db.conn.lock().unwrap();
        // Deposit 1000 USD: BUY on $CASH-USD
        let delta = cash_delta("BUY", "$CASH-USD", 1000.0, 0.0);
        adjust_cash_holding(&conn, &account_id, "USD", "US", delta).unwrap();
        assert_eq!(get_cash_shares(&conn, &account_id, "USD"), 1000.0);
    }

    #[test]
    fn test_cash_withdraw_decreases_balance() {
        let (db, account_id) = db_with_account();
        let conn = db.conn.lock().unwrap();
        // Deposit 1000, then withdraw 400: SELL on $CASH-USD
        adjust_cash_holding(
            &conn,
            &account_id,
            "USD",
            "US",
            cash_delta("BUY", "$CASH-USD", 1000.0, 0.0),
        )
        .unwrap();
        adjust_cash_holding(
            &conn,
            &account_id,
            "USD",
            "US",
            cash_delta("SELL", "$CASH-USD", 400.0, 0.0),
        )
        .unwrap();
        assert_eq!(get_cash_shares(&conn, &account_id, "USD"), 600.0);
    }

    #[test]
    fn test_cash_delete_reversal_restores_balance() {
        let (db, account_id) = db_with_account();
        let conn = db.conn.lock().unwrap();
        // Deposit 1000, withdraw 400 (balance 600)
        adjust_cash_holding(
            &conn,
            &account_id,
            "USD",
            "US",
            cash_delta("BUY", "$CASH-USD", 1000.0, 0.0),
        )
        .unwrap();
        adjust_cash_holding(
            &conn,
            &account_id,
            "USD",
            "US",
            cash_delta("SELL", "$CASH-USD", 400.0, 0.0),
        )
        .unwrap();
        assert_eq!(get_cash_shares(&conn, &account_id, "USD"), 600.0);
        // Reverse the withdrawal (what delete_transaction does): +400
        adjust_cash_holding(
            &conn,
            &account_id,
            "USD",
            "US",
            -cash_delta("SELL", "$CASH-USD", 400.0, 0.0),
        )
        .unwrap();
        assert_eq!(get_cash_shares(&conn, &account_id, "USD"), 1000.0);
    }

    #[test]
    fn test_validate_transaction_shares_allows_cash() {
        // Cash transactions have shares=0 and must pass validation.
        // Non-cash symbols with shares=0 must still be rejected.
        assert!(validate_transaction_shares("US", "AAPL", 0.0, "BUY").is_err());
        assert!(validate_transaction_shares("US", "AAPL", 0.0, "SELL").is_err());
    }

    #[test]
    fn test_validate_transaction_shares_rejects_non_cash_zero() {
        assert!(validate_transaction_shares("HK", "0700.HK", 0.0, "BUY").is_err());
        assert!(validate_transaction_shares("CN", "sh600519", 0.0, "SELL").is_err());
    }

    #[test]
    fn test_cash_withdraw_over_balance_rejected() {
        let (db, account_id) = db_with_account();
        let conn = db.conn.lock().unwrap();
        // Deposit first: a BUY on $CASH-* credits the cash holding (+100).
        adjust_cash_holding(
            &conn,
            &account_id,
            "USD",
            "US",
            cash_delta("BUY", "$CASH-USD", 100.0, 0.0),
        )
        .unwrap();
        let err = validate_cash_withdrawal(&conn, &account_id, "$CASH-USD", 500.0)
            .expect_err("over-withdrawal must be rejected");
        assert!(err.contains("Cannot withdraw"), "got: {}", err);
        // Within balance passes
        assert!(validate_cash_withdrawal(&conn, &account_id, "$CASH-USD", 100.0).is_ok());
    }

    #[test]
    fn test_cash_delta_sign_flip_for_cash_symbols() {
        assert_eq!(cash_delta("BUY", "$CASH-USD", 100.0, 0.0), 100.0);
        assert_eq!(cash_delta("SELL", "$CASH-USD", 40.0, 0.0), -40.0);
        assert_eq!(cash_delta("BUY", "AAPL", 100.0, 1.0), -101.0);
        assert_eq!(cash_delta("SELL", "AAPL", 100.0, 1.0), 99.0);
    }

    #[test]
    fn test_validate_transaction_shares_cash_symbol_allowed() {
        // Cash symbols with shares=0 must pass (deposit/withdraw)
        assert!(validate_transaction_shares("US", "$CASH-USD", 0.0, "BUY").is_ok());
        assert!(validate_transaction_shares("HK", "$CASH-HKD", 0.0, "SELL").is_ok());
        assert!(validate_transaction_shares("CN", "$CASH-CNY", 0.0, "SELL").is_ok());
    }
}
