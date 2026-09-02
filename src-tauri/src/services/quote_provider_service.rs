use crate::db::Database;
use crate::models::quote_provider::QuoteProviderConfig;
use crate::services::position_replay::rebuild_all_position_groups;
use chrono::Utc;
use rusqlite::Connection;

pub fn get_quote_provider_config(db: &Database) -> Result<QuoteProviderConfig, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    get_quote_provider_config_in(&conn)
}

fn get_quote_provider_config_in(conn: &Connection) -> Result<QuoteProviderConfig, String> {
    let result = conn.query_row(
        "SELECT us_provider, hk_provider, cn_provider, xueqiu_cookie, xueqiu_u,
                cn_adjust_sell_pay_cost, us_adjust_sell_pay_cost, hk_adjust_sell_pay_cost
         FROM quote_provider_config WHERE id = 1",
        [],
        |row| {
            Ok(QuoteProviderConfig {
                us_provider: row.get(0)?,
                hk_provider: row.get(1)?,
                cn_provider: row.get(2)?,
                xueqiu_cookie: row.get(3)?,
                xueqiu_u: row.get(4)?,
                cn_adjust_sell_pay_cost: row.get::<_, i64>(5)? != 0,
                us_adjust_sell_pay_cost: row.get::<_, i64>(6)? != 0,
                hk_adjust_sell_pay_cost: row.get::<_, i64>(7)? != 0,
            })
        },
    );

    match result {
        Ok(config) => Ok(config),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(QuoteProviderConfig::default()),
        Err(error) => Err(error.to_string()),
    }
}

fn validate_quote_provider_config(config: &QuoteProviderConfig) -> Result<(), String> {
    match config.us_provider.as_str() {
        "yahoo" | "eastmoney" | "xueqiu" => {}
        _ => return Err(format!("Invalid US provider: {}", config.us_provider)),
    }
    match config.hk_provider.as_str() {
        "yahoo" | "eastmoney" | "xueqiu" => {}
        _ => return Err(format!("Invalid HK provider: {}", config.hk_provider)),
    }
    match config.cn_provider.as_str() {
        "eastmoney" | "xueqiu" => {}
        _ => {
            return Err(format!(
                "Invalid CN provider ({}). Only 'eastmoney' and 'xueqiu' are supported.",
                config.cn_provider
            ))
        }
    }
    Ok(())
}

fn persist_quote_provider_config(
    conn: &Connection,
    config: &QuoteProviderConfig,
) -> Result<(), String> {
    let now = Utc::now().to_rfc3339();

    let xueqiu_cookie = config
        .xueqiu_cookie
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let xueqiu_u = config
        .xueqiu_u
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string);

    conn.execute(
        "INSERT INTO quote_provider_config
             (id, us_provider, hk_provider, cn_provider, xueqiu_cookie, xueqiu_u,
              cn_adjust_sell_pay_cost, us_adjust_sell_pay_cost, hk_adjust_sell_pay_cost, updated_at)
         VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
         ON CONFLICT(id) DO UPDATE SET
           us_provider = excluded.us_provider,
           hk_provider = excluded.hk_provider,
           cn_provider = excluded.cn_provider,
           xueqiu_cookie = excluded.xueqiu_cookie,
           xueqiu_u = excluded.xueqiu_u,
           cn_adjust_sell_pay_cost = excluded.cn_adjust_sell_pay_cost,
           us_adjust_sell_pay_cost = excluded.us_adjust_sell_pay_cost,
           hk_adjust_sell_pay_cost = excluded.hk_adjust_sell_pay_cost,
           updated_at = excluded.updated_at",
        rusqlite::params![
            config.us_provider,
            config.hk_provider,
            config.cn_provider,
            xueqiu_cookie,
            xueqiu_u,
            config.cn_adjust_sell_pay_cost as i64,
            config.us_adjust_sell_pay_cost as i64,
            config.hk_adjust_sell_pay_cost as i64,
            now
        ],
    )
    .map_err(|error| error.to_string())?;
    Ok(())
}

pub fn update_quote_provider_config(
    db: &Database,
    config: &QuoteProviderConfig,
) -> Result<bool, String> {
    validate_quote_provider_config(config)?;
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let previous = get_quote_provider_config_in(&transaction)?;
    let cost_policy_changed = previous.cn_adjust_sell_pay_cost != config.cn_adjust_sell_pay_cost
        || previous.us_adjust_sell_pay_cost != config.us_adjust_sell_pay_cost
        || previous.hk_adjust_sell_pay_cost != config.hk_adjust_sell_pay_cost;

    persist_quote_provider_config(&transaction, config)?;
    if cost_policy_changed {
        rebuild_all_position_groups(&transaction)?;
    }
    transaction.commit().map_err(|error| error.to_string())?;

    Ok(true)
}

/// Return whether SELL and PAY transactions should adjust avg_cost for the given market.
/// Reads from the single-row `quote_provider_config` table.
/// Defaults: CN = true, US = false, HK = false.
pub fn market_adjusts_sell_pay_cost(conn: &rusqlite::Connection, market: &str) -> bool {
    // Map market to a fixed SQL query — never interpolate user input into SQL.
    let (query, default_val): (&str, i64) = match market {
        "CN" => (
            "SELECT cn_adjust_sell_pay_cost FROM quote_provider_config WHERE id = 1",
            1,
        ),
        "US" => (
            "SELECT us_adjust_sell_pay_cost FROM quote_provider_config WHERE id = 1",
            0,
        ),
        "HK" => (
            "SELECT hk_adjust_sell_pay_cost FROM quote_provider_config WHERE id = 1",
            0,
        ),
        _ => return true, // unknown market: safe default (adjust)
    };
    conn.query_row(query, [], |row| row.get::<_, i64>(0))
        .unwrap_or(default_val)
        != 0
}

#[cfg(test)]
mod tests {
    use super::{get_quote_provider_config, update_quote_provider_config};
    use crate::db::Database;
    use crate::services::portfolio_mutation::{create_transaction_in, CreateTransactionInput};

    #[test]
    fn schema_errors_are_not_reported_as_default_config() {
        let db = Database::new(":memory:").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute_batch(
                "DROP TABLE quote_provider_config;
                 CREATE TABLE quote_provider_config (id INTEGER PRIMARY KEY, us_provider TEXT);
                 INSERT INTO quote_provider_config VALUES (1, 'broken');",
            )
            .unwrap();
        }

        let error = get_quote_provider_config(&db).unwrap_err();
        assert!(error.contains("hk_provider") || error.contains("column"));
    }

    #[test]
    fn cost_policy_update_rolls_back_config_when_position_rebuild_fails() {
        let db = Database::new(":memory:").unwrap();
        {
            let mut conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
                 VALUES ('account-1', 'Portfolio', 'US', '2026-01-01', '2026-01-01')",
                [],
            )
            .unwrap();
            let transaction = conn.transaction().unwrap();
            for input in [
                CreateTransactionInput {
                    account_id: "account-1".to_string(),
                    symbol: "AAPL".to_string(),
                    name: "Apple".to_string(),
                    market: "US".to_string(),
                    transaction_type: "BUY".to_string(),
                    shares: 10.0,
                    price: 10.0,
                    total_amount: 100.0,
                    commission: 0.0,
                    currency: "USD".to_string(),
                    traded_at: "2026-01-01".to_string(),
                    notes: None,
                },
                CreateTransactionInput {
                    account_id: "account-1".to_string(),
                    symbol: "AAPL".to_string(),
                    name: "Apple".to_string(),
                    market: "US".to_string(),
                    transaction_type: "SELL".to_string(),
                    shares: 4.0,
                    price: 20.0,
                    total_amount: 80.0,
                    commission: 0.0,
                    currency: "USD".to_string(),
                    traded_at: "2026-01-02".to_string(),
                    notes: None,
                },
            ] {
                create_transaction_in(&transaction, &input).unwrap();
            }
            transaction.commit().unwrap();
            conn.execute_batch(
                "CREATE TRIGGER reject_cost_rebuild
                 BEFORE UPDATE OF shares, avg_cost ON holdings
                 WHEN OLD.symbol = 'AAPL'
                 BEGIN
                   SELECT RAISE(ABORT, 'forced holding rebuild failure');
                 END;",
            )
            .unwrap();
        }

        let mut updated = get_quote_provider_config(&db).unwrap();
        assert!(!updated.us_adjust_sell_pay_cost);
        updated.us_adjust_sell_pay_cost = true;

        let error = update_quote_provider_config(&db, &updated).unwrap_err();

        assert!(
            error.contains("forced holding rebuild failure"),
            "got: {error}"
        );
        assert!(
            !get_quote_provider_config(&db)
                .unwrap()
                .us_adjust_sell_pay_cost
        );
        let holding: (f64, f64) = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT shares, avg_cost FROM holdings WHERE symbol = 'AAPL'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(holding, (6.0, 10.0));
    }
}
