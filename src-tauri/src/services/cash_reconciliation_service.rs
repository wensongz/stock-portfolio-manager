use crate::db::Database;
use crate::models::{Holding, Transaction};
use crate::services::portfolio_mutation::cash_delta;
use crate::services::snapshot_cache_service::{current_revision, invalidate_from};
use chrono::NaiveDate;
use rusqlite::Connection;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CashBalanceRow {
    #[serde(flatten)]
    pub transaction: Transaction,
    pub cash_delta: f64,
    pub running_balance: f64,
    #[serde(skip)]
    pub(crate) trade_date: NaiveDate,
}

#[derive(Debug)]
pub(crate) struct CashLedger {
    pub(crate) rows: Vec<CashBalanceRow>,
    pub(crate) recommended_balance: Option<f64>,
    pub(crate) opening_count: usize,
}

impl CashLedger {
    pub(crate) fn balance_at_date(&self, cutoff: NaiveDate) -> Option<f64> {
        self.recommended_balance.map(|_| {
            self.rows
                .iter()
                .rev()
                .find(|row| row.trade_date <= cutoff)
                .map_or(0.0, |row| row.running_balance)
        })
    }
}

#[derive(Debug, Serialize)]
pub struct CashBalanceReconciliation {
    pub holding_id: String,
    pub account_id: String,
    pub currency: String,
    pub current_balance: f64,
    pub recommended_balance: Option<f64>,
    pub difference: Option<f64>,
    pub revision: i64,
    pub opening_count: usize,
    pub rows: Vec<CashBalanceRow>,
}

pub(crate) fn load_cash_ledger(
    conn: &Connection,
    account_id: &str,
    currency: &str,
) -> Result<CashLedger, String> {
    if !matches!(currency, "USD" | "CNY" | "HKD") {
        return Err("不支持的现金币种".into());
    }
    let mut statement = conn
        .prepare(
            "SELECT id,holding_id,account_id,symbol,name,market,transaction_type,shares,price,
                total_amount,commission,currency,traded_at,notes,created_at,DATE(traded_at)
         FROM transactions WHERE account_id=?1 AND currency=?2
           AND ((UPPER(symbol) LIKE '$CASH-%' AND transaction_type IN ('OPEN','BUY','SELL'))
             OR (UPPER(symbol) NOT LIKE '$CASH-%' AND transaction_type IN ('BUY','SELL','PAY')))
         ORDER BY JULIANDAY(traded_at),JULIANDAY(created_at),id",
        )
        .map_err(|error| error.to_string())?;
    let records = statement
        .query_map(rusqlite::params![account_id, currency], |row| {
            Ok((
                Transaction {
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
                },
                row.get::<_, Option<String>>(15)?,
            ))
        })
        .map_err(|error| error.to_string())?;
    let mut rows = Vec::new();
    let mut balance = 0.0;
    let mut opening_count = 0;
    for record in records {
        let (transaction, date) = record.map_err(|error| error.to_string())?;
        let trade_date = date
            .as_deref()
            .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
            .ok_or_else(|| format!("现金流水日期无效：{}", transaction.id))?;
        if [
            transaction.shares,
            transaction.price,
            transaction.total_amount,
            transaction.commission,
        ]
        .iter()
        .any(|value| !value.is_finite())
        {
            return Err(format!("现金流水金额必须是有限数值：{}", transaction.id));
        }
        let symbol = transaction.symbol.to_uppercase();
        if symbol.starts_with("$CASH-") && symbol != format!("$CASH-{currency}") {
            return Err(format!("现金流水代码与币种不一致：{}", transaction.id));
        }
        let delta = if transaction.transaction_type == "OPEN" {
            opening_count += 1;
            transaction.shares - balance
        } else {
            cash_delta(
                &transaction.transaction_type,
                &symbol,
                transaction.total_amount,
                transaction.commission,
            )
        };
        balance = if transaction.transaction_type == "OPEN" {
            transaction.shares
        } else {
            balance + delta
        };
        if !delta.is_finite() || !balance.is_finite() {
            return Err("现金流水累计余额必须是有限数值".into());
        }
        rows.push(CashBalanceRow {
            transaction,
            cash_delta: delta,
            running_balance: balance,
            trade_date,
        });
    }
    Ok(CashLedger {
        recommended_balance: (!rows.is_empty()).then_some(balance),
        rows,
        opening_count,
    })
}

fn load_cash_holding(conn: &Connection, id: &str) -> Result<Holding, String> {
    let holding=conn.query_row(
        "SELECT id,account_id,symbol,name,market,category_id,shares,avg_cost,currency,created_at,updated_at FROM holdings WHERE id=?1",
        [id],|row| Ok(Holding {
            id:row.get(0)?,account_id:row.get(1)?,symbol:row.get(2)?,name:row.get(3)?,market:row.get(4)?,
            category_id:row.get(5)?,shares:row.get(6)?,avg_cost:row.get(7)?,currency:row.get(8)?,created_at:row.get(9)?,updated_at:row.get(10)?,
        }),
    ).map_err(|error| format!("现金持仓不存在：{error}"))?;
    if !matches!(holding.currency.as_str(), "USD" | "CNY" | "HKD")
        || holding.symbol.to_uppercase() != format!("$CASH-{}", holding.currency)
    {
        return Err("仅支持现金持仓，且现金代码必须与币种一致".into());
    }
    if !holding.shares.is_finite() {
        return Err("当前现金余额必须是有限数值".into());
    }
    let count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM holdings WHERE account_id=?1
           AND (UPPER(symbol)=UPPER(?2) OR (UPPER(symbol) LIKE '$CASH-%' AND currency=?3))",
            rusqlite::params![holding.account_id, holding.symbol, holding.currency],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if count != 1 {
        return Err("该账户币种存在重复现金持仓，请先核查".into());
    }
    Ok(holding)
}

pub fn get_cash_balance_reconciliation(
    db: &Database,
    id: &str,
) -> Result<CashBalanceReconciliation, String> {
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    // The mutex coordinates app calls; the read transaction also gives one
    // consistent snapshot when another SQLite connection writes concurrently.
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Deferred)
        .map_err(|error| error.to_string())?;
    let holding = load_cash_holding(&tx, id)?;
    let mut ledger = load_cash_ledger(&tx, &holding.account_id, &holding.currency)?;
    ledger.rows.reverse();
    let difference = ledger
        .recommended_balance
        .map(|recommended| recommended - holding.shares);
    if difference.is_some_and(|value| !value.is_finite()) {
        return Err("现金余额差额必须是有限数值".into());
    }
    let result = CashBalanceReconciliation {
        holding_id: holding.id,
        account_id: holding.account_id,
        currency: holding.currency,
        current_balance: holding.shares,
        recommended_balance: ledger.recommended_balance,
        difference,
        revision: current_revision(&tx)?,
        opening_count: ledger.opening_count,
        rows: ledger.rows,
    };
    tx.commit().map_err(|error| error.to_string())?;
    Ok(result)
}

fn matches_recommended_amount(balance: f64, recommended: f64) -> bool {
    // The editor shows two decimals. Accept that displayed amount without
    // changing the full-precision ledger or inventing a sub-cent opening.
    (balance - recommended).abs() <= 1e-8
        || ((balance * 100.0).round() - (recommended * 100.0).round()).abs() <= 1e-8
}

fn new_opening_date(
    conn: &Connection,
    holding: &Holding,
    ledger: &CashLedger,
) -> Result<String, String> {
    let holding_date: Option<f64> = conn
        .query_row("SELECT JULIANDAY(?1)", [&holding.created_at], |row| {
            row.get(0)
        })
        .map_err(|error| error.to_string())?;
    let Some(first) = ledger.rows.first() else {
        return holding_date
            .map(|_| holding.created_at.clone())
            .ok_or_else(|| "现金持仓期初日期无效".into());
    };
    let (first_date, preceding): (f64, Option<String>) = conn
        .query_row(
            "SELECT JULIANDAY(?1),STRFTIME('%Y-%m-%dT%H:%M:%fZ',?1,'-0.001 seconds')",
            [&first.transaction.traded_at],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map_err(|error| error.to_string())?;
    if holding_date.is_some_and(|date| date < first_date) {
        return Ok(holding.created_at.clone());
    }
    let preceding = preceding.ok_or_else(|| "无法确定现金校正期初日期".to_string())?;
    let before: bool = conn
        .query_row(
            "SELECT JULIANDAY(?1)<JULIANDAY(?2)",
            rusqlite::params![preceding, first.transaction.traded_at],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if !before {
        return Err("现金校正期初日期必须早于首条流水".into());
    }
    Ok(preceding)
}

pub fn correct_cash_balance(
    db: &Database,
    id: &str,
    balance: f64,
    expected_revision: i64,
    name: String,
    category_id: Option<String>,
) -> Result<Holding, String> {
    if !balance.is_finite() {
        return Err("现金余额必须是有限数值".into());
    }
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    if current_revision(&tx)? != expected_revision {
        return Err("账户流水或持仓已变更，请刷新推荐余额后重试".into());
    }
    let holding = load_cash_holding(&tx, id)?;
    let ledger = load_cash_ledger(&tx, &holding.account_id, &holding.currency)?;
    let adopts_recommendation = ledger
        .recommended_balance
        .is_some_and(|recommended| matches_recommended_amount(balance, recommended));
    if !adopts_recommendation && ledger.opening_count > 1 {
        return Err("存在多条现金期初记录，仅可采用推荐余额；自定义前请核查期初".into());
    }
    let saved_balance = if adopts_recommendation {
        ledger.recommended_balance.unwrap()
    } else {
        balance
    };
    // Current cash was the anchor for old daily reconstructions, so a stale
    // balance can affect any cached day. Keep saved quarterly reviews and FX.
    invalidate_from(&tx, "0000-01-01")?;
    let now = chrono::Utc::now().to_rfc3339();
    if !adopts_recommendation {
        let difference = balance - ledger.recommended_balance.unwrap_or(0.0);
        let opening = ledger
            .rows
            .iter()
            .find(|row| row.transaction.transaction_type == "OPEN");
        let opening_balance = opening.map_or(0.0, |row| row.transaction.shares) + difference;
        if !difference.is_finite() || !opening_balance.is_finite() {
            return Err("现金期初差额必须是有限数值".into());
        }
        if let Some(opening) = opening {
            tx.execute(
                "UPDATE transactions SET holding_id=?2,shares=?3,price=1,total_amount=?3,commission=0 WHERE id=?1",
                rusqlite::params![opening.transaction.id,id,opening_balance],
            ).map_err(|error| error.to_string())?;
        } else {
            let date = new_opening_date(&tx, &holding, &ledger)?;
            tx.execute(
                "INSERT INTO transactions(id,holding_id,account_id,symbol,name,market,transaction_type,shares,price,total_amount,commission,currency,traded_at,notes,created_at)
                 VALUES(?1,?2,?3,?4,?5,?6,'OPEN',?7,1,?7,0,?8,?9,'现金余额校正：期初差额（非存入或提取）',?10)",
                rusqlite::params![uuid::Uuid::new_v4().to_string(),id,holding.account_id,holding.symbol,holding.name,holding.market,opening_balance,holding.currency,date,now],
            ).map_err(|error| error.to_string())?;
        }
    }
    tx.execute(
        "UPDATE holdings SET shares=?2,avg_cost=1,name=?3,category_id=?4,updated_at=?5 WHERE id=?1",
        rusqlite::params![id, saved_balance, name, category_id, now],
    )
    .map_err(|error| error.to_string())?;
    let result = load_cash_holding(&tx, id)?;
    tx.commit().map_err(|error| error.to_string())?;
    Ok(result)
}

#[cfg(test)]
#[path = "cash_reconciliation_tests.rs"]
mod tests;
