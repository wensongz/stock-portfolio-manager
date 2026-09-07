use crate::db::Database;
use crate::models::Holding;
use crate::services::portfolio_mutation::{validate_holding_values, CreateHoldingInput};
use crate::services::position_replay::{rebuild_position_group, PositionKey};
use crate::services::quote_service::is_cash_symbol;
use rusqlite::{Connection, OptionalExtension};

fn load_holding(conn: &Connection, id: &str) -> Result<Holding, String> {
    conn.query_row(
        "SELECT id,account_id,symbol,name,market,category_id,shares,avg_cost,currency,created_at,updated_at
         FROM holdings WHERE id=?1", [id], |r| Ok(Holding {
            id: r.get(0)?, account_id: r.get(1)?, symbol: r.get(2)?, name: r.get(3)?,
            market: r.get(4)?, category_id: r.get(5)?, shares: r.get(6)?, avg_cost: r.get(7)?,
            currency: r.get(8)?, created_at: r.get(9)?, updated_at: r.get(10)?,
        }),
    ).map_err(|error| format!("Holding not found: {error}"))
}

/// Correct an opening balance, or edit metadata without rewriting traded history.
/// The opening and its holding commit together so replay cannot undo a successful edit.
pub fn update_holding(
    db: &Database,
    id: String,
    input: CreateHoldingInput,
) -> Result<Holding, String> {
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|error| error.to_string())?;
    let old = load_holding(&tx, &id)?;
    let identity_changed = old.account_id != input.account_id
        || old.symbol != input.symbol
        || old.market != input.market
        || old.currency != input.currency;
    let values_changed = old.shares != input.shares || old.avg_cost != input.avg_cost;
    let cash = is_cash_symbol(&old.symbol);
    if (cash || is_cash_symbol(&input.symbol)) && identity_changed {
        return Err("现金持仓的账户、代码、市场和币种不可直接修改，股票与现金不可互转".into());
    }
    let now = chrono::Utc::now().to_rfc3339();
    if identity_changed || values_changed {
        validate_holding_values(
            &input.market,
            &input.symbol,
            input.shares,
            input.avg_cost,
            &input.currency,
        )?;
        let total = input.shares * input.avg_cost;
        if !total.is_finite() {
            return Err("期初持仓总成本必须是有限数值".into());
        }
        if cash && input.avg_cost != 1.0 {
            return Err("现金持仓的成本必须为 1".into());
        }
        // Include unlinked records and stock cash movements for cash balances.
        let history: Vec<(String, String)> = tx.prepare(
            "SELECT id,transaction_type FROM transactions
             WHERE holding_id=?1 OR (account_id=?2 AND UPPER(symbol)=UPPER(?3))
                OR (?5 AND account_id=?2 AND currency=?4 AND transaction_type IN ('BUY','SELL','PAY'))"
        ).map_err(|e| e.to_string())?.query_map(
            rusqlite::params![id, old.account_id, old.symbol, old.currency, cash],
            |r| Ok((r.get(0)?, r.get(1)?)),
        ).map_err(|e| e.to_string())?.collect::<Result<_, _>>().map_err(|e| e.to_string())?;
        if history.len() > 1 || history.first().is_some_and(|(_, kind)| kind != "OPEN") {
            return Err(if cash {
                "该现金持仓已有资金流水，请在「交易记录」中记录存入或提取；此处仅可修改名称和类别"
            } else {
                "该持仓已有交易记录，不能直接覆盖股数、成本或所属证券；买卖、分红请在「交易记录」中修正，此处仅可修改名称和类别"
            }.into());
        }
        if identity_changed {
            let opening_id = history.first().map(|(id, _)| id.as_str()).unwrap_or("");
            let collision: bool = tx.query_row(
                "SELECT EXISTS(SELECT 1 FROM holdings WHERE id<>?1 AND account_id=?2 AND UPPER(symbol)=UPPER(?3))
                     OR EXISTS(SELECT 1 FROM transactions WHERE id<>?4 AND account_id=?2 AND UPPER(symbol)=UPPER(?3))",
                rusqlite::params![id, input.account_id, input.symbol, opening_id], |r| r.get(0),
            ).map_err(|e| e.to_string())?;
            if collision {
                return Err("目标账户中已存在该证券的持仓或交易记录，不能直接合并".into());
            }
        }
        if let Some((opening_id, _)) = history.first() {
            tx.execute(
                "UPDATE transactions SET holding_id=?2,account_id=?3,symbol=?4,name=?5,market=?6,
                    shares=?7,price=?8,total_amount=?9,currency=?10 WHERE id=?1",
                rusqlite::params![
                    opening_id,
                    id,
                    input.account_id,
                    input.symbol,
                    input.name,
                    input.market,
                    input.shares,
                    input.avg_cost,
                    total,
                    input.currency
                ],
            )
            .map_err(|e| e.to_string())?;
        } else {
            tx.execute(
                "INSERT INTO transactions (id,holding_id,account_id,symbol,name,market,transaction_type,
                     shares,price,total_amount,commission,currency,traded_at,notes,created_at)
                 VALUES (?1,?2,?3,?4,?5,?6,'OPEN',?7,?8,?9,0,?10,?11,'持仓编辑：补记期初余额',?12)",
                rusqlite::params![uuid::Uuid::new_v4().to_string(),id,input.account_id,input.symbol,input.name,
                    input.market,input.shares,input.avg_cost,total,input.currency,old.created_at,now],
            ).map_err(|e| e.to_string())?;
        }
        // These snapshots contain the previous opening balance or its old identity.
        let opening_date: Option<String> = tx.query_row(
            "SELECT DATE(traded_at) FROM transactions WHERE holding_id=?1 AND transaction_type='OPEN'",
            [&id], |r| r.get::<_, Option<String>>(0),
        ).optional().map_err(|e| e.to_string())?.flatten();
        let from = opening_date.as_deref().unwrap_or("0000-01-01");
        super::snapshot_cache_service::invalidate_from(&tx, from)?;
    }
    tx.execute(
        "UPDATE holdings SET account_id=?2,symbol=?3,name=?4,market=?5,category_id=?6,
             shares=?7,avg_cost=?8,currency=?9,updated_at=?10 WHERE id=?1",
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
            now
        ],
    )
    .map_err(|e| e.to_string())?;
    if (identity_changed || values_changed) && !cash {
        rebuild_position_group(&tx, &PositionKey::new(&input.account_id, &input.symbol))?;
    }
    let result = load_holding(&tx, &id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}

#[cfg(test)]
#[path = "holding_edit_tests.rs"]
mod tests;
