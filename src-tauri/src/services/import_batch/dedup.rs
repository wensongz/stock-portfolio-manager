use super::*;
use crate::services::portfolio_mutation::{
    validate_holding_values, validate_transaction_values, CreateHoldingInput,
    CreateTransactionInput,
};
use chrono::{DateTime, NaiveDate, NaiveDateTime};
use serde_json::{json, Value};

pub(super) fn normalize(kind: &str, account: &str, value: &Value) -> Result<Value, String> {
    let mut data = value.clone();
    let object = data.as_object_mut().ok_or("导入行必须为对象")?;
    object.insert("account_id".into(), json!(account));
    for field in ["symbol", "market", "currency", "transaction_type"] {
        if let Some(value) = object.get(field).and_then(Value::as_str) {
            object.insert(field.into(), json!(value.trim().to_uppercase()));
        }
    }
    if object
        .get("symbol")
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err("证券代码不能为空".into());
    }
    if kind == "holdings" {
        let input: CreateHoldingInput =
            serde_json::from_value(data.clone()).map_err(|e| e.to_string())?;
        validate_holding_values(
            &input.market,
            &input.symbol,
            input.shares,
            input.avg_cost,
            &input.currency,
        )?;
    } else {
        let mut input: CreateTransactionInput =
            serde_json::from_value(data.clone()).map_err(|e| e.to_string())?;
        input.traded_at = canonical_date(&input.traded_at)?;
        validate_transaction_values(&input)?;
        data = serde_json::to_value(input).map_err(|e| e.to_string())?;
    }
    Ok(data)
}
fn canonical_date(value: &str) -> Result<String, String> {
    if let Ok(date) = DateTime::parse_from_rfc3339(value) {
        return Ok(date.with_timezone(&chrono::Utc).to_rfc3339());
    }
    if let Ok(date) = NaiveDate::parse_from_str(value, "%Y-%m-%d") {
        return Ok(date.and_hms_opt(0, 0, 0).unwrap().and_utc().to_rfc3339());
    }
    for fmt in ["%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"] {
        if let Ok(date) = NaiveDateTime::parse_from_str(value, fmt) {
            return Ok(date.and_utc().to_rfc3339());
        }
    }
    Err(format!("无效交易日期：{value}"))
}
pub(super) fn fingerprint(kind: &str, data: &Value) -> String {
    let fields = if kind == "holdings" {
        vec!["symbol", "market", "currency", "shares", "avg_cost"]
    } else {
        vec![
            "symbol",
            "market",
            "currency",
            "transaction_type",
            "traded_at",
            "shares",
            "price",
            "total_amount",
            "commission",
        ]
    };
    let values: Vec<Value> = fields
        .into_iter()
        .map(|k| {
            let v = &data[k];
            if k == "traded_at" {
                json!(v
                    .as_str()
                    .and_then(|s| canonical_date(s).ok())
                    .unwrap_or_default())
            } else if let Some(n) = v.as_f64() {
                json!(if n == 0.0 { 0.0 } else { n })
            } else if let Some(s) = v.as_str() {
                json!(s.trim().to_uppercase())
            } else {
                v.clone()
            }
        })
        .collect();
    serde_json::to_string(&values).expect("finite normalized fields")
}

pub(super) fn classify(
    conn: &rusqlite::Connection,
    req: &ImportBatchRequest,
    key: &str,
    external: Option<&str>,
    data: &Value,
    exclude_batch: &str,
) -> Result<(String, Option<String>), String> {
    let fp = fingerprint(&req.kind, data);
    let mut stmt=conn.prepare("SELECT b.source,(b.source_content=?4),r.row_key,r.external_id,r.fingerprint,r.record_id FROM import_batch_rows r JOIN import_batches b ON b.id=r.batch_id WHERE b.account_id=?1 AND b.kind=?2 AND b.status!='undone' AND r.status='imported' AND b.id!=?3").map_err(|e|e.to_string())?;
    let prior = stmt
        .query_map(
            rusqlite::params![req.account_id, req.kind, exclude_batch, req.source_content],
            |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, bool>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, Option<String>>(3)?,
                    r.get::<_, String>(4)?,
                    r.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    let mut suspected = false;
    for (source, content, old_key, old_external, old_fp, _) in &prior {
        if source == &req.source && external.is_some() && external == old_external.as_deref() {
            return Ok(if old_fp == &fp {
                ("duplicate".into(), Some("成交编号已导入".into()))
            } else {
                (
                    "failed".into(),
                    Some("成交编号相同但内容冲突，请核查原始记录".into()),
                )
            });
        }
        if source == &req.source && !req.source_content.is_empty() && *content && old_key == key {
            return Ok(if old_fp == &fp {
                ("duplicate".into(), Some("同一文件行已导入".into()))
            } else {
                (
                    "failed".into(),
                    Some("同一文件行曾以不同内容导入，请修改原记录而非重复导入".into()),
                )
            });
        }
        if old_fp == &fp
            && !(source == &req.source
                && external.is_some()
                && old_external.is_some()
                && external != old_external.as_deref())
        {
            suspected = true;
        }
    }
    // Check legacy/manual records too. Records with distinct execution IDs from
    // this broker are known legitimate executions and are excluded here.
    let state = state::capture(conn, &req.account_id)?;
    let existing = if req.kind == "holdings" {
        &state.holdings
    } else {
        &state.transactions
    };
    for row in existing {
        if fingerprint(&req.kind, row) != fp {
            continue;
        }
        let known_distinct = external.is_some()
            && prior.iter().any(|(source, _, _, id, _, record)| {
                source == &req.source
                    && id.is_some()
                    && id.as_deref() != external
                    && record.as_deref() == row["id"].as_str()
            });
        if !known_distinct {
            suspected = true;
        }
    }
    Ok(if suspected {
        (
            "suspected".into(),
            Some("存在相同业务字段的记录，请确认是否为另一笔合法成交".into()),
        )
    } else {
        ("ready".into(), None)
    })
}
