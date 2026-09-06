use crate::models::import_batch::{ExpectedBalance, ReconciliationRow};
use rusqlite::{types::ValueRef, Connection};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct AccountState {
    pub holdings: Vec<Value>,
    pub transactions: Vec<Value>,
}
fn table_rows(conn: &Connection, table: &str, account: &str) -> Result<Vec<Value>, String> {
    let mut stmt = conn
        .prepare(&format!(
            "SELECT * FROM {table} WHERE account_id=?1 ORDER BY id"
        ))
        .map_err(|e| e.to_string())?;
    let columns: Vec<String> = stmt.column_names().iter().map(|s| s.to_string()).collect();
    let rows = stmt
        .query_map([account], |row| {
            let mut object = Map::new();
            for (i, name) in columns.iter().enumerate() {
                let value = match row.get_ref(i)? {
                    ValueRef::Null => Value::Null,
                    ValueRef::Integer(n) => Value::from(n),
                    ValueRef::Real(n) if n.is_finite() => Value::from(n),
                    ValueRef::Real(_) => {
                        return Err(rusqlite::Error::FromSqlConversionFailure(
                            i,
                            rusqlite::types::Type::Real,
                            Box::new(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "账户数值溢出或非有限，不能保存导入审计状态",
                            )),
                        ))
                    }
                    ValueRef::Text(s) => Value::String(String::from_utf8_lossy(s).into_owned()),
                    ValueRef::Blob(_) => return Err(rusqlite::Error::InvalidQuery),
                };
                object.insert(name.clone(), value);
            }
            Ok(Value::Object(object))
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}
pub(super) fn capture(conn: &Connection, account: &str) -> Result<AccountState, String> {
    Ok(AccountState {
        holdings: table_rows(conn, "holdings", account)?,
        transactions: table_rows(conn, "transactions", account)?,
    })
}
pub(super) fn restore(
    conn: &Connection,
    account: &str,
    state: &AccountState,
) -> Result<(), String> {
    // Only transactions reference holdings in the schema. Both tables belong to
    // the guarded account; all original IDs and timestamps are restored exactly.
    conn.execute("DELETE FROM transactions WHERE account_id=?1", [account])
        .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM holdings WHERE account_id=?1", [account])
        .map_err(|e| e.to_string())?;
    for (table, rows) in [
        ("holdings", &state.holdings),
        ("transactions", &state.transactions),
    ] {
        for row in rows {
            let object = row.as_object().ok_or("Invalid saved account state")?;
            let columns: Vec<_> = object.keys().map(|s| format!("\"{s}\"")).collect();
            let placeholders = vec!["?"; columns.len()].join(",");
            let values: Vec<rusqlite::types::Value> = object
                .values()
                .map(|v| match v {
                    Value::Null => rusqlite::types::Value::Null,
                    Value::Number(n) if n.is_i64() => {
                        rusqlite::types::Value::Integer(n.as_i64().unwrap())
                    }
                    Value::Number(n) => rusqlite::types::Value::Real(n.as_f64().unwrap()),
                    Value::String(s) => rusqlite::types::Value::Text(s.clone()),
                    _ => rusqlite::types::Value::Null,
                })
                .collect();
            conn.execute(
                &format!(
                    "INSERT INTO {table} ({}) VALUES ({placeholders})",
                    columns.join(",")
                ),
                rusqlite::params_from_iter(values),
            )
            .map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
pub(super) fn reconciliation(
    before: &AccountState,
    after: &AccountState,
    expected: &[ExpectedBalance],
) -> Vec<ReconciliationRow> {
    let mut symbols: BTreeSet<String> = before
        .holdings
        .iter()
        .chain(&after.holdings)
        .filter_map(|r| r["symbol"].as_str().map(String::from))
        .collect();
    symbols.extend(expected.iter().map(|e| e.symbol.clone()));
    symbols
        .into_iter()
        .map(|symbol| {
            let amount = |state: &AccountState| {
                state
                    .holdings
                    .iter()
                    .filter(|r| r["symbol"].as_str() == Some(&symbol))
                    .map(|r| r["shares"].as_f64().unwrap_or(0.0))
                    .sum::<f64>()
            };
            let before_shares = amount(before);
            let after_shares = amount(after);
            let expected_shares = expected
                .iter()
                .find(|e| e.symbol == symbol)
                .map(|e| e.expected_shares);
            let currency = after
                .holdings
                .iter()
                .chain(&before.holdings)
                .find(|r| r["symbol"].as_str() == Some(&symbol))
                .and_then(|r| r["currency"].as_str())
                .unwrap_or("")
                .to_string();
            ReconciliationRow {
                symbol,
                currency,
                before_shares,
                after_shares,
                expected_shares,
                difference: expected_shares.map(|e| after_shares - e),
            }
        })
        .collect()
}
pub(super) fn invalidate_daily(
    conn: &Connection,
    before: &AccountState,
    after: &AccountState,
) -> Result<(), String> {
    let changed = before
        .transactions
        .iter()
        .filter(|row| !after.transactions.contains(row))
        .chain(
            after
                .transactions
                .iter()
                .filter(|row| !before.transactions.contains(row)),
        );
    let start = changed
        .filter_map(|r| r["traded_at"].as_str())
        .filter_map(|s| s.get(..10))
        .min()
        .unwrap_or("0000-01-01");
    conn.execute(
        "DELETE FROM daily_holding_snapshots WHERE date>=?1",
        [start],
    )
    .map_err(|e| e.to_string())?;
    conn.execute("DELETE FROM daily_portfolio_values WHERE date>=?1", [start])
        .map_err(|e| e.to_string())?;
    Ok(())
}
