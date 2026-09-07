use super::WorkingHolding;
use crate::services::portfolio_mutation::cash_delta;
use crate::services::quote_service::{cash_display_name, is_cash_symbol};
use chrono::NaiveDate;
use rusqlite::Connection;
use std::collections::BTreeMap;

struct CashEvent {
    date: NaiveDate,
    opening: Option<f64>,
    delta: f64,
}

struct CashAccount {
    holding: WorkingHolding,
    current_balance: Option<f64>,
    holding_date: Option<NaiveDate>,
    events: Vec<CashEvent>,
}

fn parse_date(value: &str) -> Result<NaiveDate, String> {
    NaiveDate::parse_from_str(value, "%Y-%m-%d")
        .ok()
        .ok_or_else(|| format!("invalid historical cash date: {value}"))
}

/// Cash mutations operate by account and currency, even when the account's
/// market differs from its cash currency. Keep that same ledger identity here.
pub(super) fn load_cash_holdings(
    conn: &Connection,
    cutoff: NaiveDate,
) -> Result<Vec<WorkingHolding>, String> {
    let mut accounts: BTreeMap<(String, String), CashAccount> = BTreeMap::new();
    let mut stmt = conn
        .prepare(
            "SELECT h.account_id, COALESCE(a.name, ''), h.symbol, h.name, h.market,
                COALESCE(c.name, '现金类'), COALESCE(c.color, '#22C55E'),
                h.currency, h.shares, DATE(h.created_at)
         FROM holdings h
         LEFT JOIN accounts a ON a.id = h.account_id
         LEFT JOIN categories c ON c.id = h.category_id
         WHERE UPPER(h.symbol) LIKE '$CASH-%'",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                WorkingHolding {
                    account_id: row.get(0)?,
                    account_name: row.get(1)?,
                    symbol: row.get(2)?,
                    name: row.get(3)?,
                    market: row.get(4)?,
                    category_name: row.get(5)?,
                    category_color: row.get(6)?,
                    currency: row.get(7)?,
                    shares: row.get(8)?,
                    avg_cost: 1.0,
                    notes: None,
                    decision_quality: None,
                },
                row.get::<_, String>(9)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (holding, created_at) = row.map_err(|error| error.to_string())?;
        let key = (holding.account_id.clone(), holding.currency.clone());
        if !holding.shares.is_finite() {
            return Err(format!(
                "non-finite historical cash balance for {}/{}",
                key.0, key.1
            ));
        }
        let account = CashAccount {
            current_balance: Some(holding.shares),
            holding_date: Some(parse_date(&created_at)?),
            holding,
            events: Vec::new(),
        };
        if accounts.insert(key.clone(), account).is_some() {
            return Err(format!(
                "ambiguous historical cash baseline for {}/{}",
                key.0, key.1
            ));
        }
    }

    let mut stmt = conn
        .prepare(
            "SELECT t.account_id, COALESCE(a.name, ''), t.symbol, t.market, t.currency,
                t.transaction_type, t.shares, t.total_amount, t.commission, DATE(t.traded_at)
         FROM transactions t LEFT JOIN accounts a ON a.id = t.account_id
         ORDER BY JULIANDAY(t.traded_at), JULIANDAY(t.created_at), t.id",
        )
        .map_err(|error| error.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, f64>(7)?,
                row.get::<_, f64>(8)?,
                row.get::<_, String>(9)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    for row in rows {
        let (
            account_id,
            account_name,
            symbol,
            market,
            currency,
            kind,
            shares,
            amount,
            fee,
            traded_at,
        ) = row.map_err(|error| error.to_string())?;
        if [shares, amount, fee].iter().any(|value| !value.is_finite()) {
            return Err(format!(
                "non-finite historical cash transaction for {account_id}/{currency}"
            ));
        }
        let cash_symbol = format!("$CASH-{currency}");
        let account = accounts
            .entry((account_id.clone(), currency.clone()))
            .or_insert_with(|| CashAccount {
                holding: WorkingHolding {
                    account_id,
                    account_name,
                    name: cash_display_name(&cash_symbol),
                    symbol: cash_symbol,
                    market,
                    currency,
                    category_name: "现金类".into(),
                    category_color: "#22C55E".into(),
                    shares: 0.0,
                    avg_cost: 1.0,
                    notes: None,
                    decision_quality: None,
                },
                current_balance: None,
                holding_date: None,
                events: Vec::new(),
            });
        account.events.push(CashEvent {
            date: parse_date(&traded_at)?,
            opening: (is_cash_symbol(&symbol) && kind == "OPEN").then_some(shares),
            delta: cash_delta(&kind, &symbol, amount, fee),
        });
    }

    let mut holdings = Vec::new();
    for ((account_id, currency), mut account) in accounts {
        let opening = account
            .events
            .iter()
            .enumerate()
            .rfind(|(_, event)| event.date <= cutoff && event.opening.is_some());
        let missing = || {
            format!("missing historical cash baseline for {account_id}/{currency} on or before {cutoff}")
        };
        let balance = if let Some((index, event)) = opening {
            event.opening.unwrap()
                + account.events[index + 1..]
                    .iter()
                    .filter(|event| event.date <= cutoff)
                    .map(|event| event.delta)
                    .sum::<f64>()
        } else if account.events.iter().any(|event| event.opening.is_some()) {
            // A later OPEN resets cash. It cannot establish the balance before
            // its date, even when a current cash row is available.
            if account
                .events
                .iter()
                .any(|event| event.date <= cutoff && event.delta != 0.0)
            {
                return Err(missing());
            }
            continue;
        } else if let Some(current) = account.current_balance {
            let origin = account
                .events
                .first()
                .map(|event| event.date)
                .into_iter()
                .chain(account.holding_date)
                .min();
            if origin.is_none_or(|origin| origin > cutoff) {
                continue;
            }
            // Include every future flow when anchoring at the actual balance;
            // the cash row itself may have been auto-created much later.
            current
                - account
                    .events
                    .iter()
                    .filter(|event| event.date > cutoff)
                    .map(|event| event.delta)
                    .sum::<f64>()
        } else {
            if account
                .events
                .iter()
                .any(|event| event.date <= cutoff && event.delta != 0.0)
            {
                return Err(missing());
            }
            continue;
        };
        if !balance.is_finite() {
            return Err(format!(
                "non-finite historical cash balance for {account_id}/{currency}"
            ));
        }
        account.holding.shares = balance;
        holdings.push(account.holding);
    }
    Ok(holdings)
}
