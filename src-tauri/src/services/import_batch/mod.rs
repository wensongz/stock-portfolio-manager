mod dedup;
mod state;
#[cfg(test)]
mod tests;

use crate::db::Database;
use crate::models::import_batch::*;
use crate::services::portfolio_mutation::{
    create_holding_in, create_transaction_in, CreateHoldingInput, CreateTransactionInput,
};
use rusqlite::{params, Connection, OptionalExtension};
use state::AccountState;
use std::collections::HashSet;

const CONFLICT: &str =
    "账户在此批次提交后已有变更，请先撤销后续批次或核查后续交易；本次未修改账户数据。";
fn json<T: serde::Serialize>(v: &T) -> Result<String, String> {
    serde_json::to_string(v).map_err(|e| e.to_string())
}
fn decode<T: serde::de::DeserializeOwned>(s: &str) -> Result<T, String> {
    serde_json::from_str(s).map_err(|e| e.to_string())
}
fn request_for(conn: &Connection, id: &str) -> Result<ImportBatchRequest, String> {
    let value: String = conn
        .query_row(
            "SELECT request_json FROM import_batches WHERE id=?1",
            [id],
            |r| r.get(0),
        )
        .map_err(|e| e.to_string())?;
    decode(&value)
}
fn states(
    conn: &Connection,
    id: &str,
) -> Result<(Option<AccountState>, Option<AccountState>), String> {
    let (before, after): (Option<String>, Option<String>) = conn
        .query_row(
            "SELECT before_state,after_state FROM import_batches WHERE id=?1",
            [id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| e.to_string())?;
    Ok((
        before.as_deref().map(decode).transpose()?,
        after.as_deref().map(decode).transpose()?,
    ))
}
fn read_batch(conn: &Connection, id: &str) -> Result<ImportBatch, String> {
    let mut batch=conn.query_row("SELECT id,account_id,source,file_name,parser_version,kind,status,created_at FROM import_batches WHERE id=?1",[id],|r|Ok(ImportBatch{id:r.get(0)?,account_id:r.get(1)?,source:r.get(2)?,file_name:r.get(3)?,parser_version:r.get(4)?,kind:r.get(5)?,status:r.get(6)?,created_at:r.get(7)?,rows:vec![],reconciliation:vec![],can_undo:false,conflict:None})).map_err(|e|e.to_string())?;
    let mut stmt=conn.prepare("SELECT row_key,raw,external_id,data,status,error,record_id FROM import_batch_rows WHERE batch_id=?1 ORDER BY ordinal").map_err(|e|e.to_string())?;
    batch.rows = stmt
        .query_map([id], |r| {
            let data: String = r.get(3)?;
            let data = serde_json::from_str(&data).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            Ok(ImportBatchRow {
                key: r.get(0)?,
                raw: serde_json::from_str(&r.get::<_, String>(1)?).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        Box::new(e),
                    )
                })?,
                external_id: r.get(2)?,
                data,
                status: r.get(4)?,
                error: r.get(5)?,
                record_id: r.get(6)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let (before, after) = states(conn, id)?;
    if let (Some(before), Some(after)) = (before, after) {
        let expected: String = conn
            .query_row(
                "SELECT expected_balances FROM import_batches WHERE id=?1",
                [id],
                |r| r.get(0),
            )
            .map_err(|e| e.to_string())?;
        batch.reconciliation =
            state::reconciliation(&before, &after, &decode::<Vec<ExpectedBalance>>(&expected)?);
        if batch.status == "applied" {
            let unchanged = state::capture(conn, &batch.account_id)? == after;
            batch.can_undo = unchanged && batch.rows.iter().any(|r| r.status == "imported");
            if !unchanged {
                batch.conflict = Some(CONFLICT.into());
            }
        }
    }
    Ok(batch)
}
pub fn get_import_batch(db: &Database, id: &str) -> Result<ImportBatch, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    read_batch(&conn, id)
}
pub fn list_import_batches(
    db: &Database,
    account_id: Option<&str>,
) -> Result<Vec<ImportBatch>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let ids=conn.prepare("SELECT id FROM import_batches WHERE (?1 IS NULL OR account_id=?1) ORDER BY created_at DESC,id DESC LIMIT 100").map_err(|e|e.to_string())?.query_map([account_id],|r|r.get::<_,String>(0)).map_err(|e|e.to_string())?.collect::<Result<Vec<_>,_>>().map_err(|e|e.to_string())?;
    ids.iter().map(|id| read_batch(&conn, id)).collect()
}
pub fn preview_import_batch(
    db: &Database,
    request: &ImportBatchRequest,
) -> Result<ImportBatch, String> {
    if request.request_id.trim().is_empty()
        || request.source.trim().is_empty()
        || request.parser_version.trim().is_empty()
    {
        return Err("批次请求、来源和解析版本不能为空".into());
    }
    if !["transactions", "holdings"].contains(&request.kind.as_str()) {
        return Err("不支持的批次类型".into());
    }
    if request.rows.is_empty() {
        return Err("没有可导入的记录".into());
    }
    let mut keys = HashSet::new();
    if request
        .rows
        .iter()
        .any(|r| r.key.is_empty() || !keys.insert(&r.key))
    {
        return Err("导入行编号不能为空或重复".into());
    }
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let existing: Option<(String, String)> = tx
        .query_row(
            "SELECT id,request_json FROM import_batches WHERE request_id=?1",
            [&request.request_id],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()
        .map_err(|e| e.to_string())?;
    if let Some((id, original)) = existing {
        if json(request)? != original {
            return Err("同一请求编号不能用于不同的导入内容".into());
        }
        return read_batch(&tx, &id);
    }
    let id = uuid::Uuid::new_v4().to_string();
    tx.execute("INSERT INTO import_batches (id,request_id,account_id,source,file_name,source_content,parser_version,kind,status,created_at,request_json) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'preview',?9,?10)",params![id,request.request_id,request.account_id,request.source,request.file_name,request.source_content,request.parser_version,request.kind,chrono::Utc::now().to_rfc3339(),json(request)?]).map_err(|e|e.to_string())?;
    let mut seen: Vec<(Option<String>, String)> = vec![];
    for (ordinal, row) in request.rows.iter().enumerate() {
        let external = row
            .external_id
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let normalized = dedup::normalize(&request.kind, &request.account_id, &row.data);
        let (data, mut status, mut error) = match normalized {
            Ok(data) => {
                let (s, e) = dedup::classify(&tx, request, &row.key, external, &data, &id)?;
                (data, s, e)
            }
            Err(e) => (row.data.clone(), "failed".into(), Some(e)),
        };
        let fp = dedup::fingerprint(&request.kind, &data);
        if status == "ready" || status == "suspected" {
            for (old_id, old_fp) in &seen {
                if external.is_some() && old_id.as_deref() == external {
                    status = if old_fp == &fp { "duplicate" } else { "failed" }.into();
                    error = Some(
                        if old_fp == &fp {
                            "文件内成交编号重复"
                        } else {
                            "文件内成交编号内容冲突"
                        }
                        .into(),
                    );
                    break;
                }
                if old_fp == &fp
                    && !(external.is_some() && old_id.is_some() && external != old_id.as_deref())
                {
                    status = "suspected".into();
                    error = Some("文件内存在相同业务字段，请确认是否为另一笔合法成交".into());
                }
            }
        }
        if status != "failed" {
            seen.push((external.map(String::from), fp.clone()));
        }
        tx.execute("INSERT INTO import_batch_rows (batch_id,row_key,ordinal,raw,external_id,data,fingerprint,status,error) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![id,row.key,ordinal as i64,json(&row.raw)?,external,json(&data)?,fp,status,error]).map_err(|e|e.to_string())?;
    }
    let batch = read_batch(&tx, &id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(batch)
}
fn write_row(conn: &Connection, kind: &str, data: &serde_json::Value) -> Result<String, String> {
    if kind == "holdings" {
        let input: CreateHoldingInput =
            serde_json::from_value(data.clone()).map_err(|e| e.to_string())?;
        Ok(create_holding_in(conn, &input)?.id)
    } else {
        let input: CreateTransactionInput =
            serde_json::from_value(data.clone()).map_err(|e| e.to_string())?;
        let created = create_transaction_in(conn, &input)?;
        Ok(created.id)
    }
}
pub fn apply_import_batch(
    db: &Database,
    id: &str,
    keys: &[String],
    allow: &[String],
) -> Result<ImportBatch, String> {
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let batch = read_batch(&tx, id)?;
    if batch.status == "undone" {
        return Err("已撤销批次不能再次提交，请创建新批次".into());
    }
    if batch.conflict.is_some() {
        return Err(CONFLICT.into());
    }
    if keys
        .iter()
        .any(|key| !batch.rows.iter().any(|r| &r.key == key))
    {
        return Err("包含未知行编号".into());
    }
    let request = request_for(&tx, id)?;
    let (original, _) = states(&tx, id)?;
    let current = state::capture(&tx, &batch.account_id)?;
    let before = original.unwrap_or_else(|| current.clone());
    let mut rows: Vec<_> = batch
        .rows
        .iter()
        .filter(|r| keys.contains(&r.key) && r.status != "imported" && r.status != "duplicate")
        .collect();
    // Stable sort preserves source order among executions sharing a timestamp.
    rows.sort_by(|a, b| {
        a.data["traded_at"]
            .as_str()
            .unwrap_or("")
            .cmp(b.data["traded_at"].as_str().unwrap_or(""))
    });
    let mut wrote = false;
    for row in rows {
        let outcome = (|| -> Result<(String, Option<String>, Option<String>), String> {
            let data = dedup::normalize(&batch.kind, &batch.account_id, &row.data)?;
            let (status, error) = dedup::classify(
                &tx,
                &request,
                &row.key,
                row.external_id.as_deref(),
                &data,
                "",
            )?;
            if status == "duplicate" || status == "failed" {
                return Ok((status, error, None));
            }
            if (status == "suspected" || row.status == "suspected") && !allow.contains(&row.key) {
                return Ok((
                    "suspected".into(),
                    Some("请明确确认疑似重复记录后再导入".into()),
                    None,
                ));
            }
            let sp = tx.savepoint().map_err(|e| e.to_string())?;
            let record = write_row(&sp, &batch.kind, &data)?;
            // Validate computed values as well as inputs before this row commits.
            state::capture(&sp, &batch.account_id)?;
            sp.commit().map_err(|e| e.to_string())?;
            Ok(("imported".into(), None, Some(record)))
        })();
        let (status, error, record) = outcome.unwrap_or_else(|e| ("failed".into(), Some(e), None));
        wrote |= status == "imported";
        tx.execute("UPDATE import_batch_rows SET status=?3,error=?4,record_id=?5 WHERE batch_id=?1 AND row_key=?2",params![id,row.key,status,error,record]).map_err(|e|e.to_string())?;
    }
    if wrote {
        let after = state::capture(&tx, &batch.account_id)?;
        state::invalidate_daily(&tx, &current, &after)?;
        tx.execute(
            "UPDATE import_batches SET status='applied',before_state=?2,after_state=?3 WHERE id=?1",
            params![id, json(&before)?, json(&after)?],
        )
        .map_err(|e| e.to_string())?;
    }
    let result = read_batch(&tx, id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}
pub fn undo_import_batch(db: &Database, id: &str) -> Result<ImportBatch, String> {
    let mut conn = db.conn.lock().map_err(|e| e.to_string())?;
    let tx = conn
        .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)
        .map_err(|e| e.to_string())?;
    let batch = read_batch(&tx, id)?;
    if batch.status == "undone" {
        return Ok(batch);
    }
    if !batch.can_undo {
        return Err(batch
            .conflict
            .unwrap_or_else(|| "此批次没有可撤销的已导入记录".into()));
    }
    let (before, after) = states(&tx, id)?;
    let before = before.ok_or("缺少撤销前状态")?;
    let after = after.ok_or("缺少撤销后状态")?;
    state::restore(&tx, &batch.account_id, &before)?;
    state::invalidate_daily(&tx, &after, &before)?;
    tx.execute(
        "UPDATE import_batches SET status='undone' WHERE id=?1",
        [id],
    )
    .map_err(|e| e.to_string())?;
    let result = read_batch(&tx, id)?;
    tx.commit().map_err(|e| e.to_string())?;
    Ok(result)
}
pub fn reconcile_import_batch(
    db: &Database,
    id: &str,
    balances: &[ExpectedBalance],
) -> Result<ImportBatch, String> {
    let mut seen = HashSet::new();
    let balances: Vec<_> = balances
        .iter()
        .map(|b| ExpectedBalance {
            symbol: b.symbol.trim().to_uppercase(),
            expected_shares: b.expected_shares,
        })
        .collect();
    if balances.iter().any(|b| {
        b.symbol.is_empty() || !b.expected_shares.is_finite() || !seen.insert(b.symbol.clone())
    }) {
        return Err("对账证券代码须唯一且非空，余额须为有效数值".into());
    }
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let batch = read_batch(&conn, id)?;
    if batch.status != "applied" {
        return Err("仅可核对已提交批次".into());
    }
    conn.execute(
        "UPDATE import_batches SET expected_balances=?2 WHERE id=?1",
        params![id, json(&balances)?],
    )
    .map_err(|e| e.to_string())?;
    read_batch(&conn, id)
}
