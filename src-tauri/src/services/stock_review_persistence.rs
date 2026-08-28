#![allow(dead_code)]

use crate::db::Database;
use crate::models::stock_review::{
    StockReviewAnnotation, StockReviewAnnotationInput, StockReviewIssue, StockReviewIssueSeverity,
    StockReviewOverride, StockReviewOverrideInput,
};
use chrono::{DateTime, Duration, SecondsFormat, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

const EPSILON: f64 = 1e-9;

/// The caller's trust boundary for an annotation write. `ai_confirmed` is a
/// provenance label, never standalone authority for an AI to write data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnnotationSaveContext {
    UserInitiated,
    AiAfterExplicitUserConfirmation,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StockReviewAnnotationFilter {
    pub scope_type: Option<String>,
    pub scope_key: Option<String>,
    pub account_id: Option<String>,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OverrideValidationResult {
    pub is_valid: bool,
    pub issues: Vec<StockReviewIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewOverrideList {
    /// Only currently valid corrections. This is the list replay consumers use.
    pub overrides: Vec<StockReviewOverride>,
    /// Persisted rows excluded from replay because their confirmation no longer
    /// matches the referenced source ledger.
    pub stale_overrides: Vec<StockReviewOverride>,
    pub issues: Vec<StockReviewIssue>,
}

#[derive(Debug, Clone)]
pub struct ValidatedOverrideCandidate {
    pub input: StockReviewOverrideInput,
    active_override_revision: String,
    reference_fingerprint_json: String,
    review_source_revision: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TransactionReferenceFingerprint {
    id: String,
    account_id: String,
    symbol: String,
    transaction_type: String,
    shares: f64,
    price: f64,
    total_amount: f64,
    commission: f64,
    currency: String,
    traded_at: String,
}

impl TransactionReferenceFingerprint {
    fn from_transaction(transaction: &StoredTransaction) -> Self {
        Self {
            id: transaction.id.clone(),
            account_id: transaction.account_id.clone(),
            symbol: transaction.symbol.clone(),
            transaction_type: transaction.transaction_type.clone(),
            shares: transaction.shares,
            price: transaction.price,
            total_amount: transaction.total_amount,
            commission: transaction.commission,
            currency: transaction.currency.clone(),
            traded_at: transaction.traded_at.clone(),
        }
    }
}

#[derive(Debug, Clone)]
struct StoredTransaction {
    id: String,
    account_id: String,
    symbol: String,
    transaction_type: String,
    shares: f64,
    price: f64,
    total_amount: f64,
    commission: f64,
    currency: String,
    traded_at: String,
}

pub fn list_annotations(
    db: &Database,
    filter: &StockReviewAnnotationFilter,
) -> Result<Vec<StockReviewAnnotation>, String> {
    let normalized = normalize_annotation_filter(filter)?;
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT id, scope_type, scope_key, account_id, symbol, annotation_type, value_json, source, created_at, updated_at
             FROM stock_review_annotations
             WHERE (?1 IS NULL OR scope_type = ?1)
               AND (?2 IS NULL OR scope_key = ?2)
               AND (?3 IS NULL OR account_id = ?3)
               AND (?4 IS NULL OR symbol = ?4)
             ORDER BY updated_at ASC, id ASC",
        )
        .map_err(|error| error.to_string())?;
    let annotations = statement
        .query_map(
            params![
                normalized.scope_type,
                normalized.scope_key,
                normalized.account_id,
                normalized.symbol,
            ],
            map_annotation,
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(annotations)
}

pub fn save_annotation(
    db: &Database,
    input: StockReviewAnnotationInput,
    context: AnnotationSaveContext,
) -> Result<StockReviewAnnotation, String> {
    let input = normalize_annotation_input(input, context)?;
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    let now = next_audit_timestamp(&transaction, "stock_review_annotations", &input.id)?;
    transaction
        .execute(
            "INSERT INTO stock_review_annotations
                (id, scope_type, scope_key, account_id, symbol, annotation_type, value_json, source, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
             ON CONFLICT(id) DO UPDATE SET
                scope_type = excluded.scope_type,
                scope_key = excluded.scope_key,
                account_id = excluded.account_id,
                symbol = excluded.symbol,
                annotation_type = excluded.annotation_type,
                value_json = excluded.value_json,
                source = excluded.source,
                updated_at = excluded.updated_at",
            params![
                input.id,
                input.scope_type,
                input.scope_key,
                input.account_id,
                input.symbol,
                input.annotation_type,
                input.value_json,
                input.source,
                now,
            ],
        )
        .map_err(|error| error.to_string())?;
    let annotation = transaction
        .query_row(
            "SELECT id, scope_type, scope_key, account_id, symbol, annotation_type, value_json, source, created_at, updated_at
             FROM stock_review_annotations WHERE id = ?1",
            params![input.id],
            map_annotation,
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(annotation)
}

/// Validate persisted correction semantics without changing the database.
pub fn validate_override(
    db: &Database,
    input: &StockReviewOverrideInput,
) -> Result<OverrideValidationResult, String> {
    let input = match normalize_override_input(input.clone()) {
        Ok(input) => input,
        Err(message) => return Ok(invalid_override_input_result(message)),
    };
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    validate_normalized_override(&conn, &input)
}

pub fn save_override(
    db: &Database,
    input: StockReviewOverrideInput,
) -> Result<StockReviewOverride, String> {
    let candidate = prepare_override_candidate(db, input)?;
    save_override_candidate(db, candidate)
}

pub fn prepare_override_candidate(
    db: &Database,
    input: StockReviewOverrideInput,
) -> Result<ValidatedOverrideCandidate, String> {
    let input = normalize_override_input(input)?;
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let validation = validate_normalized_override(&conn, &input)?;
    if !validation.is_valid {
        return Err(format_override_validation_error(&validation));
    }
    let references =
        load_transactions(&conn, &parse_transaction_ids(&input.transaction_ids_json)?)?;
    let fingerprints = references
        .iter()
        .map(TransactionReferenceFingerprint::from_transaction)
        .collect::<Vec<_>>();
    Ok(ValidatedOverrideCandidate {
        input,
        active_override_revision: active_override_revision(&conn)?,
        reference_fingerprint_json: serde_json::to_string(&fingerprints)
            .map_err(|error| error.to_string())?,
        review_source_revision: review_source_revision(&conn)?,
    })
}

/// Refresh only after the asynchronous cache preparation owned by this
/// candidate has completed. The active override revision deliberately remains
/// the original one so a concurrent confirmation is still detected.
pub fn refresh_candidate_source_revision(
    db: &Database,
    candidate: &mut ValidatedOverrideCandidate,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    candidate.review_source_revision = review_source_revision(&conn)?;
    Ok(())
}

pub fn save_override_candidate(
    db: &Database,
    candidate: ValidatedOverrideCandidate,
) -> Result<StockReviewOverride, String> {
    let input = candidate.input;
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    if active_override_revision(&transaction)? != candidate.active_override_revision {
        return Err(
            "The active override set changed while the candidate report was being built; rebuild the report before confirming."
                .to_string(),
        );
    }
    if review_source_revision(&transaction)? != candidate.review_source_revision {
        return Err(
            "A report source changed while the candidate report was being built; rebuild before confirming."
                .to_string(),
        );
    }
    let validation = validate_normalized_override(&transaction, &input)?;
    if !validation.is_valid {
        return Err(format_override_validation_error(&validation));
    }
    let references = load_transactions(
        &transaction,
        &parse_transaction_ids(&input.transaction_ids_json)?,
    )?;
    let fingerprints = references
        .iter()
        .map(TransactionReferenceFingerprint::from_transaction)
        .collect::<Vec<_>>();
    let current_fingerprint_json =
        serde_json::to_string(&fingerprints).map_err(|error| error.to_string())?;
    if current_fingerprint_json != candidate.reference_fingerprint_json {
        return Err(
            "The referenced source transactions changed while the candidate report was being built; rebuild before confirming."
                .to_string(),
        );
    }
    let now = next_audit_timestamp(&transaction, "stock_review_overrides", &input.id)?;
    transaction
        .execute(
            "INSERT INTO stock_review_overrides
                (id, override_type, transaction_ids_json, value_json, reference_fingerprint_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)
             ON CONFLICT(id) DO UPDATE SET
                override_type = excluded.override_type,
                transaction_ids_json = excluded.transaction_ids_json,
                value_json = excluded.value_json,
                reference_fingerprint_json = excluded.reference_fingerprint_json,
                updated_at = excluded.updated_at",
            params![
                input.id,
                input.override_type,
                input.transaction_ids_json,
                input.value_json,
                current_fingerprint_json,
                now,
            ],
        )
        .map_err(|error| error.to_string())?;
    let override_record = transaction
        .query_row(
            "SELECT id, override_type, transaction_ids_json, value_json, created_at, updated_at
             FROM stock_review_overrides WHERE id = ?1",
            params![input.id],
            map_override,
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(override_record)
}

fn active_override_revision(conn: &Connection) -> Result<String, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, updated_at, reference_fingerprint_json
             FROM stock_review_overrides ORDER BY id ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    serde_json::to_string(&rows).map_err(|error| error.to_string())
}

fn review_source_revision(conn: &Connection) -> Result<String, String> {
    const TABLES: &[&str] = &[
        "transactions",
        "holdings",
        "daily_portfolio_values",
        "daily_holding_snapshots",
        "benchmark_daily_prices",
        "stock_daily_prices",
        "stock_market_sessions",
        "stock_splits",
        "stock_review_annotations",
        "cached_exchange_rates",
    ];
    let mut revision = Vec::<(&str, Vec<Vec<String>>)>::new();
    for table in TABLES {
        let mut columns_statement = conn
            .prepare(&format!("PRAGMA table_info({table})"))
            .map_err(|error| error.to_string())?;
        let columns = columns_statement
            .query_map([], |row| row.get::<_, String>(1))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let sql = format!("SELECT {} FROM {table} ORDER BY rowid", columns.join(", "));
        let mut statement = conn.prepare(&sql).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                (0..columns.len())
                    .map(|index| row.get_ref(index).map(|value| format!("{value:?}")))
                    .collect::<Result<Vec<_>, _>>()
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        revision.push((table, rows));
    }
    serde_json::to_string(&revision).map_err(|error| error.to_string())
}

pub fn list_overrides(db: &Database) -> Result<StockReviewOverrideList, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT id, override_type, transaction_ids_json, value_json, created_at, updated_at, reference_fingerprint_json
             FROM stock_review_overrides ORDER BY created_at ASC, id ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| Ok((map_override(row)?, row.get::<_, String>(6)?)))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let mut result = StockReviewOverrideList {
        overrides: Vec::new(),
        stale_overrides: Vec::new(),
        issues: Vec::new(),
    };
    for (override_record, stored_fingerprint_json) in rows {
        let input = StockReviewOverrideInput {
            id: override_record.id.clone(),
            override_type: override_record.override_type.clone(),
            transaction_ids_json: override_record.transaction_ids_json.clone(),
            value_json: override_record.value_json.clone(),
        };
        let validation = normalize_override_input(input)
            .and_then(|input| validate_normalized_override(&conn, &input));
        let fingerprints =
            serde_json::from_str::<Vec<TransactionReferenceFingerprint>>(&stored_fingerprint_json);
        let is_current = match (validation, fingerprints) {
            (Ok(validation), Ok(fingerprints))
                if validation.is_valid && !fingerprints.is_empty() =>
            {
                let ids = parse_transaction_ids(&override_record.transaction_ids_json)?;
                let current = load_transactions(&conn, &ids)?;
                current.len() == fingerprints.len()
                    && current
                        .iter()
                        .map(TransactionReferenceFingerprint::from_transaction)
                        .eq(fingerprints.into_iter())
            }
            _ => false,
        };
        if is_current {
            result.overrides.push(override_record);
        } else {
            result.issues.push(stale_override_issue(&override_record));
            result.stale_overrides.push(override_record);
        }
    }
    Ok(result)
}

fn normalize_annotation_filter(
    filter: &StockReviewAnnotationFilter,
) -> Result<StockReviewAnnotationFilter, String> {
    Ok(StockReviewAnnotationFilter {
        scope_type: filter
            .scope_type
            .as_deref()
            .map(normalize_scope_type)
            .transpose()?,
        scope_key: filter
            .scope_key
            .as_deref()
            .map(|value| normalize_identifier("scope_key", value))
            .transpose()?,
        account_id: filter
            .account_id
            .as_deref()
            .map(|value| normalize_identifier("account_id", value))
            .transpose()?,
        symbol: filter.symbol.as_deref().map(normalize_symbol).transpose()?,
    })
}

fn normalize_annotation_input(
    input: StockReviewAnnotationInput,
    context: AnnotationSaveContext,
) -> Result<StockReviewAnnotationInput, String> {
    let value: Value = serde_json::from_str(&input.value_json)
        .map_err(|error| format!("Annotation value_json must be valid JSON: {error}"))?;
    if !value.is_object() {
        return Err("Annotation value_json must be a JSON object.".to_string());
    }
    let source = input.source.trim();
    match (source, context) {
        ("user", AnnotationSaveContext::UserInitiated)
        | ("ai_confirmed", AnnotationSaveContext::AiAfterExplicitUserConfirmation) => {}
        ("ai_confirmed", _) => {
            return Err("ai_confirmed annotations require explicit user confirmation.".to_string())
        }
        ("user", _) => {
            return Err("User annotations must use the user-initiated context.".to_string())
        }
        _ => return Err("Unknown annotation source.".to_string()),
    }
    Ok(StockReviewAnnotationInput {
        id: normalize_identifier("annotation id", &input.id)?,
        scope_type: normalize_scope_type(&input.scope_type)?,
        scope_key: normalize_identifier("scope_key", &input.scope_key)?,
        account_id: input
            .account_id
            .as_deref()
            .map(|value| normalize_identifier("account_id", value))
            .transpose()?,
        symbol: input.symbol.as_deref().map(normalize_symbol).transpose()?,
        annotation_type: normalize_identifier("annotation_type", &input.annotation_type)?,
        value_json: input.value_json,
        source: source.to_string(),
    })
}

fn normalize_override_input(
    input: StockReviewOverrideInput,
) -> Result<StockReviewOverrideInput, String> {
    let override_type = input.override_type.trim();
    if !matches!(
        override_type,
        "transfer" | "duplicate" | "same_day_order" | "non_trade"
    ) {
        return Err("Unknown stock review override type.".to_string());
    }
    let transaction_ids = parse_transaction_ids(&input.transaction_ids_json)?;
    match override_type {
        "same_day_order" => {
            let order = parse_transaction_ids(&input.value_json)?;
            if order.len() != transaction_ids.len()
                || order.iter().collect::<HashSet<_>>()
                    != transaction_ids.iter().collect::<HashSet<_>>()
            {
                return Err("same_day_order value_json must contain each referenced transaction ID exactly once.".to_string());
            }
        }
        _ => {
            let value: Value = serde_json::from_str(&input.value_json)
                .map_err(|error| format!("Override value_json must be valid JSON: {error}"))?;
            if !value.is_object() {
                return Err("Override value_json must be a JSON object.".to_string());
            }
        }
    }
    Ok(StockReviewOverrideInput {
        id: normalize_identifier("override id", &input.id)?,
        override_type: override_type.to_string(),
        transaction_ids_json: serde_json::to_string(&transaction_ids)
            .map_err(|error| error.to_string())?,
        value_json: input.value_json,
    })
}

fn validate_normalized_override(
    conn: &Connection,
    input: &StockReviewOverrideInput,
) -> Result<OverrideValidationResult, String> {
    let ids = parse_transaction_ids(&input.transaction_ids_json)?;
    let transactions = load_transactions(conn, &ids)?;
    let mut issues = missing_transaction_issues(&ids, &transactions);
    if issues.is_empty() {
        match input.override_type.as_str() {
            "transfer" => validate_transfer(&transactions, &mut issues),
            "duplicate" => validate_duplicate(&transactions, &mut issues),
            "same_day_order" => validate_same_day_order(conn, &transactions, input, &mut issues)?,
            "non_trade" => validate_non_trade(&transactions, &mut issues),
            _ => unreachable!("normalized override type is checked above"),
        }
    }
    Ok(OverrideValidationResult {
        is_valid: issues.is_empty(),
        issues,
    })
}

fn validate_transfer(transactions: &[StoredTransaction], issues: &mut Vec<StockReviewIssue>) {
    if transactions.len() != 2 {
        issues.push(validation_issue(
            "invalid_transfer",
            "A transfer must reference exactly two transactions.",
        ));
        return;
    }
    let first = &transactions[0];
    let second = &transactions[1];
    let reversed_sides = (normalized_type(&first.transaction_type) == "SELL"
        && normalized_type(&second.transaction_type) == "BUY")
        || (normalized_type(&first.transaction_type) == "BUY"
            && normalized_type(&second.transaction_type) == "SELL");
    if first.account_id == second.account_id
        || normalize_symbol(&first.symbol).ok() != normalize_symbol(&second.symbol).ok()
        || !reversed_sides
        || !approximately_equal(first.shares.abs(), second.shares.abs())
        || first.shares <= 0.0
        || second.shares <= 0.0
    {
        issues.push(validation_issue(
            "invalid_transfer",
            "Transfer references must be opposite BUY/SELL records in different accounts with the same normalized symbol and quantity.",
        ));
    }
}

fn validate_duplicate(transactions: &[StoredTransaction], issues: &mut Vec<StockReviewIssue>) {
    if transactions.len() < 2 {
        issues.push(validation_issue(
            "invalid_duplicate",
            "A duplicate override must reference at least two transactions.",
        ));
        return;
    }
    let first = &transactions[0];
    if transactions
        .iter()
        .skip(1)
        .any(|transaction| !economically_equivalent(first, transaction))
    {
        issues.push(validation_issue(
            "invalid_duplicate",
            "Duplicate transactions must have the same account, symbol, type, trade date, quantity, price, amount, commission, and currency.",
        ));
    }
}

fn validate_same_day_order(
    conn: &Connection,
    transactions: &[StoredTransaction],
    input: &StockReviewOverrideInput,
    issues: &mut Vec<StockReviewIssue>,
) -> Result<(), String> {
    if transactions.len() < 2 {
        issues.push(validation_issue(
            "invalid_same_day_order",
            "A same-day order must reference at least two transactions.",
        ));
        return Ok(());
    }
    let first = &transactions[0];
    let Some(date) = trade_date(&first.traded_at) else {
        issues.push(validation_issue(
            "invalid_same_day_order",
            "Same-day order references require valid trade dates.",
        ));
        return Ok(());
    };
    let same_position_day = transactions.iter().all(|transaction| {
        transaction.account_id == first.account_id
            && normalize_symbol(&transaction.symbol).ok() == normalize_symbol(&first.symbol).ok()
            && trade_date(&transaction.traded_at) == Some(date)
            && matches!(
                normalized_type(&transaction.transaction_type).as_str(),
                "BUY" | "SELL"
            )
    });
    let has_reversal = transactions
        .iter()
        .any(|transaction| normalized_type(&transaction.transaction_type) == "BUY")
        && transactions
            .iter()
            .any(|transaction| normalized_type(&transaction.transaction_type) == "SELL");
    let input_ids = parse_transaction_ids(&input.transaction_ids_json)?;
    let full_database_order_set =
        same_day_reversal_ids(conn, &first.account_id, &first.symbol, date)?;
    if !same_position_day
        || !has_reversal
        || full_database_order_set.len() != input_ids.len()
        || !full_database_order_set
            .iter()
            .all(|id| input_ids.contains(id))
    {
        issues.push(validation_issue(
            "invalid_same_day_order",
            "Same-day order must contain the complete ordered BUY/SELL reversal set for one account, normalized symbol, and trade date.",
        ));
    }
    Ok(())
}

fn validate_non_trade(transactions: &[StoredTransaction], issues: &mut Vec<StockReviewIssue>) {
    if transactions.len() != 1 {
        issues.push(validation_issue(
            "invalid_non_trade",
            "A non-trade override must reference exactly one existing transaction.",
        ));
    }
}

fn load_transactions(conn: &Connection, ids: &[String]) -> Result<Vec<StoredTransaction>, String> {
    let mut transactions = Vec::with_capacity(ids.len());
    for id in ids {
        if let Some(transaction) = conn
            .query_row(
                "SELECT id, account_id, symbol, transaction_type, shares, price, total_amount, commission, currency, traded_at
                 FROM transactions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(StoredTransaction {
                        id: row.get(0)?,
                        account_id: row.get(1)?,
                        symbol: row.get(2)?,
                        transaction_type: row.get(3)?,
                        shares: row.get(4)?,
                        price: row.get(5)?,
                        total_amount: row.get(6)?,
                        commission: row.get(7)?,
                        currency: row.get(8)?,
                        traded_at: row.get(9)?,
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())?
        {
            transactions.push(transaction);
        }
    }
    Ok(transactions)
}

fn same_day_reversal_ids(
    conn: &Connection,
    account_id: &str,
    symbol: &str,
    date: chrono::NaiveDate,
) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id FROM transactions
             WHERE account_id = ?1 AND UPPER(TRIM(symbol)) = UPPER(TRIM(?2)) AND substr(traded_at, 1, 10) = ?3
               AND transaction_type IN ('BUY', 'SELL')
             ORDER BY id ASC",
        )
        .map_err(|error| error.to_string())?;
    let ids = statement
        .query_map(
            params![account_id, symbol, date.format("%Y-%m-%d").to_string()],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(ids)
}

fn missing_transaction_issues(
    ids: &[String],
    transactions: &[StoredTransaction],
) -> Vec<StockReviewIssue> {
    let found = transactions
        .iter()
        .map(|transaction| transaction.id.as_str())
        .collect::<HashSet<_>>();
    ids.iter()
        .filter(|id| !found.contains(id.as_str()))
        .map(|id| {
            validation_issue(
                "missing_transaction",
                &format!("Referenced transaction '{id}' does not exist."),
            )
        })
        .collect()
}

fn parse_transaction_ids(json: &str) -> Result<Vec<String>, String> {
    let value: Value = serde_json::from_str(json)
        .map_err(|error| format!("transaction_ids_json must be valid JSON: {error}"))?;
    let Value::Array(values) = value else {
        return Err("transaction_ids_json must be a JSON array of IDs.".to_string());
    };
    if values.is_empty() {
        return Err("transaction_ids_json must contain at least one ID.".to_string());
    }
    let mut seen = HashSet::new();
    values
        .into_iter()
        .map(|value| {
            let Value::String(id) = value else {
                return Err("transaction_ids_json must contain only string IDs.".to_string());
            };
            let id = normalize_identifier("transaction id", &id)?;
            if !seen.insert(id.clone()) {
                return Err("transaction_ids_json must not contain duplicate IDs.".to_string());
            }
            Ok(id)
        })
        .collect()
}

fn economically_equivalent(left: &StoredTransaction, right: &StoredTransaction) -> bool {
    left.account_id == right.account_id
        && normalize_symbol(&left.symbol).ok() == normalize_symbol(&right.symbol).ok()
        && normalized_type(&left.transaction_type) == normalized_type(&right.transaction_type)
        && trade_date(&left.traded_at) == trade_date(&right.traded_at)
        && approximately_equal(left.shares, right.shares)
        && approximately_equal(left.price, right.price)
        && approximately_equal(left.total_amount, right.total_amount)
        && approximately_equal(left.commission, right.commission)
        && left
            .currency
            .trim()
            .eq_ignore_ascii_case(right.currency.trim())
}

fn map_annotation(row: &rusqlite::Row<'_>) -> rusqlite::Result<StockReviewAnnotation> {
    Ok(StockReviewAnnotation {
        id: row.get(0)?,
        scope_type: row.get(1)?,
        scope_key: row.get(2)?,
        account_id: row.get(3)?,
        symbol: row.get(4)?,
        annotation_type: row.get(5)?,
        value_json: row.get(6)?,
        source: row.get(7)?,
        created_at: row.get(8)?,
        updated_at: row.get(9)?,
    })
}

fn map_override(row: &rusqlite::Row<'_>) -> rusqlite::Result<StockReviewOverride> {
    Ok(StockReviewOverride {
        id: row.get(0)?,
        override_type: row.get(1)?,
        transaction_ids_json: row.get(2)?,
        value_json: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

fn normalize_scope_type(value: &str) -> Result<String, String> {
    let value = value.trim().to_ascii_lowercase();
    if matches!(value.as_str(), "period" | "stock" | "campaign" | "action") {
        Ok(value)
    } else {
        Err("Unknown annotation scope_type.".to_string())
    }
}

fn normalize_symbol(value: &str) -> Result<String, String> {
    crate::models::stock_review::normalized_stock_symbol(value)
        .ok_or_else(|| "symbol must not be empty.".to_string())
}

fn next_audit_timestamp(conn: &Connection, table: &str, id: &str) -> Result<String, String> {
    let sql = match table {
        "stock_review_annotations" | "stock_review_overrides" => {
            format!("SELECT created_at, updated_at FROM {table} WHERE id = ?1")
        }
        _ => return Err("Unsupported audit table.".to_string()),
    };
    let prior = conn
        .query_row(&sql, params![id], |row| Ok((row.get(0)?, row.get(1)?)))
        .optional()
        .map_err(|error| error.to_string())?;
    Ok(monotonic_audit_timestamp(Utc::now(), prior.as_ref())?
        .to_rfc3339_opts(SecondsFormat::Nanos, true))
}

fn monotonic_audit_timestamp(
    now: DateTime<Utc>,
    prior: Option<&(String, String)>,
) -> Result<DateTime<Utc>, String> {
    let Some((created_at, updated_at)) = prior else {
        return Ok(now);
    };
    let created_at = DateTime::parse_from_rfc3339(created_at)
        .map_err(|error| format!("Invalid persisted created_at: {error}"))?
        .with_timezone(&Utc);
    let updated_at = DateTime::parse_from_rfc3339(updated_at)
        .map_err(|error| format!("Invalid persisted updated_at: {error}"))?
        .with_timezone(&Utc);
    let latest = created_at.max(updated_at);
    if now > latest {
        Ok(now)
    } else {
        latest
            .checked_add_signed(Duration::nanoseconds(1))
            .ok_or_else(|| "Audit timestamp overflow.".to_string())
    }
}

fn normalize_identifier(name: &str, value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.is_empty() {
        Err(format!("{name} must not be empty."))
    } else {
        Ok(value.to_string())
    }
}

fn normalized_type(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

fn trade_date(value: &str) -> Option<chrono::NaiveDate> {
    value
        .get(..10)
        .and_then(|date| chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
}

fn approximately_equal(left: f64, right: f64) -> bool {
    left.is_finite()
        && right.is_finite()
        && (left - right).abs() <= EPSILON * left.abs().max(right.abs()).max(1.0)
}

fn validation_issue(code: &str, message: &str) -> StockReviewIssue {
    StockReviewIssue {
        code: code.to_string(),
        severity: StockReviewIssueSeverity::Error,
        message: message.to_string(),
        affected_symbol: None,
        affected_date: None,
    }
}

fn invalid_override_input_result(message: String) -> OverrideValidationResult {
    OverrideValidationResult {
        is_valid: false,
        issues: vec![validation_issue("invalid_override_input", &message)],
    }
}

fn stale_override_issue(override_record: &StockReviewOverride) -> StockReviewIssue {
    StockReviewIssue {
        code: "stale_override".to_string(),
        severity: StockReviewIssueSeverity::Warning,
        message: format!(
            "Override {} is excluded because its referenced transactions no longer match the confirmed ledger state.",
            override_record.id
        ),
        affected_symbol: None,
        affected_date: None,
    }
}

fn format_override_validation_error(validation: &OverrideValidationResult) -> String {
    validation
        .issues
        .iter()
        .map(|issue| issue.message.as_str())
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::{
        list_annotations, list_overrides, monotonic_audit_timestamp, prepare_override_candidate,
        save_annotation, save_override, save_override_candidate, validate_override,
        AnnotationSaveContext, StockReviewAnnotationFilter,
    };
    use crate::db::Database;
    use crate::models::stock_review::{
        StockReviewAnnotationInput, StockReviewIssueSeverity, StockReviewOverrideInput,
    };
    use rusqlite::params;

    fn database() -> Database {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, description, created_at, updated_at) VALUES (?1, ?2, 'US', NULL, ?3, ?3)",
            params!["acct-a", "Account A", "2024-01-01T00:00:00Z"],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, description, created_at, updated_at) VALUES (?1, ?2, 'US', NULL, ?3, ?3)",
            params!["acct-b", "Account B", "2024-01-01T00:00:00Z"],
        )
        .unwrap();
        drop(conn);
        db
    }

    fn insert_transaction(
        db: &Database,
        id: &str,
        account_id: &str,
        symbol: &str,
        transaction_type: &str,
        shares: f64,
        price: f64,
        traded_at: &str,
    ) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO transactions
                 (id, holding_id, account_id, symbol, name, market, transaction_type,
                  shares, price, total_amount, commission, currency, traded_at, notes, created_at)
             VALUES (?1, NULL, ?2, ?3, ?3, 'US', ?4, ?5, ?6, ?7, 0, 'USD', ?8, NULL, ?9)",
            params![
                id,
                account_id,
                symbol,
                transaction_type,
                shares,
                price,
                shares * price,
                traded_at,
                "2024-01-01T00:00:00Z",
            ],
        )
        .unwrap();
    }

    fn annotation_input(id: &str, value_json: &str) -> StockReviewAnnotationInput {
        StockReviewAnnotationInput {
            id: id.to_string(),
            scope_type: "campaign".to_string(),
            scope_key: "campaign:acct-a:AAPL:open-1".to_string(),
            account_id: Some("acct-a".to_string()),
            symbol: Some("aapl".to_string()),
            annotation_type: "investment_hypothesis".to_string(),
            value_json: value_json.to_string(),
            source: "user".to_string(),
        }
    }

    fn override_input(
        id: &str,
        override_type: &str,
        transaction_ids: &[&str],
        value_json: &str,
    ) -> StockReviewOverrideInput {
        StockReviewOverrideInput {
            id: id.to_string(),
            override_type: override_type.to_string(),
            transaction_ids_json: serde_json::to_string(transaction_ids).unwrap(),
            value_json: value_json.to_string(),
        }
    }

    #[test]
    fn annotation_upsert_preserves_created_at_and_filters_exact_scope_account_and_symbol() {
        // Replacing created_at, accepting a scalar value, or broadening any
        // filter would corrupt annotation history or attach it to the wrong review.
        let db = database();
        let first = save_annotation(
            &db,
            annotation_input("annotation-1", r#"{"thesis":"initial"}"#),
            AnnotationSaveContext::UserInitiated,
        )
        .unwrap();
        let updated = save_annotation(
            &db,
            annotation_input("annotation-1", r#"{"thesis":"revised"}"#),
            AnnotationSaveContext::UserInitiated,
        )
        .unwrap();

        assert_eq!(updated.created_at, first.created_at);
        assert_ne!(updated.updated_at, first.updated_at);
        assert_eq!(updated.symbol.as_deref(), Some("AAPL"));

        let annotations = list_annotations(
            &db,
            &StockReviewAnnotationFilter {
                scope_type: Some("campaign".to_string()),
                scope_key: Some("campaign:acct-a:AAPL:open-1".to_string()),
                account_id: Some("acct-a".to_string()),
                symbol: Some("aapl".to_string()),
            },
        )
        .unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].value_json, r#"{"thesis":"revised"}"#);

        let no_match = list_annotations(
            &db,
            &StockReviewAnnotationFilter {
                scope_type: Some("campaign".to_string()),
                scope_key: None,
                account_id: Some("acct-b".to_string()),
                symbol: Some("AAPL".to_string()),
            },
        )
        .unwrap();
        assert!(no_match.is_empty());

        assert!(save_annotation(
            &db,
            annotation_input("annotation-scalar", "[]"),
            AnnotationSaveContext::UserInitiated,
        )
        .is_err());
    }

    #[test]
    fn stable_id_upserts_advance_annotation_and_override_audit_times_without_sleeping() {
        // Reusing a same-tick or regressed clock value would make an update
        // indistinguishable from its prior persisted revision.
        let db = database();
        let annotation_first = save_annotation(
            &db,
            annotation_input("audit-annotation", r#"{"thesis":"first"}"#),
            AnnotationSaveContext::UserInitiated,
        )
        .unwrap();
        let annotation_second = save_annotation(
            &db,
            annotation_input("audit-annotation", r#"{"thesis":"second"}"#),
            AnnotationSaveContext::UserInitiated,
        )
        .unwrap();
        assert_eq!(annotation_second.created_at, annotation_first.created_at);
        assert!(
            chrono::DateTime::parse_from_rfc3339(&annotation_second.updated_at).unwrap()
                > chrono::DateTime::parse_from_rfc3339(&annotation_first.updated_at).unwrap()
        );

        insert_transaction(
            &db,
            "audit-non-trade",
            "acct-a",
            "AAPL",
            "OPEN",
            1.0,
            100.0,
            "2024-02-01",
        );
        let override_first = save_override(
            &db,
            override_input("audit-override", "non_trade", &["audit-non-trade"], "{}"),
        )
        .unwrap();
        let override_second = save_override(
            &db,
            override_input(
                "audit-override",
                "non_trade",
                &["audit-non-trade"],
                r#"{"note":"second"}"#,
            ),
        )
        .unwrap();
        assert_eq!(override_second.created_at, override_first.created_at);
        assert!(
            chrono::DateTime::parse_from_rfc3339(&override_second.updated_at).unwrap()
                > chrono::DateTime::parse_from_rfc3339(&override_first.updated_at).unwrap()
        );
    }

    #[test]
    fn audit_timestamp_advances_when_the_clock_is_equal_or_behind_the_saved_audit_time() {
        // A wall-clock rollback must never make an idempotent update appear
        // older than the correction/annotation revision it replaces.
        let prior = (
            "2024-02-01T00:00:00.000000010Z".to_string(),
            "2024-02-01T00:00:00.000000010Z".to_string(),
        );
        let now = chrono::DateTime::parse_from_rfc3339("2024-02-01T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        assert_eq!(
            monotonic_audit_timestamp(now, Some(&prior)).unwrap(),
            chrono::DateTime::parse_from_rfc3339("2024-02-01T00:00:00.000000011Z")
                .unwrap()
                .with_timezone(&chrono::Utc)
        );
    }

    #[test]
    fn ai_confirmed_annotation_requires_the_explicit_confirmation_context() {
        // Treating provenance text as authorization would allow an AI writer
        // to self-authorize a durable user-confirmed annotation.
        let db = database();
        let mut input = annotation_input("annotation-ai", r#"{"reason":"confirmed"}"#);
        input.source = "ai_confirmed".to_string();

        assert!(save_annotation(&db, input.clone(), AnnotationSaveContext::UserInitiated).is_err());
        let saved = save_annotation(
            &db,
            input,
            AnnotationSaveContext::AiAfterExplicitUserConfirmation,
        )
        .unwrap();
        assert_eq!(saved.source, "ai_confirmed");
    }

    #[test]
    fn validate_override_checks_all_types_without_writing_and_save_is_idempotent() {
        // Omitting semantic checks would let corrections reclassify unrelated
        // ledger rows; validation itself must never create durable corrections.
        let db = database();
        insert_transaction(
            &db,
            "transfer-out",
            "acct-a",
            "AAPL",
            "SELL",
            10.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            &db,
            "transfer-in",
            "acct-b",
            "aapl",
            "BUY",
            10.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            &db,
            "duplicate-a",
            "acct-a",
            "MSFT",
            "BUY",
            3.0,
            200.0,
            "2024-02-02",
        );
        insert_transaction(
            &db,
            "duplicate-b",
            "acct-a",
            "msft",
            "BUY",
            3.0,
            200.0,
            "2024-02-02",
        );
        insert_transaction(
            &db,
            "order-buy",
            "acct-a",
            "NVDA",
            "BUY",
            2.0,
            500.0,
            "2024-02-03",
        );
        insert_transaction(
            &db,
            "order-sell",
            "acct-a",
            "nvda",
            "SELL",
            2.0,
            500.0,
            "2024-02-03",
        );
        insert_transaction(
            &db,
            "non-trade",
            "acct-a",
            "META",
            "OPEN",
            1.0,
            300.0,
            "2024-02-04",
        );

        let cases = [
            override_input(
                "transfer-1",
                "transfer",
                &["transfer-out", "transfer-in"],
                "{}",
            ),
            override_input(
                "duplicate-1",
                "duplicate",
                &["duplicate-a", "duplicate-b"],
                "{}",
            ),
            override_input(
                "order-1",
                "same_day_order",
                &["order-buy", "order-sell"],
                r#"["order-buy","order-sell"]"#,
            ),
            override_input("non-trade-1", "non_trade", &["non-trade"], "{}"),
        ];
        for input in cases {
            let validation = validate_override(&db, &input).unwrap();
            assert!(
                validation.is_valid,
                "{}: {:?}",
                input.override_type, validation.issues
            );
        }
        let count_before: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM stock_review_overrides", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count_before, 0);

        let first = save_override(
            &db,
            override_input(
                "transfer-1",
                "transfer",
                &["transfer-out", "transfer-in"],
                "{}",
            ),
        )
        .unwrap();
        let second = save_override(
            &db,
            override_input(
                "transfer-1",
                "transfer",
                &["transfer-out", "transfer-in"],
                r#"{"note":"reviewed"}"#,
            ),
        )
        .unwrap();
        assert_eq!(first.created_at, second.created_at);
        assert_ne!(first.updated_at, second.updated_at);
        assert_eq!(
            second.transaction_ids_json,
            r#"["transfer-out","transfer-in"]"#
        );
    }

    #[test]
    fn prepared_override_rejects_a_changed_active_override_revision() {
        // Saving another active correction after preview must invalidate the
        // prepared candidate instead of returning a stale report.
        let db = database();
        insert_transaction(
            &db,
            "first",
            "acct-a",
            "AAPL",
            "BUY",
            1.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            &db,
            "second",
            "acct-a",
            "MSFT",
            "BUY",
            1.0,
            100.0,
            "2024-02-01",
        );
        let candidate = prepare_override_candidate(
            &db,
            override_input("candidate", "non_trade", &["first"], "{}"),
        )
        .unwrap();
        save_override(
            &db,
            override_input("concurrent", "non_trade", &["second"], "{}"),
        )
        .unwrap();
        assert!(save_override_candidate(&db, candidate).is_err());
        assert_eq!(list_overrides(&db).unwrap().overrides.len(), 1);
    }

    #[test]
    fn prepared_override_rejects_a_changed_full_review_source_revision() {
        // A split, price, session, FX, annotation, or other report source
        // changing after candidate materialization must prevent a stale save.
        let db = database();
        insert_transaction(
            &db,
            "buy",
            "acct-a",
            "AAPL",
            "BUY",
            1.0,
            100.0,
            "2024-02-01",
        );
        let candidate = prepare_override_candidate(
            &db,
            override_input("candidate", "non_trade", &["buy"], "{}"),
        )
        .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO stock_splits (stock_code, split_date, ratio_from, ratio_to, created_at)
                 VALUES ('AAPL', '2024-02-02', 1, 2, '2024-02-01')",
                [],
            )
            .unwrap();

        assert!(save_override_candidate(&db, candidate).is_err());
        assert_eq!(list_overrides(&db).unwrap().overrides.len(), 0);
    }

    #[test]
    fn same_day_order_uses_confirmed_value_order_not_the_unordered_reference_id_list() {
        // Requiring transaction_ids_json to already be sorted would make a
        // valid explicit user order depend on incidental input-array order.
        let db = database();
        insert_transaction(
            &db,
            "buy",
            "acct-a",
            "AAPL",
            "BUY",
            2.0,
            100.0,
            "2024-02-03",
        );
        insert_transaction(
            &db,
            "sell",
            "acct-a",
            "aapl",
            "SELL",
            2.0,
            100.0,
            "2024-02-03",
        );
        let validation = validate_override(
            &db,
            &override_input(
                "order-value-authority",
                "same_day_order",
                &["sell", "buy"],
                r#"["buy","sell"]"#,
            ),
        )
        .unwrap();
        assert!(validation.is_valid, "{:?}", validation.issues);
    }

    #[test]
    fn same_day_order_accepts_trimmed_case_equivalent_symbols_from_the_source_ledger() {
        // The SQL completeness lookup must use the same trim/case identity as
        // validation, otherwise a valid confirmed reversal cannot be saved.
        let db = database();
        insert_transaction(
            &db,
            "buy",
            "acct-a",
            "AAPL",
            "BUY",
            2.0,
            100.0,
            "2024-02-03",
        );
        insert_transaction(
            &db,
            "sell",
            "acct-a",
            " aapl ",
            "SELL",
            2.0,
            100.0,
            "2024-02-03",
        );
        let input = override_input(
            "trimmed-order",
            "same_day_order",
            &["sell", "buy"],
            r#"["buy","sell"]"#,
        );

        assert!(validate_override(&db, &input).unwrap().is_valid);
        let saved = save_override(&db, input).unwrap();
        assert_eq!(saved.id, "trimmed-order");
    }

    #[test]
    fn invalid_override_has_structured_errors_and_no_database_side_effect() {
        // A missing reference must not partly persist an otherwise plausible override.
        let db = database();
        insert_transaction(
            &db,
            "exists",
            "acct-a",
            "AAPL",
            "BUY",
            1.0,
            100.0,
            "2024-02-01",
        );
        let invalid = override_input("invalid", "non_trade", &["exists", "missing"], "{}");

        let validation = validate_override(&db, &invalid).unwrap();
        assert!(!validation.is_valid);
        assert!(validation
            .issues
            .iter()
            .any(|issue| issue.code == "missing_transaction"));
        assert_eq!(
            validation.issues[0].severity,
            StockReviewIssueSeverity::Error
        );
        assert!(save_override(&db, invalid).is_err());
        let count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM stock_review_overrides", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn override_validation_rejects_each_unsafe_semantic_shape_with_structured_errors() {
        // Accepting any of these would change replay classification from an
        // ambiguous or unrelated ledger shape, rather than a user-confirmed fact.
        let db = database();
        insert_transaction(
            &db,
            "same-account-out",
            "acct-a",
            "AAPL",
            "SELL",
            5.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            &db,
            "same-account-in",
            "acct-a",
            "AAPL",
            "BUY",
            5.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            &db,
            "not-duplicate-a",
            "acct-a",
            "MSFT",
            "BUY",
            3.0,
            200.0,
            "2024-02-02",
        );
        insert_transaction(
            &db,
            "not-duplicate-b",
            "acct-a",
            "MSFT",
            "BUY",
            4.0,
            200.0,
            "2024-02-02",
        );
        insert_transaction(
            &db,
            "order-buy-a",
            "acct-a",
            "NVDA",
            "BUY",
            1.0,
            500.0,
            "2024-02-03",
        );
        insert_transaction(
            &db,
            "order-buy-b",
            "acct-a",
            "NVDA",
            "BUY",
            1.0,
            500.0,
            "2024-02-03",
        );
        insert_transaction(
            &db,
            "non-trade-a",
            "acct-a",
            "META",
            "OPEN",
            1.0,
            300.0,
            "2024-02-04",
        );
        insert_transaction(
            &db,
            "non-trade-b",
            "acct-a",
            "META",
            "OPEN",
            1.0,
            300.0,
            "2024-02-04",
        );

        let invalid_cases = [
            override_input(
                "bad-transfer",
                "transfer",
                &["same-account-out", "same-account-in"],
                "{}",
            ),
            override_input(
                "bad-duplicate",
                "duplicate",
                &["not-duplicate-a", "not-duplicate-b"],
                "{}",
            ),
            override_input(
                "bad-order",
                "same_day_order",
                &["order-buy-a", "order-buy-b"],
                r#"["order-buy-a","order-buy-b"]"#,
            ),
            override_input(
                "bad-non-trade",
                "non_trade",
                &["non-trade-a", "non-trade-b"],
                "{}",
            ),
        ];
        for input in invalid_cases {
            let validation = validate_override(&db, &input).unwrap();
            assert!(
                !validation.is_valid,
                "{} must be rejected",
                input.override_type
            );
            assert_eq!(
                validation.issues[0].severity,
                StockReviewIssueSeverity::Error
            );
        }

        let unknown = override_input("unknown", "unsafe_future_type", &["non-trade-a"], "{}");
        let validation = validate_override(&db, &unknown).unwrap();
        assert!(!validation.is_valid);
        assert_eq!(validation.issues[0].code, "invalid_override_input");
    }

    #[test]
    fn list_overrides_excludes_stale_records_but_returns_them_for_audit() {
        // Applying an override after its referenced ledger data changes would
        // silently replay a different history than the user confirmed.
        let db = database();
        insert_transaction(
            &db,
            "source",
            "acct-a",
            "AAPL",
            "SELL",
            5.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            &db,
            "destination",
            "acct-b",
            "AAPL",
            "BUY",
            5.0,
            100.0,
            "2024-02-01",
        );
        save_override(
            &db,
            override_input(
                "transfer-stale",
                "transfer",
                &["source", "destination"],
                "{}",
            ),
        )
        .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM transactions WHERE id = 'destination'", [])
            .unwrap();

        let result = list_overrides(&db).unwrap();
        assert!(result.overrides.is_empty());
        assert_eq!(result.stale_overrides.len(), 1);
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "stale_override"));
    }

    #[test]
    fn list_overrides_detects_mutated_references_before_replay() {
        // Rechecking only that IDs exist would reuse a transfer confirmation
        // after the underlying quantity changed.
        let db = database();
        insert_transaction(
            &db,
            "source",
            "acct-a",
            "AAPL",
            "SELL",
            5.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            &db,
            "destination",
            "acct-b",
            "AAPL",
            "BUY",
            5.0,
            100.0,
            "2024-02-01",
        );
        save_override(
            &db,
            override_input(
                "transfer-mutated",
                "transfer",
                &["source", "destination"],
                "{}",
            ),
        )
        .unwrap();
        db.conn.lock().unwrap().execute(
            "UPDATE transactions SET shares = 4.0, total_amount = 400.0 WHERE id = 'destination'",
            [],
        ).unwrap();

        let result = list_overrides(&db).unwrap();
        assert!(result.overrides.is_empty());
        assert_eq!(result.stale_overrides.len(), 1);
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "stale_override"));
    }
}
