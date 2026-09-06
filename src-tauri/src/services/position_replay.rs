use crate::services::quote_provider_service::market_adjusts_sell_pay_cost;
use rusqlite::Connection;
use std::collections::{HashMap, HashSet};

const POSITION_EPSILON: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct PositionKey {
    account_id: String,
    normalized_symbol: String,
}

impl PositionKey {
    pub(crate) fn new(account_id: &str, symbol: &str) -> Self {
        Self {
            account_id: account_id.to_string(),
            normalized_symbol: symbol.to_uppercase(),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ReplayTransaction {
    pub transaction_type: String,
    pub shares: f64,
    pub price: f64,
    pub total_amount: f64,
    pub commission: f64,
}

#[derive(Debug, Clone)]
struct StoredTransaction {
    symbol: String,
    name: String,
    market: String,
    currency: String,
    replay: ReplayTransaction,
}

#[derive(Debug, Clone)]
struct StoredHolding {
    id: String,
    market: String,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PositionProjection {
    pub shares: f64,
    pub avg_cost: f64,
}

pub(crate) fn replay_transactions(
    transactions: &[ReplayTransaction],
    adjust_sell_pay_cost: bool,
) -> Result<PositionProjection, String> {
    let mut projection = PositionProjection {
        shares: 0.0,
        avg_cost: 0.0,
    };

    for transaction in transactions {
        for (label, value) in [
            ("shares", transaction.shares),
            ("price", transaction.price),
            ("total amount", transaction.total_amount),
            ("commission", transaction.commission),
        ] {
            if !value.is_finite() {
                return Err(format!("Transaction {label} must be finite during replay"));
            }
        }

        match transaction.transaction_type.as_str() {
            "OPEN" => {
                projection.shares = transaction.shares;
                projection.avg_cost = transaction.price;
            }
            "BUY" => {
                let new_shares = projection.shares + transaction.shares;
                if new_shares <= 0.0 {
                    return Err("BUY must leave a positive historical position".to_string());
                }
                projection.avg_cost = (projection.shares * projection.avg_cost
                    + transaction.shares * transaction.price
                    + transaction.commission)
                    / new_shares;
                projection.shares = new_shares;
            }
            "SELL" => {
                let remaining = projection.shares - transaction.shares;
                // Allow a few floating-point ulps for sums such as 0.1 + 0.7,
                // not an absolute epsilon that erases real tiny positions.
                // The cap also preserves representable residuals at large sizes.
                let tolerance =
                    (4.0 * f64::EPSILON * projection.shares.abs().max(transaction.shares.abs()))
                        .min(POSITION_EPSILON);
                if remaining < -tolerance {
                    return Err(format!(
                        "SELL of {} exceeds historical position of {}",
                        transaction.shares, projection.shares
                    ));
                }
                let remaining = if remaining.abs() <= tolerance {
                    0.0
                } else {
                    remaining
                };
                if adjust_sell_pay_cost {
                    projection.avg_cost = if remaining > 0.0 {
                        (projection.shares * projection.avg_cost - transaction.total_amount
                            + transaction.commission)
                            / remaining
                    } else {
                        0.0
                    };
                }
                projection.shares = remaining;
            }
            "PAY" if adjust_sell_pay_cost && projection.shares > 0.0 => {
                let net_amount = transaction.total_amount - transaction.commission;
                projection.avg_cost =
                    (projection.shares * projection.avg_cost - net_amount) / projection.shares;
            }
            "PAY" => {}
            other => {
                return Err(format!(
                    "Unsupported transaction type during replay: {other}"
                ))
            }
        }
    }

    Ok(projection)
}

fn load_group_holdings(conn: &Connection, key: &PositionKey) -> Result<Vec<StoredHolding>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, market FROM holdings
             WHERE account_id = ?1 AND UPPER(symbol) = ?2
             ORDER BY created_at, id",
        )
        .map_err(|error| error.to_string())?;
    let holdings = statement
        .query_map(
            rusqlite::params![key.account_id, key.normalized_symbol],
            |row| {
                Ok(StoredHolding {
                    id: row.get(0)?,
                    market: row.get(1)?,
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(holdings)
}

fn load_group_transactions(
    conn: &Connection,
    key: &PositionKey,
) -> Result<Vec<StoredTransaction>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, symbol, name, market, currency, transaction_type,
                    shares, price, total_amount, commission
             FROM transactions
             WHERE account_id = ?1 AND UPPER(symbol) = ?2
             ORDER BY traded_at, created_at, id",
        )
        .map_err(|error| error.to_string())?;
    let transactions = statement
        .query_map(
            rusqlite::params![key.account_id, key.normalized_symbol],
            |row| {
                Ok(StoredTransaction {
                    symbol: row.get(1)?,
                    name: row.get(2)?,
                    market: row.get(3)?,
                    currency: row.get(4)?,
                    replay: ReplayTransaction {
                        transaction_type: row.get(5)?,
                        shares: row.get(6)?,
                        price: row.get(7)?,
                        total_amount: row.get(8)?,
                        commission: row.get(9)?,
                    },
                })
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(transactions)
}

fn apply_group_projection(
    conn: &Connection,
    key: &PositionKey,
    holdings: &[StoredHolding],
    transactions: &[StoredTransaction],
) -> Result<Option<String>, String> {
    let market = transactions
        .first()
        .map(|transaction| transaction.market.as_str())
        .or_else(|| holdings.first().map(|holding| holding.market.as_str()))
        .unwrap_or("US");
    let replay_rows: Vec<ReplayTransaction> = transactions
        .iter()
        .map(|transaction| transaction.replay.clone())
        .collect();
    let projection = replay_transactions(&replay_rows, market_adjusts_sell_pay_cost(conn, market))?;

    let produces_position = transactions
        .iter()
        .any(|transaction| matches!(transaction.replay.transaction_type.as_str(), "OPEN" | "BUY"));
    let now = chrono::Utc::now().to_rfc3339();
    let primary_id = if let Some(primary) = holdings.first() {
        conn.execute(
            "UPDATE holdings SET shares = ?2, avg_cost = ?3, updated_at = ?4 WHERE id = ?1",
            rusqlite::params![primary.id, projection.shares, projection.avg_cost, now],
        )
        .map_err(|error| error.to_string())?;
        Some(primary.id.clone())
    } else if produces_position {
        let source = transactions
            .last()
            .ok_or_else(|| "Position-producing history is empty".to_string())?;
        let holding_id = uuid::Uuid::new_v4().to_string();
        conn.execute(
            "INSERT INTO holdings
               (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                currency, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL, ?6, ?7, ?8, ?9, ?9)",
            rusqlite::params![
                holding_id,
                key.account_id,
                source.symbol,
                source.name,
                source.market,
                projection.shares,
                projection.avg_cost,
                source.currency,
                now
            ],
        )
        .map_err(|error| error.to_string())?;
        Some(holding_id)
    } else {
        None
    };

    if let Some(primary_id) = primary_id.as_deref() {
        conn.execute(
            "UPDATE transactions SET holding_id = ?1
             WHERE account_id = ?2 AND UPPER(symbol) = ?3",
            rusqlite::params![primary_id, key.account_id, key.normalized_symbol],
        )
        .map_err(|error| error.to_string())?;

        for duplicate in holdings.iter().skip(1) {
            conn.execute(
                "DELETE FROM holdings WHERE id = ?1",
                rusqlite::params![duplicate.id],
            )
            .map_err(|error| error.to_string())?;
        }
    } else if !transactions.is_empty() {
        conn.execute(
            "UPDATE transactions SET holding_id = NULL
             WHERE account_id = ?1 AND UPPER(symbol) = ?2",
            rusqlite::params![key.account_id, key.normalized_symbol],
        )
        .map_err(|error| error.to_string())?;
    }

    Ok(primary_id)
}

pub(crate) fn rebuild_position_group(
    conn: &Connection,
    key: &PositionKey,
) -> Result<Option<String>, String> {
    let holdings = load_group_holdings(conn, key)?;
    let transactions = load_group_transactions(conn, key)?;
    apply_group_projection(conn, key, &holdings, &transactions)
}

pub(crate) fn rebuild_all_position_groups(conn: &Connection) -> Result<(), String> {
    let mut holdings_by_key: HashMap<PositionKey, Vec<StoredHolding>> = HashMap::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT account_id, symbol, id, market FROM holdings
                 WHERE symbol NOT LIKE '$CASH-%'
                 ORDER BY account_id, UPPER(symbol), created_at, id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                let account_id: String = row.get(0)?;
                let symbol: String = row.get(1)?;
                Ok((
                    PositionKey::new(&account_id, &symbol),
                    StoredHolding {
                        id: row.get(2)?,
                        market: row.get(3)?,
                    },
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (key, holding) = row.map_err(|error| error.to_string())?;
            holdings_by_key.entry(key).or_default().push(holding);
        }
    }

    let mut transactions_by_key: HashMap<PositionKey, Vec<StoredTransaction>> = HashMap::new();
    {
        let mut statement = conn
            .prepare(
                "SELECT account_id, symbol, id, name, market, currency, transaction_type,
                        shares, price, total_amount, commission
                 FROM transactions
                 WHERE symbol NOT LIKE '$CASH-%'
                 ORDER BY account_id, UPPER(symbol), traded_at, created_at, id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                let account_id: String = row.get(0)?;
                let symbol: String = row.get(1)?;
                Ok((
                    PositionKey::new(&account_id, &symbol),
                    StoredTransaction {
                        symbol,
                        name: row.get(3)?,
                        market: row.get(4)?,
                        currency: row.get(5)?,
                        replay: ReplayTransaction {
                            transaction_type: row.get(6)?,
                            shares: row.get(7)?,
                            price: row.get(8)?,
                            total_amount: row.get(9)?,
                            commission: row.get(10)?,
                        },
                    },
                ))
            })
            .map_err(|error| error.to_string())?;
        for row in rows {
            let (key, transaction) = row.map_err(|error| error.to_string())?;
            transactions_by_key
                .entry(key)
                .or_default()
                .push(transaction);
        }
    }

    let keys: HashSet<PositionKey> = holdings_by_key
        .keys()
        .chain(transactions_by_key.keys())
        .cloned()
        .collect();
    for key in keys {
        let holdings = holdings_by_key.remove(&key).unwrap_or_default();
        let transactions = transactions_by_key.remove(&key).unwrap_or_default();
        apply_group_projection(conn, &key, &holdings, &transactions)?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        rebuild_all_position_groups, rebuild_position_group, replay_transactions, PositionKey,
        ReplayTransaction,
    };
    use crate::db::Database;

    fn transaction(
        transaction_type: &str,
        shares: f64,
        price: f64,
        total_amount: f64,
        commission: f64,
    ) -> ReplayTransaction {
        ReplayTransaction {
            transaction_type: transaction_type.to_string(),
            shares,
            price,
            total_amount,
            commission,
        }
    }

    fn database_with_account() -> Database {
        let database = Database::new(":memory:").unwrap();
        {
            let connection = database.conn.lock().unwrap();
            connection
                .execute(
                    "INSERT INTO accounts (id, name, market, created_at, updated_at)
                     VALUES ('account-1', 'Primary', 'US', '2026-01-01', '2026-01-01')",
                    [],
                )
                .unwrap();
        }
        database
    }

    #[test]
    fn replay_rejects_a_historical_sell_larger_than_the_position() {
        let transactions = vec![
            transaction("BUY", 10.0, 20.0, 200.0, 1.0),
            transaction("SELL", 11.0, 25.0, 275.0, 1.0),
        ];

        let error = replay_transactions(&transactions, false).unwrap_err();

        assert!(error.contains("historical position"), "got: {error}");
    }

    #[test]
    fn replay_preserves_real_tiny_positions_and_rejects_real_oversells() {
        let position = replay_transactions(
            &[
                transaction("BUY", 1e-10, 10.0, 1e-9, 0.0),
                transaction("SELL", 5e-11, 10.0, 5e-10, 0.0),
            ],
            false,
        )
        .unwrap();
        assert_eq!(position.shares, 5e-11);

        for (held, sold) in [(0.8, 0.8000000001), (1e-10, 2e-10)] {
            assert!(
                replay_transactions(
                    &[
                        transaction("BUY", held, 10.0, held * 10.0, 0.0),
                        transaction("SELL", sold, 10.0, sold * 10.0, 0.0),
                    ],
                    false
                )
                .is_err(),
                "accepted oversell of {sold} with {held} held"
            );
        }
    }

    #[test]
    fn replay_uses_chronological_cost_formulas() {
        let transactions = vec![
            transaction("BUY", 10.0, 20.0, 200.0, 2.0),
            transaction("BUY", 5.0, 30.0, 150.0, 1.0),
            transaction("SELL", 3.0, 40.0, 120.0, 2.0),
        ];

        let fixed_cost = replay_transactions(&transactions, false).unwrap();
        assert_eq!(fixed_cost.shares, 12.0);
        assert!((fixed_cost.avg_cost - (353.0 / 15.0)).abs() < 1e-9);

        let adjusted_cost = replay_transactions(&transactions, true).unwrap();
        assert_eq!(adjusted_cost.shares, 12.0);
        assert!((adjusted_cost.avg_cost - (235.0 / 12.0)).abs() < 1e-9);
    }

    #[test]
    fn replay_applies_pay_only_when_cost_adjustment_is_enabled() {
        let transactions = vec![
            transaction("OPEN", 10.0, 20.0, 200.0, 0.0),
            transaction("PAY", 0.0, 0.0, 30.0, 2.0),
        ];

        let fixed_cost = replay_transactions(&transactions, false).unwrap();
        assert_eq!(fixed_cost.avg_cost, 20.0);

        let adjusted_cost = replay_transactions(&transactions, true).unwrap();
        assert_eq!(adjusted_cost.avg_cost, 17.2);
    }

    #[test]
    fn targeted_rebuild_replaces_stale_holding_with_chronological_projection() {
        let database = database_with_account();
        let connection = database.conn.lock().unwrap();
        connection
            .execute_batch(
                "INSERT INTO holdings
                   (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                    currency, created_at, updated_at)
                 VALUES
                   ('holding-1', 'account-1', 'AAPL', 'Apple', 'US', NULL, 999, 999,
                    'USD', '2026-01-01', '2026-01-01');
                 INSERT INTO transactions
                   (id, holding_id, account_id, symbol, name, market, transaction_type,
                    shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                 VALUES
                   ('buy-1', 'holding-1', 'account-1', 'AAPL', 'Apple', 'US', 'BUY',
                    10, 20, 200, 2, 'USD', '2026-01-02', NULL, '2026-01-02'),
                   ('sell-1', 'holding-1', 'account-1', 'AAPL', 'Apple', 'US', 'SELL',
                    4, 30, 120, 1, 'USD', '2026-01-03', NULL, '2026-01-03');",
            )
            .unwrap();

        let holding_id =
            rebuild_position_group(&connection, &PositionKey::new("account-1", "aapl")).unwrap();

        assert_eq!(holding_id.as_deref(), Some("holding-1"));
        let projection: (f64, f64) = connection
            .query_row(
                "SELECT shares, avg_cost FROM holdings WHERE id = 'holding-1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(projection.0, 6.0);
        assert_eq!(projection.1, 20.2);
    }

    #[test]
    fn bulk_rebuild_projects_each_position_and_relinks_duplicate_holdings() {
        let database = database_with_account();
        let connection = database.conn.lock().unwrap();
        connection
            .execute_batch(
                "INSERT INTO holdings
                   (id, account_id, symbol, name, market, category_id, shares, avg_cost,
                    currency, created_at, updated_at)
                 VALUES
                   ('holding-a', 'account-1', 'AAPL', 'Apple', 'US', NULL, 1, 1,
                    'USD', '2026-01-01', '2026-01-01'),
                   ('holding-a-duplicate', 'account-1', 'aapl', 'Apple', 'US', NULL, 2, 2,
                    'USD', '2026-01-02', '2026-01-02'),
                   ('holding-m', 'account-1', 'MSFT', 'Microsoft', 'US', NULL, 3, 3,
                    'USD', '2026-01-01', '2026-01-01');
                 INSERT INTO transactions
                   (id, holding_id, account_id, symbol, name, market, transaction_type,
                    shares, price, total_amount, commission, currency, traded_at, notes, created_at)
                 VALUES
                   ('buy-a', 'holding-a-duplicate', 'account-1', 'AAPL', 'Apple', 'US', 'BUY',
                    5, 10, 50, 0, 'USD', '2026-01-03', NULL, '2026-01-03'),
                   ('buy-m', 'holding-m', 'account-1', 'MSFT', 'Microsoft', 'US', 'BUY',
                    7, 20, 140, 0, 'USD', '2026-01-03', NULL, '2026-01-03');",
            )
            .unwrap();

        rebuild_all_position_groups(&connection).unwrap();

        let holdings: Vec<(String, f64)> = {
            let mut statement = connection
                .prepare(
                    "SELECT UPPER(symbol), shares FROM holdings
                     WHERE symbol NOT LIKE '$CASH-%' ORDER BY UPPER(symbol)",
                )
                .unwrap();
            statement
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
                .unwrap()
                .collect::<Result<_, _>>()
                .unwrap()
        };
        assert_eq!(
            holdings,
            vec![("AAPL".to_string(), 5.0), ("MSFT".to_string(), 7.0)]
        );

        let linked_holding: String = connection
            .query_row(
                "SELECT holding_id FROM transactions WHERE id = 'buy-a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(linked_holding, "holding-a");
    }
}
