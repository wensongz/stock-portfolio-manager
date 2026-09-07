use crate::db::Database;
use crate::models::Holding;
use crate::services::portfolio_mutation::{create_holding_in, CreateHoldingInput};
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub fn get_cash_balance_reconciliation(
    db: State<Database>,
    id: String,
) -> Result<crate::services::cash_reconciliation_service::CashBalanceReconciliation, String> {
    crate::services::cash_reconciliation_service::get_cash_balance_reconciliation(db.inner(), &id)
}

#[tauri::command(rename_all = "camelCase")]
pub fn correct_cash_balance(
    db: State<Database>,
    id: String,
    balance: f64,
    expected_revision: i64,
    name: String,
    category_id: Option<String>,
) -> Result<Holding, String> {
    crate::services::cash_reconciliation_service::correct_cash_balance(
        db.inner(),
        &id,
        balance,
        expected_revision,
        name,
        category_id,
    )
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
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
    let input = CreateHoldingInput {
        account_id,
        symbol,
        name,
        market,
        category_id,
        shares,
        avg_cost,
        currency,
    };
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let holding = create_holding_in(&transaction, &input)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(holding)
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
#[allow(clippy::too_many_arguments)]
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
    crate::services::holding_edit::update_holding(
        db.inner(),
        id,
        CreateHoldingInput {
            account_id,
            symbol,
            name,
            market,
            category_id,
            shares,
            avg_cost,
            currency,
        },
    )
}

#[tauri::command(rename_all = "camelCase")]
pub fn delete_holding(db: State<Database>, id: String) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| e.to_string())?;
    let result = (|| -> Result<(), String> {
        // Delete all transactions that belong to this holding (including the
        // initial BUY record created by create_holding).
        conn.execute(
            "DELETE FROM transactions WHERE holding_id = ?1",
            rusqlite::params![id],
        )
        .map_err(|e| e.to_string())?;
        conn.execute("DELETE FROM holdings WHERE id = ?1", rusqlite::params![id])
            .map_err(|e| e.to_string())?;
        Ok(())
    })();
    match result {
        Ok(()) => conn.execute_batch("COMMIT").map_err(|e| e.to_string()),
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::services::portfolio_mutation::validate_holding_values;

    #[test]
    fn validate_holding_values_rejects_invalid_average_cost() {
        for avg_cost in [-0.01, f64::NAN, f64::INFINITY] {
            assert!(validate_holding_values("US", "AAPL", 1.0, avg_cost, "USD").is_err());
        }
    }

    #[test]
    fn validate_holding_values_accepts_zero_average_cost() {
        assert!(validate_holding_values("US", "AAPL", 1.0, 0.0, "USD").is_ok());
    }
}
