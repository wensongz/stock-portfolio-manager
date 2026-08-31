use crate::models::{Holding, Transaction};
use crate::services::quote_provider_service::market_adjusts_sell_pay_cost;
use crate::services::quote_service::{cash_display_name, is_cash_symbol, CASH_SYMBOL_PREFIX};
use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone)]
pub struct CreateHoldingInput {
    pub account_id: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub category_id: Option<String>,
    pub shares: f64,
    pub avg_cost: f64,
    pub currency: String,
}

#[derive(Debug, Clone)]
pub struct CreateTransactionInput {
    pub account_id: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub transaction_type: String,
    pub shares: f64,
    pub price: f64,
    pub total_amount: f64,
    pub commission: f64,
    pub currency: String,
    pub traded_at: String,
    pub notes: Option<String>,
}

fn validate_market_and_currency(market: &str, currency: &str) -> Result<(), String> {
    if !matches!(market, "US" | "CN" | "HK") {
        return Err(format!("Unsupported market: {market}"));
    }
    if !matches!(currency, "USD" | "CNY" | "HKD") {
        return Err(format!("Unsupported currency: {currency}"));
    }
    Ok(())
}

pub(crate) fn validate_holding_values(
    market: &str,
    symbol: &str,
    shares: f64,
    avg_cost: f64,
    currency: &str,
) -> Result<(), String> {
    validate_market_and_currency(market, currency)?;
    if !shares.is_finite() || shares < 0.0 {
        return Err("Holding shares must be a non-negative number".to_string());
    }
    if !symbol.starts_with("$CASH-") && market != "US" && shares.fract().abs() > 1e-9 {
        return Err(
            "Only US holdings support fractional shares; CN and HK holdings must use whole shares"
                .to_string(),
        );
    }
    if !avg_cost.is_finite() || avg_cost < 0.0 {
        return Err("Holding average cost must be a non-negative number".to_string());
    }
    Ok(())
}

pub(crate) fn validate_transaction_shares(
    market: &str,
    symbol: &str,
    shares: f64,
    transaction_type: &str,
) -> Result<(), String> {
    if transaction_type == "PAY" || is_cash_symbol(symbol) {
        return if shares.is_finite() {
            Ok(())
        } else {
            Err("Transaction shares must be finite".to_string())
        };
    }
    if !shares.is_finite() || shares <= 0.0 {
        return Err("Transaction shares must be a positive number".to_string());
    }
    if market != "US" && shares.fract().abs() > 1e-9 {
        return Err("Only US transactions support fractional shares; CN and HK transactions must use whole shares".to_string());
    }
    Ok(())
}

pub(crate) fn validate_transaction_values(input: &CreateTransactionInput) -> Result<(), String> {
    validate_market_and_currency(&input.market, &input.currency)?;
    if !matches!(
        input.transaction_type.as_str(),
        "BUY" | "SELL" | "OPEN" | "PAY"
    ) {
        return Err(format!(
            "Unsupported transaction type: {}",
            input.transaction_type
        ));
    }
    validate_transaction_shares(
        &input.market,
        &input.symbol,
        input.shares,
        &input.transaction_type,
    )?;
    for (label, value) in [
        ("price", input.price),
        ("total_amount", input.total_amount),
        ("commission", input.commission),
    ] {
        if !value.is_finite() {
            return Err(format!("Transaction {label} must be finite"));
        }
    }
    Ok(())
}

pub(crate) fn cash_delta(
    transaction_type: &str,
    symbol: &str,
    total_amount: f64,
    commission: f64,
) -> f64 {
    if is_cash_symbol(symbol) {
        return match transaction_type {
            "BUY" => total_amount + commission,
            "SELL" => -(total_amount + commission),
            _ => 0.0,
        };
    }
    match transaction_type {
        "BUY" => -(total_amount + commission),
        "SELL" | "PAY" => total_amount - commission,
        "OPEN" => 0.0,
        other => panic!("Unexpected transaction_type for cash_delta: {other}"),
    }
}

pub(crate) fn adjust_cash_holding(
    conn: &Connection,
    account_id: &str,
    currency: &str,
    market: &str,
    delta: f64,
) -> Result<(), String> {
    let cash_symbol = format!("{CASH_SYMBOL_PREFIX}{currency}");
    let existing: Option<(String, f64)> = conn
        .query_row(
            "SELECT id, shares FROM holdings WHERE account_id = ?1 AND symbol = ?2",
            rusqlite::params![account_id, cash_symbol],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let updated_at = chrono::Utc::now().to_rfc3339();

    if let Some((cash_id, current_shares)) = existing {
        conn.execute(
            "UPDATE holdings SET shares = ?2, updated_at = ?3 WHERE id = ?1",
            rusqlite::params![cash_id, current_shares + delta, updated_at],
        )
        .map_err(|error| error.to_string())?;
    } else {
        let cash_id = uuid::Uuid::new_v4().to_string();
        let cash_name = cash_display_name(&cash_symbol);
        conn.execute(
            "INSERT INTO holdings (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, 1.0, ?7, ?8, ?9)",
            rusqlite::params![
                cash_id,
                account_id,
                cash_symbol,
                cash_name,
                market,
                delta,
                currency,
                updated_at,
                updated_at
            ],
        )
        .map_err(|error| error.to_string())?;
    }
    Ok(())
}

pub(crate) fn validate_cash_withdrawal(
    conn: &Connection,
    account_id: &str,
    symbol: &str,
    total_amount: f64,
) -> Result<(), String> {
    if !total_amount.is_finite() || total_amount <= 0.0 {
        return Err(format!("Invalid cash amount: {total_amount}"));
    }
    let balance: f64 = conn
        .query_row(
            "SELECT shares FROM holdings WHERE account_id = ?1 AND UPPER(symbol) = UPPER(?2)",
            rusqlite::params![account_id, symbol],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .unwrap_or(0.0);
    if total_amount > balance {
        return Err(format!(
            "Cannot withdraw {total_amount}: only {balance} cash available"
        ));
    }
    Ok(())
}

pub fn create_holding_in(conn: &Connection, input: &CreateHoldingInput) -> Result<Holding, String> {
    validate_holding_values(
        &input.market,
        &input.symbol,
        input.shares,
        input.avg_cost,
        &input.currency,
    )?;
    let exists = conn
        .query_row(
            "SELECT 1 FROM holdings WHERE account_id = ?1 AND UPPER(symbol) = UPPER(?2) LIMIT 1",
            rusqlite::params![input.account_id, input.symbol],
            |_| Ok(()),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .is_some();
    if exists {
        return Err(format!(
            "账户中已存在股票代码为「{}」的持仓记录。若需调整持仓数量，请前往「交易记录」页面新增买入或卖出记录，而非重复创建持仓。",
            input.symbol
        ));
    }

    let id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    conn.execute(
        "INSERT INTO holdings (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        rusqlite::params![
            id,
            input.account_id,
            input.symbol,
            input.name,
            input.market,
            input.category_id,
            input.shares,
            input.avg_cost,
            input.currency,
            now,
            now
        ],
    )
    .map_err(|error| error.to_string())?;

    let transaction_id = uuid::Uuid::new_v4().to_string();
    conn.execute(
        "INSERT INTO transactions (id, holding_id, account_id, symbol, name, market, transaction_type, shares, price, total_amount, commission, currency, traded_at, notes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'OPEN', ?7, ?8, ?9, 0.0, ?10, ?11, NULL, ?12)",
        rusqlite::params![
            transaction_id,
            id,
            input.account_id,
            input.symbol,
            input.name,
            input.market,
            input.shares,
            input.avg_cost,
            input.shares * input.avg_cost,
            input.currency,
            now,
            now
        ],
    )
    .map_err(|error| error.to_string())?;

    Ok(Holding {
        id,
        account_id: input.account_id.clone(),
        symbol: input.symbol.clone(),
        name: input.name.clone(),
        market: input.market.clone(),
        category_id: input.category_id.clone(),
        shares: input.shares,
        avg_cost: input.avg_cost,
        currency: input.currency.clone(),
        created_at: now.clone(),
        updated_at: now,
    })
}

pub fn create_transaction_in(
    conn: &Connection,
    input: &CreateTransactionInput,
) -> Result<Transaction, String> {
    validate_transaction_values(input)?;
    let transaction_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let mut holding_id: Option<String> = conn
        .query_row(
            "SELECT id FROM holdings WHERE account_id = ?1 AND UPPER(symbol) = UPPER(?2)",
            rusqlite::params![input.account_id, input.symbol],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    if !is_cash_symbol(&input.symbol) && holding_id.is_none() && input.transaction_type == "BUY" {
        let new_holding_id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO holdings (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, 0.0, 0.0, ?6, ?7, ?8)",
            rusqlite::params![
                new_holding_id,
                input.account_id,
                input.symbol,
                input.name,
                input.market,
                input.currency,
                now,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        holding_id = Some(new_holding_id);
    }

    if input.transaction_type == "SELL" && !is_cash_symbol(&input.symbol) && holding_id.is_none() {
        return Err(format!(
            "Cannot sell {}: no holding exists in this account",
            input.symbol
        ));
    }
    if is_cash_symbol(&input.symbol) && input.transaction_type == "SELL" {
        validate_cash_withdrawal(conn, &input.account_id, &input.symbol, input.total_amount)?;
    }

    if !is_cash_symbol(&input.symbol) {
        if let Some(ref id) = holding_id {
            let (current_shares, current_avg_cost): (f64, f64) = conn
                .query_row(
                    "SELECT shares, avg_cost FROM holdings WHERE id = ?1",
                    rusqlite::params![id],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|error| error.to_string())?;
            if input.transaction_type == "SELL" && input.shares > current_shares {
                return Err(format!(
                    "Cannot sell {} shares of {}: only {} shares held",
                    input.shares, input.symbol, current_shares
                ));
            }

            let adjust = market_adjusts_sell_pay_cost(conn, &input.market);
            let (new_shares, new_avg_cost) = match input.transaction_type.as_str() {
                "BUY" => {
                    let total_shares = current_shares + input.shares;
                    let average = if total_shares > 0.0 {
                        (current_shares * current_avg_cost
                            + input.shares * input.price
                            + input.commission)
                            / total_shares
                    } else {
                        input.price
                    };
                    (total_shares, average)
                }
                "PAY" => {
                    let net_amount = input.total_amount - input.commission;
                    let average = if adjust && current_shares > 0.0 {
                        (current_shares * current_avg_cost - net_amount) / current_shares
                    } else {
                        current_avg_cost
                    };
                    (current_shares, average)
                }
                "SELL" => {
                    let remaining = current_shares - input.shares;
                    let average = if adjust {
                        if remaining > 0.0 {
                            (current_shares * current_avg_cost - input.total_amount
                                + input.commission)
                                / remaining
                        } else {
                            0.0
                        }
                    } else {
                        current_avg_cost
                    };
                    (remaining, average)
                }
                "OPEN" => (current_shares, current_avg_cost),
                _ => unreachable!("validated transaction type"),
            };
            conn.execute(
                "UPDATE holdings SET shares = ?2, avg_cost = ?3, updated_at = ?4 WHERE id = ?1",
                rusqlite::params![id, new_shares, new_avg_cost, now],
            )
            .map_err(|error| error.to_string())?;
        }
    }

    conn.execute(
        "INSERT INTO transactions (id, holding_id, account_id, symbol, name, market, transaction_type, shares, price, total_amount, commission, currency, traded_at, notes, created_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
        rusqlite::params![
            transaction_id,
            holding_id,
            input.account_id,
            input.symbol,
            input.name,
            input.market,
            input.transaction_type,
            input.shares,
            input.price,
            input.total_amount,
            input.commission,
            input.currency,
            input.traded_at,
            input.notes,
            now
        ],
    )
    .map_err(|error| error.to_string())?;

    let delta = cash_delta(
        &input.transaction_type,
        &input.symbol,
        input.total_amount,
        input.commission,
    );
    adjust_cash_holding(
        conn,
        &input.account_id,
        &input.currency,
        &input.market,
        delta,
    )?;

    let response_holding_id = conn
        .query_row(
            "SELECT id FROM holdings WHERE account_id = ?1 AND UPPER(symbol) = UPPER(?2)",
            rusqlite::params![input.account_id, input.symbol],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    Ok(Transaction {
        id: transaction_id,
        holding_id: response_holding_id,
        account_id: input.account_id.clone(),
        symbol: input.symbol.clone(),
        name: input.name.clone(),
        market: input.market.clone(),
        transaction_type: input.transaction_type.clone(),
        shares: input.shares,
        price: input.price,
        total_amount: input.total_amount,
        commission: input.commission,
        currency: input.currency.clone(),
        traded_at: input.traded_at.clone(),
        notes: input.notes.clone(),
        created_at: now,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        create_holding_in, create_transaction_in, CreateHoldingInput, CreateTransactionInput,
    };
    use crate::db::Database;

    fn database_with_account() -> Database {
        let db = Database::new(":memory:").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
                 VALUES ('account-1', 'Portfolio', 'US', '2026-08-31', '2026-08-31')",
                [],
            )
            .unwrap();
        }
        db
    }

    #[test]
    fn create_holding_in_records_an_open_baseline() {
        let db = database_with_account();
        let conn = db.conn.lock().unwrap();

        let holding = create_holding_in(
            &conn,
            &CreateHoldingInput {
                account_id: "account-1".to_string(),
                symbol: "AAPL".to_string(),
                name: "Apple".to_string(),
                market: "US".to_string(),
                category_id: None,
                shares: 10.0,
                avg_cost: 100.0,
                currency: "USD".to_string(),
            },
        )
        .unwrap();

        let (transaction_type, total_amount): (String, f64) = conn
            .query_row(
                "SELECT transaction_type, total_amount FROM transactions WHERE holding_id = ?1",
                rusqlite::params![holding.id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(transaction_type, "OPEN");
        assert_eq!(total_amount, 1_000.0);
    }

    #[test]
    fn create_transaction_in_updates_position_and_cash_together() {
        let db = database_with_account();
        let conn = db.conn.lock().unwrap();

        create_transaction_in(
            &conn,
            &CreateTransactionInput {
                account_id: "account-1".to_string(),
                symbol: "AAPL".to_string(),
                name: "Apple".to_string(),
                market: "US".to_string(),
                transaction_type: "BUY".to_string(),
                shares: 2.0,
                price: 50.0,
                total_amount: 100.0,
                commission: 3.0,
                currency: "USD".to_string(),
                traded_at: "2026-08-31".to_string(),
                notes: None,
            },
        )
        .unwrap();

        let (shares, avg_cost): (f64, f64) = conn
            .query_row(
                "SELECT shares, avg_cost FROM holdings WHERE symbol = 'AAPL'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let cash: f64 = conn
            .query_row(
                "SELECT shares FROM holdings WHERE symbol = '$CASH-USD'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(shares, 2.0);
        assert_eq!(avg_cost, 51.5);
        assert_eq!(cash, -103.0);
    }
}
