use crate::models::{Holding, Transaction};
use crate::services::position_replay::{rebuild_position_group, PositionKey};
use crate::services::quote_service::{cash_display_name, is_cash_symbol, CASH_SYMBOL_PREFIX};
use rusqlite::{Connection, OptionalExtension};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    // OPEN stores a cost baseline, which may legitimately be negative after
    // distributions. Trade proceeds and dividend amounts use unsigned gross
    // amounts; direction comes from the transaction type. Keep signed
    // commissions so broker fee refunds remain representable.
    if input.transaction_type != "OPEN" && input.total_amount < 0.0 {
        return Err("Transaction total_amount must be non-negative".to_string());
    }
    if matches!(input.transaction_type.as_str(), "BUY" | "SELL") && input.price < 0.0 {
        return Err("Transaction price must be non-negative".to_string());
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

/// Recover the existing balance of a legacy position that has no ledger yet.
/// Its creation time is the only known baseline date; never backdate it merely
/// to make a historical import pass.
fn ensure_legacy_opening(
    conn: &Connection,
    holding_id: &str,
    input: &CreateTransactionInput,
) -> Result<(), String> {
    let has_history: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM transactions WHERE account_id=?1 AND UPPER(symbol)=UPPER(?2))",
        rusqlite::params![input.account_id, input.symbol],
        |row| row.get(0),
    ).map_err(|error| error.to_string())?;
    if has_history || input.transaction_type == "OPEN" {
        return Ok(());
    }
    let created_at: String = conn
        .query_row(
            "SELECT created_at FROM holdings WHERE id=?1",
            [holding_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if input.traded_at < created_at {
        return Err(
            "该持仓没有交易历史，成交时间早于已知期初日期；请先核查并补齐期初记录。".into(),
        );
    }
    conn.execute(
        "INSERT INTO transactions (id,holding_id,account_id,symbol,name,market,transaction_type,shares,price,total_amount,commission,currency,traded_at,notes,created_at)
         SELECT ?1,id,account_id,symbol,name,market,'OPEN',shares,avg_cost,shares*avg_cost,0,currency,created_at,'legacy:initial-position',created_at
         FROM holdings WHERE id=?2",
        rusqlite::params![uuid::Uuid::new_v4().to_string(), holding_id],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

pub fn create_transaction_in(
    conn: &Connection,
    input: &CreateTransactionInput,
) -> Result<Transaction, String> {
    require_caller_transaction(conn)?;
    validate_transaction_values(input)?;
    let transaction_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().to_rfc3339();
    let holding_id: Option<String> = conn
        .query_row(
            "SELECT id FROM holdings WHERE account_id = ?1 AND UPPER(symbol) = UPPER(?2)",
            rusqlite::params![input.account_id, input.symbol],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;

    if !is_cash_symbol(&input.symbol) {
        if let Some(id) = holding_id.as_deref() {
            ensure_legacy_opening(conn, id, input)?;
        }
    } else if input.transaction_type == "SELL" {
        validate_cash_withdrawal(conn, &input.account_id, &input.symbol, input.total_amount)?;
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

    if !is_cash_symbol(&input.symbol) {
        rebuild_position_group(conn, &PositionKey::new(&input.account_id, &input.symbol))?;
    }

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

fn transaction_by_id(conn: &Connection, id: &str) -> Result<Transaction, String> {
    conn.query_row(
        "SELECT id, holding_id, account_id, symbol, name, market, transaction_type,
                shares, price, total_amount, commission, currency, traded_at, notes, created_at
         FROM transactions WHERE id = ?1",
        rusqlite::params![id],
        |row| {
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
        },
    )
    .map_err(|error| format!("Transaction not found: {error}"))
}

fn require_caller_transaction(conn: &Connection) -> Result<(), String> {
    if conn.is_autocommit() {
        return Err(
            "Transaction mutation requires a caller-owned database transaction".to_string(),
        );
    }
    Ok(())
}

pub(crate) fn update_transaction_in(
    conn: &Connection,
    id: &str,
    input: &CreateTransactionInput,
) -> Result<Transaction, String> {
    require_caller_transaction(conn)?;
    validate_transaction_values(input)?;
    let old_transaction = transaction_by_id(conn, id)?;
    if old_transaction.transaction_type == "OPEN" {
        return Err("Cannot edit the initial position-opening record".to_string());
    }

    let old_key = (!is_cash_symbol(&old_transaction.symbol))
        .then(|| PositionKey::new(&old_transaction.account_id, &old_transaction.symbol));
    let new_key = (!is_cash_symbol(&input.symbol))
        .then(|| PositionKey::new(&input.account_id, &input.symbol));

    let old_cash_delta = cash_delta(
        &old_transaction.transaction_type,
        &old_transaction.symbol,
        old_transaction.total_amount,
        old_transaction.commission,
    );
    adjust_cash_holding(
        conn,
        &old_transaction.account_id,
        &old_transaction.currency,
        &old_transaction.market,
        -old_cash_delta,
    )?;

    if is_cash_symbol(&input.symbol) && input.transaction_type == "SELL" {
        validate_cash_withdrawal(conn, &input.account_id, &input.symbol, input.total_amount)?;
    }

    let existing_cash_holding: Option<String> = if is_cash_symbol(&input.symbol) {
        conn.query_row(
            "SELECT id FROM holdings WHERE account_id = ?1 AND UPPER(symbol) = UPPER(?2)",
            rusqlite::params![input.account_id, input.symbol],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
    } else {
        None
    };

    conn.execute(
        "UPDATE transactions
         SET holding_id = ?2, account_id = ?3, symbol = ?4, name = ?5, market = ?6,
             transaction_type = ?7, shares = ?8, price = ?9, total_amount = ?10,
             commission = ?11, currency = ?12, traded_at = ?13, notes = ?14
         WHERE id = ?1",
        rusqlite::params![
            id,
            existing_cash_holding,
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
            input.notes
        ],
    )
    .map_err(|error| error.to_string())?;

    let new_cash_delta = cash_delta(
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
        new_cash_delta,
    )?;

    if let Some(key) = old_key.as_ref() {
        rebuild_position_group(conn, key)?;
    }
    if let Some(key) = new_key
        .as_ref()
        .filter(|key| old_key.as_ref() != Some(*key))
    {
        rebuild_position_group(conn, key)?;
    }

    transaction_by_id(conn, id)
}

pub(crate) fn delete_transaction_in(conn: &Connection, id: &str) -> Result<(), String> {
    require_caller_transaction(conn)?;
    let transaction = transaction_by_id(conn, id)?;
    if transaction.transaction_type == "OPEN" {
        return Err("Cannot delete the initial position-opening record".to_string());
    }

    conn.execute(
        "DELETE FROM transactions WHERE id = ?1",
        rusqlite::params![id],
    )
    .map_err(|error| error.to_string())?;

    let delta = cash_delta(
        &transaction.transaction_type,
        &transaction.symbol,
        transaction.total_amount,
        transaction.commission,
    );
    adjust_cash_holding(
        conn,
        &transaction.account_id,
        &transaction.currency,
        &transaction.market,
        -delta,
    )?;

    if !is_cash_symbol(&transaction.symbol) {
        rebuild_position_group(
            conn,
            &PositionKey::new(&transaction.account_id, &transaction.symbol),
        )?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        create_holding_in, create_transaction_in, delete_transaction_in, update_transaction_in,
        CreateHoldingInput, CreateTransactionInput,
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

    fn stock_transaction(
        transaction_type: &str,
        shares: f64,
        price: f64,
        total_amount: f64,
        traded_at: &str,
    ) -> CreateTransactionInput {
        CreateTransactionInput {
            account_id: "account-1".to_string(),
            symbol: "AAPL".to_string(),
            name: "Apple".to_string(),
            market: "US".to_string(),
            transaction_type: transaction_type.to_string(),
            shares,
            price,
            total_amount,
            commission: 0.0,
            currency: "USD".to_string(),
            traded_at: traded_at.to_string(),
            notes: None,
        }
    }

    fn seed_buy_then_sell(database: &Database) -> (String, String) {
        let mut connection = database.conn.lock().unwrap();
        let transaction = connection.transaction().unwrap();
        let buy = create_transaction_in(
            &transaction,
            &stock_transaction("BUY", 10.0, 10.0, 100.0, "2026-01-01"),
        )
        .unwrap();
        let sell = create_transaction_in(
            &transaction,
            &stock_transaction("SELL", 10.0, 12.0, 120.0, "2026-01-02"),
        )
        .unwrap();
        transaction.commit().unwrap();
        (buy.id, sell.id)
    }

    fn portfolio_counts(database: &Database) -> (i64, f64, f64) {
        let connection = database.conn.lock().unwrap();
        let transaction_count = connection
            .query_row("SELECT COUNT(*) FROM transactions", [], |row| row.get(0))
            .unwrap();
        let stock_shares = connection
            .query_row(
                "SELECT shares FROM holdings WHERE symbol = 'AAPL'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let cash_shares = connection
            .query_row(
                "SELECT shares FROM holdings WHERE symbol = '$CASH-USD'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        (transaction_count, stock_shares, cash_shares)
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
        let mut connection = db.conn.lock().unwrap();
        let conn = connection.transaction().unwrap();

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

    #[test]
    fn deleting_an_earlier_buy_cannot_leave_a_historical_sell_unfunded() {
        let database = database_with_account();
        let (buy_id, _) = seed_buy_then_sell(&database);
        let before = portfolio_counts(&database);

        let error = {
            let mut connection = database.conn.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            let error = delete_transaction_in(&transaction, &buy_id).unwrap_err();
            transaction.rollback().unwrap();
            error
        };

        assert!(error.contains("historical position"), "got: {error}");
        assert_eq!(portfolio_counts(&database), before);
    }

    #[test]
    fn shrinking_an_earlier_buy_rolls_back_when_later_sells_exceed_it() {
        let database = database_with_account();
        let (buy_id, _) = seed_buy_then_sell(&database);
        let before = portfolio_counts(&database);

        let error = {
            let mut connection = database.conn.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            let error = update_transaction_in(
                &transaction,
                &buy_id,
                &stock_transaction("BUY", 5.0, 10.0, 50.0, "2026-01-01"),
            )
            .unwrap_err();
            transaction.rollback().unwrap();
            error
        };

        assert!(error.contains("historical position"), "got: {error}");
        assert_eq!(portfolio_counts(&database), before);
    }

    #[test]
    fn moving_a_sell_to_a_symbol_without_a_position_rolls_back() {
        let database = database_with_account();
        let (_, sell_id) = seed_buy_then_sell(&database);
        let before = portfolio_counts(&database);
        let mut moved_sell = stock_transaction("SELL", 10.0, 12.0, 120.0, "2026-01-02");
        moved_sell.symbol = "MSFT".to_string();
        moved_sell.name = "Microsoft".to_string();

        let error = {
            let mut connection = database.conn.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            let error = update_transaction_in(&transaction, &sell_id, &moved_sell).unwrap_err();
            transaction.rollback().unwrap();
            error
        };

        assert!(error.contains("historical position"), "got: {error}");
        assert_eq!(portfolio_counts(&database), before);
        let msft_transactions: i64 = database
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM transactions WHERE symbol = 'MSFT'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(msft_transactions, 0);
    }

    #[test]
    fn a_valid_historical_buy_edit_rebuilds_holding_and_cash() {
        let database = database_with_account();
        let (buy_id, _) = seed_buy_then_sell(&database);

        {
            let mut connection = database.conn.lock().unwrap();
            let transaction = connection.transaction().unwrap();
            update_transaction_in(
                &transaction,
                &buy_id,
                &stock_transaction("BUY", 12.0, 10.0, 120.0, "2026-01-01"),
            )
            .unwrap();
            transaction.commit().unwrap();
        }

        assert_eq!(portfolio_counts(&database), (2, 2.0, 0.0));
    }

    #[test]
    fn inserting_an_earlier_buy_uses_historical_average_cost() {
        let database = database_with_account();
        let mut conn = database.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        for input in [
            stock_transaction("BUY", 10.0, 10.0, 100.0, "2026-01-01"),
            stock_transaction("SELL", 5.0, 30.0, 150.0, "2026-01-03"),
            stock_transaction("BUY", 10.0, 20.0, 200.0, "2026-01-02"),
        ] {
            create_transaction_in(&tx, &input).unwrap();
        }
        let position: (f64, f64) = tx
            .query_row(
                "SELECT shares, avg_cost FROM holdings WHERE symbol='AAPL'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(position, (15.0, 15.0));
    }

    #[test]
    fn inserting_a_sell_before_its_buy_rejects_and_rolls_back() {
        let database = database_with_account();
        {
            let mut conn = database.conn.lock().unwrap();
            let tx = conn.transaction().unwrap();
            create_transaction_in(
                &tx,
                &stock_transaction("BUY", 10.0, 10.0, 100.0, "2026-01-02"),
            )
            .unwrap();
            tx.commit().unwrap();
        }
        let before = portfolio_counts(&database);
        {
            let mut conn = database.conn.lock().unwrap();
            let tx = conn.transaction().unwrap();
            let error = create_transaction_in(
                &tx,
                &stock_transaction("SELL", 1.0, 20.0, 20.0, "2026-01-01"),
            )
            .unwrap_err();
            assert!(error.contains("historical position"), "{error}");
            tx.rollback().unwrap();
        }
        assert_eq!(portfolio_counts(&database), before);
    }

    #[test]
    fn fractional_buys_can_be_sold_in_full_without_a_dust_position() {
        let database = database_with_account();
        let mut conn = database.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        for input in [
            stock_transaction("BUY", 0.1, 10.0, 1.0, "2026-01-01"),
            stock_transaction("BUY", 0.7, 10.0, 7.0, "2026-01-02"),
            stock_transaction("SELL", 0.8, 10.0, 8.0, "2026-01-03"),
        ] {
            create_transaction_in(&tx, &input).unwrap();
        }
        let shares: f64 = tx
            .query_row("SELECT shares FROM holdings WHERE symbol='AAPL'", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(shares, 0.0);
    }

    #[test]
    fn negative_buy_amount_is_rejected_before_any_account_write() {
        let database = database_with_account();
        let mut conn = database.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        let error = create_transaction_in(
            &tx,
            &stock_transaction("BUY", 1.0, 10.0, -100.0, "2026-01-01"),
        )
        .unwrap_err();
        assert!(error.contains("total_amount"), "{error}");
        let counts: (i64, i64) = tx
            .query_row(
                "SELECT (SELECT COUNT(*) FROM holdings), (SELECT COUNT(*) FROM transactions)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(counts, (0, 0));
    }

    #[test]
    fn financial_validation_preserves_rebates_and_signed_open_baselines() {
        for kind in ["BUY", "SELL", "PAY"] {
            let mut input = stock_transaction(kind, 1.0, 10.0, -1.0, "2026-01-01");
            assert!(
                super::validate_transaction_values(&input).is_err(),
                "{kind} negative amount"
            );
            input.total_amount = 0.0;
            input.price = 0.0;
            input.commission = -0.01;
            assert!(
                super::validate_transaction_values(&input).is_ok(),
                "{kind} zero value with fee refund"
            );
        }
        for kind in ["BUY", "SELL"] {
            let input = stock_transaction(kind, 1.0, -10.0, 10.0, "2026-01-01");
            assert!(
                super::validate_transaction_values(&input).is_err(),
                "{kind} negative price"
            );
        }
        let open = stock_transaction("OPEN", 10.0, -2.0, -20.0, "2026-01-01");
        assert!(super::validate_transaction_values(&open).is_ok());
    }

    #[test]
    fn historyless_holding_gets_a_dated_open_before_its_first_trade() {
        let database = database_with_account();
        let mut conn = database.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        tx.execute("INSERT INTO holdings (id,account_id,symbol,name,market,shares,avg_cost,currency,created_at,updated_at) VALUES ('legacy','account-1','AAPL','Apple','US',10,20,'USD','2026-01-01','2026-01-01')", []).unwrap();
        create_transaction_in(
            &tx,
            &stock_transaction("BUY", 10.0, 30.0, 300.0, "2026-01-02"),
        )
        .unwrap();
        create_transaction_in(
            &tx,
            &stock_transaction("SELL", 5.0, 40.0, 200.0, "2026-01-03"),
        )
        .unwrap();
        let baseline: (f64, f64, String) = tx
            .query_row(
                "SELECT shares,price,traded_at FROM transactions WHERE transaction_type='OPEN'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        assert_eq!(baseline, (10.0, 20.0, "2026-01-01".into()));
        let position: (f64, f64) = tx
            .query_row(
                "SELECT shares,avg_cost FROM holdings WHERE id='legacy'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(position, (15.0, 25.0));
        let cash: f64 = tx
            .query_row(
                "SELECT shares FROM holdings WHERE symbol='$CASH-USD'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(cash, -100.0);
    }

    #[test]
    fn first_legacy_trade_cannot_silently_move_the_opening_date() {
        let database = database_with_account();
        let mut conn = database.conn.lock().unwrap();
        let tx = conn.transaction().unwrap();
        tx.execute("INSERT INTO holdings (id,account_id,symbol,name,market,shares,avg_cost,currency,created_at,updated_at) VALUES ('legacy','account-1','AAPL','Apple','US',10,20,'USD','2026-01-02','2026-01-02')", []).unwrap();
        let error = create_transaction_in(
            &tx,
            &stock_transaction("BUY", 1.0, 10.0, 10.0, "2026-01-01"),
        )
        .unwrap_err();
        assert!(error.contains("期初"), "{error}");
        let count: i64 = tx
            .query_row("SELECT COUNT(*) FROM transactions", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 0);
    }
}
