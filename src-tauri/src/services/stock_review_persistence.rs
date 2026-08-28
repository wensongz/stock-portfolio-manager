#![allow(dead_code)]

use crate::db::Database;
use crate::models::stock_review::{
    normalized_stock_symbol, StockReviewAnnotation, StockReviewAnnotationInput, StockReviewIssue,
    StockReviewIssueSeverity, StockReviewOverride, StockReviewOverrideInput, StockReviewQuery,
};
use chrono::{DateTime, Duration, NaiveDate, SecondsFormat, Utc};
use rusqlite::types::{Value as SqlValue, ValueRef};
use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;

const EPSILON: f64 = 1e-9;
const REFERENCE_FINGERPRINT_VERSION: u8 = 2;

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AnnotationEconomicDates {
    pub effective_date: Option<NaiveDate>,
    pub effective_start: Option<NaiveDate>,
    pub effective_end: Option<NaiveDate>,
    pub snapshot_date: Option<NaiveDate>,
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
    preparation_scope: CandidateRevisionScope,
    source_scope: CandidateRevisionScope,
    preparation_user_revision: String,
    review_source_revision: ReviewSourceRevision,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CandidateRevisionScope {
    pub report_start: NaiveDate,
    pub report_end: NaiveDate,
    pub price_start: NaiveDate,
    pub evaluation_end: NaiveDate,
    pub current_horizon: NaiveDate,
    pub display_cutoff: NaiveDate,
    pub account_ids: Vec<String>,
    pub markets: Vec<String>,
    pub securities: Vec<(String, String)>,
    pub benchmark_symbols: Vec<String>,
    pub currencies: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ReviewSourceRevision {
    user: String,
    cache: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct TransactionReferenceFingerprint {
    id: String,
    holding_id: Option<String>,
    account_id: String,
    symbol: String,
    name: String,
    market: String,
    transaction_type: String,
    shares: f64,
    price: f64,
    total_amount: f64,
    commission: f64,
    currency: String,
    traded_at: String,
    notes: Option<String>,
    created_at: String,
}

impl TransactionReferenceFingerprint {
    fn from_transaction(transaction: &StoredTransaction) -> Self {
        Self {
            id: transaction.id.clone(),
            holding_id: transaction.holding_id.clone(),
            account_id: transaction.account_id.trim().to_string(),
            symbol: normalize_symbol(&transaction.symbol)
                .unwrap_or_else(|_| transaction.symbol.trim().to_ascii_uppercase()),
            name: transaction.name.clone(),
            market: transaction.market.trim().to_ascii_uppercase(),
            transaction_type: normalized_type(&transaction.transaction_type),
            shares: transaction.shares,
            price: transaction.price,
            total_amount: transaction.total_amount,
            commission: transaction.commission,
            currency: transaction.currency.trim().to_ascii_uppercase(),
            traded_at: transaction.traded_at.clone(),
            notes: transaction.notes.clone(),
            created_at: transaction.created_at.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct VersionedTransactionReferenceFingerprints {
    version: u8,
    transactions: Vec<TransactionReferenceFingerprint>,
}

#[derive(Debug, Clone, Deserialize)]
struct LegacyTransactionReferenceFingerprint {
    id: String,
    account_id: String,
    #[allow(dead_code)]
    symbol: String,
    #[allow(dead_code)]
    transaction_type: String,
    #[allow(dead_code)]
    shares: f64,
    #[allow(dead_code)]
    price: f64,
    #[allow(dead_code)]
    total_amount: f64,
    #[allow(dead_code)]
    commission: f64,
    #[allow(dead_code)]
    currency: String,
    traded_at: String,
}

#[derive(Debug, Clone)]
enum DecodedReferenceFingerprints {
    Current(Vec<TransactionReferenceFingerprint>),
    Legacy(Vec<LegacyTransactionReferenceFingerprint>),
    Invalid,
}

#[derive(Debug, Clone)]
struct ScopedOverrideSnapshot {
    record: StockReviewOverride,
    stored_fingerprint_json: String,
    reference_ids: Vec<String>,
    current_references: Vec<Option<StoredTransaction>>,
    is_current: bool,
}

#[derive(Debug, Clone)]
struct StoredTransaction {
    id: String,
    holding_id: Option<String>,
    account_id: String,
    symbol: String,
    name: String,
    market: String,
    transaction_type: String,
    shares: f64,
    price: f64,
    total_amount: f64,
    commission: f64,
    currency: String,
    traded_at: String,
    notes: Option<String>,
    created_at: String,
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
    let today = Utc::now().date_naive();
    let first_date = references
        .iter()
        .filter_map(|transaction| trade_date(&transaction.traded_at))
        .min()
        .unwrap_or(today);
    let source_scope = CandidateRevisionScope {
        report_start: first_date,
        report_end: today,
        price_start: first_date - Duration::days(10),
        evaluation_end: today,
        current_horizon: today,
        display_cutoff: today,
        account_ids: sorted_unique(
            references
                .iter()
                .map(|transaction| transaction.account_id.clone()),
        ),
        markets: sorted_unique(
            references
                .iter()
                .map(|transaction| transaction.market.clone()),
        ),
        // Discovery must remain broad within the referenced account/market;
        // exact securities are installed only after the async dependency plan
        // is known, so a new holding cannot alter that plan unnoticed.
        securities: Vec::new(),
        benchmark_symbols: Vec::new(),
        currencies: sorted_unique(
            references
                .iter()
                .map(|transaction| transaction.currency.clone()),
        ),
    };
    let preparation_user_revision = user_source_revision(&conn, &source_scope)?;
    Ok(ValidatedOverrideCandidate {
        input,
        active_override_revision: active_override_revision(&conn, &source_scope)?,
        reference_fingerprint_json: encode_reference_fingerprints(&fingerprints)?,
        review_source_revision: review_source_revision(&conn, &source_scope)?,
        preparation_user_revision,
        preparation_scope: source_scope.clone(),
        source_scope,
    })
}

pub fn scope_candidate_to_query(
    db: &Database,
    candidate: &mut ValidatedOverrideCandidate,
    query: &StockReviewQuery,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    candidate.source_scope = CandidateRevisionScope {
        report_start: query.start_date,
        report_end: query.end_date,
        price_start: query.start_date - Duration::days(10),
        evaluation_end: query.end_date.min(Utc::now().date_naive()),
        current_horizon: Utc::now().date_naive(),
        display_cutoff: query.end_date,
        account_ids: query.account_id.clone().into_iter().collect(),
        markets: query.market.clone().into_iter().collect(),
        securities: Vec::new(),
        benchmark_symbols: query.benchmark_symbol.clone().into_iter().collect(),
        currencies: vec![query.base_currency.clone()],
    };
    candidate.preparation_scope = candidate.source_scope.clone();
    candidate.preparation_user_revision = user_source_revision(&conn, &candidate.source_scope)?;
    candidate.active_override_revision = active_override_revision(&conn, &candidate.source_scope)?;
    candidate.review_source_revision = review_source_revision(&conn, &candidate.source_scope)?;
    Ok(())
}

pub fn set_candidate_revision_scope(
    candidate: &mut ValidatedOverrideCandidate,
    mut scope: CandidateRevisionScope,
) {
    scope.account_ids.sort();
    scope.account_ids.dedup();
    scope.markets.sort();
    scope.markets.dedup();
    scope.securities.sort();
    scope.securities.dedup();
    scope.benchmark_symbols.sort();
    scope.benchmark_symbols.dedup();
    scope.currencies.sort();
    scope.currencies.dedup();
    candidate.source_scope = scope;
}

pub fn pin_candidate_source_revision_after_cache_fill(
    db: &Database,
    candidate: &mut ValidatedOverrideCandidate,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    verify_candidate_discovery_revision(&conn, candidate)?;
    candidate.active_override_revision = active_override_revision(&conn, &candidate.source_scope)?;
    candidate.review_source_revision = review_source_revision(&conn, &candidate.source_scope)?;
    Ok(())
}

pub fn verify_candidate_discovery_revision_after_cache_fill(
    db: &Database,
    candidate: &ValidatedOverrideCandidate,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    verify_candidate_discovery_revision(&conn, candidate)
}

fn verify_candidate_discovery_revision(
    conn: &Connection,
    candidate: &ValidatedOverrideCandidate,
) -> Result<(), String> {
    let current_user = user_source_revision(&conn, &candidate.preparation_scope)?;
    if current_user != candidate.preparation_user_revision {
        return Err(
            "A user-owned report source changed during cache preparation; rebuild before confirming."
                .to_string(),
        );
    }
    let current_overrides = active_override_revision(&conn, &candidate.preparation_scope)?;
    if current_overrides != candidate.active_override_revision {
        return Err(
            "The scoped override set changed during cache preparation; rebuild before confirming."
                .to_string(),
        );
    }
    Ok(())
}

pub fn verify_candidate_source_revision(
    db: &Database,
    candidate: &ValidatedOverrideCandidate,
) -> Result<(), String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    if review_source_revision(&conn, &candidate.source_scope)? != candidate.review_source_revision {
        return Err(
            "A report source changed while candidate inputs were materialized; rebuild before confirming."
                .to_string(),
        );
    }
    Ok(())
}

pub fn save_override_candidate(
    db: &Database,
    candidate: ValidatedOverrideCandidate,
) -> Result<StockReviewOverride, String> {
    let input = candidate.input;
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    if active_override_revision(&transaction, &candidate.source_scope)?
        != candidate.active_override_revision
    {
        return Err(
            "The active override set changed while the candidate report was being built; rebuild the report before confirming."
                .to_string(),
        );
    }
    if review_source_revision(&transaction, &candidate.source_scope)?
        != candidate.review_source_revision
    {
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
    let current_fingerprint_json = encode_reference_fingerprints(&fingerprints)?;
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

fn active_override_revision(
    conn: &Connection,
    scope: &CandidateRevisionScope,
) -> Result<String, String> {
    let mut digest = StableDigest::new("stock-review-overrides-v3");
    digest_scoped_override_snapshots(conn, scope, &mut digest)?;
    Ok(digest.finish())
}

fn encode_reference_fingerprints(
    fingerprints: &[TransactionReferenceFingerprint],
) -> Result<String, String> {
    serde_json::to_string(&VersionedTransactionReferenceFingerprints {
        version: REFERENCE_FINGERPRINT_VERSION,
        transactions: fingerprints.to_vec(),
    })
    .map_err(|error| error.to_string())
}

fn decode_reference_fingerprints(value: &str) -> DecodedReferenceFingerprints {
    if let Ok(versioned) = serde_json::from_str::<VersionedTransactionReferenceFingerprints>(value)
    {
        if versioned.version == REFERENCE_FINGERPRINT_VERSION {
            return DecodedReferenceFingerprints::Current(versioned.transactions);
        }
        return DecodedReferenceFingerprints::Invalid;
    }
    if let Ok(legacy) = serde_json::from_str::<Vec<LegacyTransactionReferenceFingerprint>>(value) {
        return if legacy.is_empty() {
            DecodedReferenceFingerprints::Invalid
        } else {
            DecodedReferenceFingerprints::Legacy(legacy)
        };
    }
    DecodedReferenceFingerprints::Invalid
}

fn scoped_override_snapshots(
    conn: &Connection,
    scope: Option<&CandidateRevisionScope>,
) -> Result<Vec<ScopedOverrideSnapshot>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id, override_type, transaction_ids_json, value_json,
                    created_at, updated_at, reference_fingerprint_json
             FROM stock_review_overrides
             ORDER BY id ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map([], |row| Ok((map_override(row)?, row.get::<_, String>(6)?)))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut snapshots = Vec::new();
    for (record, stored_fingerprint_json) in rows {
        let declared_transaction_ids =
            parse_transaction_ids(&record.transaction_ids_json).unwrap_or_default();
        let decoded_fingerprints = decode_reference_fingerprints(&stored_fingerprint_json);
        let mut reference_ids = declared_transaction_ids.clone();
        let mut seen = reference_ids.iter().cloned().collect::<HashSet<_>>();
        for original_id in decoded_reference_ids(&decoded_fingerprints) {
            if seen.insert(original_id.clone()) {
                reference_ids.push(original_id);
            }
        }
        let current_references = load_transaction_references(conn, &reference_ids)?;
        if !override_snapshot_is_relevant(&decoded_fingerprints, &current_references, scope) {
            continue;
        }
        let input = StockReviewOverrideInput {
            id: record.id.clone(),
            override_type: record.override_type.clone(),
            transaction_ids_json: record.transaction_ids_json.clone(),
            value_json: record.value_json.clone(),
        };
        let valid = normalize_override_input(input)
            .and_then(|input| validate_normalized_override(conn, &input))
            .is_ok_and(|validation| validation.is_valid);
        let is_current = match &decoded_fingerprints {
            DecodedReferenceFingerprints::Current(stored) if valid && !stored.is_empty() => {
                declared_transaction_ids.len() == stored.len()
                    && current_references
                        .iter()
                        .take(declared_transaction_ids.len())
                        .map(|reference| {
                            reference
                                .as_ref()
                                .map(TransactionReferenceFingerprint::from_transaction)
                        })
                        .eq(stored.iter().cloned().map(Some))
            }
            // Legacy snapshots did not include market and therefore cannot
            // prove the exact original report scope. Preserve them for audit,
            // but never silently replay them as active corrections.
            DecodedReferenceFingerprints::Legacy(_) | DecodedReferenceFingerprints::Invalid => {
                false
            }
            DecodedReferenceFingerprints::Current(_) => false,
        };
        snapshots.push(ScopedOverrideSnapshot {
            record,
            stored_fingerprint_json,
            reference_ids,
            current_references,
            is_current,
        });
    }
    Ok(snapshots)
}

fn decoded_reference_ids(decoded: &DecodedReferenceFingerprints) -> Vec<String> {
    match decoded {
        DecodedReferenceFingerprints::Current(references) => references
            .iter()
            .map(|reference| reference.id.clone())
            .collect(),
        DecodedReferenceFingerprints::Legacy(references) => references
            .iter()
            .map(|reference| reference.id.clone())
            .collect(),
        DecodedReferenceFingerprints::Invalid => Vec::new(),
    }
}

fn override_snapshot_is_relevant(
    decoded: &DecodedReferenceFingerprints,
    current: &[Option<StoredTransaction>],
    scope: Option<&CandidateRevisionScope>,
) -> bool {
    let Some(scope) = scope else {
        return true;
    };
    let current_matches = current.iter().flatten().any(|transaction| {
        transaction_reference_in_scope(
            &TransactionReferenceFingerprint::from_transaction(transaction),
            scope,
        )
    });
    let original_matches = match decoded {
        DecodedReferenceFingerprints::Current(references) => references
            .iter()
            .any(|reference| transaction_reference_in_scope(reference, scope)),
        // Market was absent in the legacy format. Account/date identity can
        // exclude an unrelated query, but a matching account is conservatively
        // audit-visible for every market and never replay-active.
        DecodedReferenceFingerprints::Legacy(references) => references
            .iter()
            .any(|reference| legacy_reference_in_scope(reference, scope)),
        // A malformed migrated row has no trustworthy original identity.
        // Keeping it visible is conservative and ensures it cannot disappear
        // merely because all current referenced rows were deleted.
        DecodedReferenceFingerprints::Invalid => true,
    };
    original_matches || current_matches
}

fn transaction_reference_in_scope(
    reference: &TransactionReferenceFingerprint,
    scope: &CandidateRevisionScope,
) -> bool {
    scope_identity_matches(
        &reference.account_id,
        Some(&reference.market),
        &reference.traded_at,
        scope,
    )
}

fn legacy_reference_in_scope(
    reference: &LegacyTransactionReferenceFingerprint,
    scope: &CandidateRevisionScope,
) -> bool {
    scope_identity_matches(&reference.account_id, None, &reference.traded_at, scope)
}

fn scope_identity_matches(
    account_id: &str,
    market: Option<&str>,
    traded_at: &str,
    scope: &CandidateRevisionScope,
) -> bool {
    let account_matches = scope.account_ids.is_empty()
        || scope
            .account_ids
            .iter()
            .any(|expected| expected.trim() == account_id.trim());
    let market_matches = scope.markets.is_empty()
        || market.is_none()
        || scope.markets.iter().any(|expected| {
            market.is_some_and(|actual| expected.trim().eq_ignore_ascii_case(actual.trim()))
        });
    let date_matches = trade_date(traded_at).is_none_or(|date| date <= scope.current_horizon);
    account_matches && market_matches && date_matches
}

fn digest_scoped_override_snapshots(
    conn: &Connection,
    scope: &CandidateRevisionScope,
    digest: &mut StableDigest,
) -> Result<(), String> {
    digest.write_frame(0x30, b"scoped_override_snapshots_v3");
    for (override_index, snapshot) in scoped_override_snapshots(conn, Some(scope))?
        .into_iter()
        .enumerate()
    {
        digest.write_frame(0x31, &(override_index as u64).to_le_bytes());
        for value in [
            snapshot.record.id.as_str(),
            snapshot.record.override_type.as_str(),
            snapshot.record.transaction_ids_json.as_str(),
            snapshot.record.value_json.as_str(),
            snapshot.record.created_at.as_str(),
            snapshot.record.updated_at.as_str(),
            snapshot.stored_fingerprint_json.as_str(),
        ] {
            digest.write_frame(0x32, value.as_bytes());
        }
        digest.write_frame(0x39, &[u8::from(snapshot.is_current)]);
        for (reference_index, (id, current)) in snapshot
            .reference_ids
            .iter()
            .zip(snapshot.current_references.iter())
            .enumerate()
        {
            digest.write_frame(0x33, &(reference_index as u64).to_le_bytes());
            digest.write_frame(0x34, id.as_bytes());
            match current {
                Some(transaction) => {
                    digest.write_frame(0x35, b"present");
                    let fingerprint =
                        TransactionReferenceFingerprint::from_transaction(transaction);
                    let encoded =
                        serde_json::to_vec(&fingerprint).map_err(|error| error.to_string())?;
                    digest.write_frame(0x36, &encoded);
                }
                None => digest.write_frame(0x37, b"tombstone"),
            }
        }
        digest.write_frame(0x38, &[]);
    }
    Ok(())
}

fn review_source_revision(
    conn: &Connection,
    scope: &CandidateRevisionScope,
) -> Result<ReviewSourceRevision, String> {
    Ok(ReviewSourceRevision {
        user: user_source_revision(conn, scope)?,
        cache: cache_source_revision(conn, scope)?,
    })
}

fn user_source_revision(
    conn: &Connection,
    scope: &CandidateRevisionScope,
) -> Result<String, String> {
    let mut digest = StableDigest::new("stock-review-user-v3");
    digest_scope(&mut digest, scope)?;
    // Scoped corrections consume every referenced ledger row, including a
    // cross-account/cross-market leg and explicit absence. Hash the same
    // centralized snapshot used by replay selection so async preparation
    // cannot bless a mutation outside the visible account filter.
    digest_scoped_override_snapshots(conn, scope, &mut digest)?;
    let accounts = json_string_list(&scope.account_ids)?;
    let markets = json_string_list(&scope.markets)?;
    let symbols = json_string_list(&sorted_unique(
        scope
            .securities
            .iter()
            .filter_map(|(symbol, _)| normalized_stock_symbol(symbol)),
    ))?;
    let securities = serde_json::to_string(&scope.securities).map_err(|error| error.to_string())?;
    let common = || {
        vec![
            SqlValue::Text(accounts.clone()),
            SqlValue::Text(markets.clone()),
            SqlValue::Text(securities.clone()),
        ]
    };
    stream_query_digest(
        conn,
        &mut digest,
        "accounts",
        "SELECT id FROM accounts
         WHERE (json_array_length(?1) = 0 OR id IN (SELECT value FROM json_each(?1)))
         ORDER BY id",
        vec![SqlValue::Text(accounts.clone())],
        1,
    )?;
    let mut values = common();
    values.push(SqlValue::Text(date_text(scope.current_horizon)));
    stream_query_digest(
        conn,
        &mut digest,
        "transactions",
        "SELECT id, holding_id, account_id, symbol, name, market, transaction_type,
                shares, price, total_amount, commission, currency, traded_at, notes, created_at
         FROM transactions
         WHERE (json_array_length(?1) = 0 OR account_id IN (SELECT value FROM json_each(?1)))
           AND (json_array_length(?2) = 0 OR market IN (SELECT value FROM json_each(?2)))
           AND substr(traded_at, 1, 10) <= ?4
         ORDER BY traded_at, created_at, id",
        values,
        15,
    )?;
    stream_query_digest(
        conn,
        &mut digest,
        "holdings",
        "SELECT id, account_id, symbol, market, shares, currency, created_at, updated_at
         FROM holdings
         WHERE (json_array_length(?1) = 0 OR account_id IN (SELECT value FROM json_each(?1)))
           AND (json_array_length(?2) = 0 OR market IN (SELECT value FROM json_each(?2)))
           AND (symbol LIKE '$CASH-%' OR json_array_length(?3) = 0 OR EXISTS (
               SELECT 1 FROM json_each(?3) security
               WHERE symbol = json_extract(security.value, '$[0]')
                 AND market = json_extract(security.value, '$[1]')
           ))
         ORDER BY account_id, market, symbol, id",
        common(),
        8,
    )?;
    stream_query_digest(
        conn,
        &mut digest,
        "holding_snapshots",
        "SELECT id, date, account_id, symbol, market, shares, avg_cost, close_price, market_value
         FROM daily_holding_snapshots
         WHERE (json_array_length(?1) = 0 OR account_id IN (SELECT value FROM json_each(?1)))
           AND (json_array_length(?2) = 0 OR market IN (SELECT value FROM json_each(?2)))
           AND date BETWEEN ?3 AND ?4
         ORDER BY date, account_id, market, symbol, id",
        vec![
            SqlValue::Text(accounts.clone()),
            SqlValue::Text(markets.clone()),
            SqlValue::Text(date_text(scope.report_start)),
            SqlValue::Text(date_text(scope.report_end)),
        ],
        9,
    )?;
    stream_query_digest(
        conn,
        &mut digest,
        "portfolio_values",
        "SELECT date, total_value, exchange_rates FROM daily_portfolio_values
         WHERE date <= ?1 ORDER BY date",
        vec![SqlValue::Text(date_text(scope.report_end))],
        3,
    )?;
    stream_query_digest(
        conn,
        &mut digest,
        "splits",
        "SELECT id, stock_code, split_date, ratio_from, ratio_to, created_at FROM stock_splits
         WHERE (json_array_length(?1) = 0 OR UPPER(TRIM(stock_code)) IN (SELECT value FROM json_each(?1)))
           AND split_date <= ?2
         ORDER BY split_date, id",
        vec![
            SqlValue::Text(symbols.clone()),
            SqlValue::Text(date_text(scope.current_horizon)),
        ],
        6,
    )?;
    stream_query_digest(
        conn,
        &mut digest,
        "annotations",
        "SELECT id, scope_type, scope_key, account_id, symbol, annotation_type,
                value_json, source, created_at, updated_at
         FROM stock_review_annotations
         WHERE (json_array_length(?1) = 0 OR account_id IN (SELECT value FROM json_each(?1)))
         ORDER BY updated_at, id",
        vec![SqlValue::Text(accounts.clone())],
        10,
    )?;
    stream_query_digest(
        conn,
        &mut digest,
        "quarterly_context",
        "SELECT qh.id, qh.account_id, qh.symbol, qh.market, qh.notes,
                qh.decision_quality, qs.snapshot_date
         FROM quarterly_holding_snapshots qh
         JOIN quarterly_snapshots qs ON qs.id = qh.quarterly_snapshot_id
         WHERE (json_array_length(?1) = 0 OR qh.account_id IN (SELECT value FROM json_each(?1)))
           AND (json_array_length(?2) = 0 OR qh.market IN (SELECT value FROM json_each(?2)))
           AND qs.snapshot_date <= ?3
         ORDER BY qs.snapshot_date, qh.id",
        vec![
            SqlValue::Text(accounts),
            SqlValue::Text(markets),
            SqlValue::Text(date_text(scope.display_cutoff)),
        ],
        7,
    )?;
    Ok(digest.finish())
}

fn cache_source_revision(
    conn: &Connection,
    scope: &CandidateRevisionScope,
) -> Result<String, String> {
    let mut digest = StableDigest::new("stock-review-cache-v2");
    digest_scope(&mut digest, scope)?;
    let markets = json_string_list(&scope.markets)?;
    let securities = serde_json::to_string(&scope.securities).map_err(|error| error.to_string())?;
    let benchmarks = json_string_list(&scope.benchmark_symbols)?;
    stream_query_digest(
        conn,
        &mut digest,
        "stock_prices",
        "SELECT symbol, market, date, open, high, low, close, volume,
                adjusted_close, dividend, source, updated_at
         FROM stock_daily_prices
         WHERE (json_array_length(?1) = 0 OR market IN (SELECT value FROM json_each(?1)))
           AND (json_array_length(?2) = 0 OR EXISTS (
               SELECT 1 FROM json_each(?2) security
               WHERE symbol = json_extract(security.value, '$[0]')
                 AND market = json_extract(security.value, '$[1]')
           ))
           AND date BETWEEN ?3 AND ?4
         ORDER BY market, symbol, date",
        vec![
            SqlValue::Text(markets.clone()),
            SqlValue::Text(securities),
            SqlValue::Text(date_text(scope.price_start)),
            SqlValue::Text(date_text(scope.evaluation_end)),
        ],
        12,
    )?;
    stream_query_digest(
        conn,
        &mut digest,
        "benchmark_prices",
        "SELECT symbol, date, close_price, change_percent FROM benchmark_daily_prices
         WHERE (json_array_length(?1) = 0 OR symbol IN (SELECT value FROM json_each(?1)))
           AND date BETWEEN ?2 AND ?3
         ORDER BY symbol, date",
        vec![
            SqlValue::Text(benchmarks),
            SqlValue::Text(date_text(scope.price_start)),
            SqlValue::Text(date_text(scope.evaluation_end)),
        ],
        4,
    )?;
    stream_query_digest(
        conn,
        &mut digest,
        "market_sessions",
        "SELECT market, date, is_session, source, updated_at FROM stock_market_sessions
         WHERE (json_array_length(?1) = 0 OR market IN (SELECT value FROM json_each(?1)))
           AND date BETWEEN ?2 AND ?3
         ORDER BY market, date",
        vec![
            SqlValue::Text(markets.clone()),
            SqlValue::Text(date_text(scope.price_start)),
            SqlValue::Text(date_text(scope.evaluation_end)),
        ],
        5,
    )?;
    stream_query_digest(
        conn,
        &mut digest,
        "calendar_coverage",
        "SELECT market, source, complete_start, complete_through, revision,
                encodes_closed_dates, updated_at
         FROM stock_market_calendar_coverage
         WHERE (json_array_length(?1) = 0 OR market IN (SELECT value FROM json_each(?1)))
         ORDER BY market",
        vec![SqlValue::Text(markets)],
        7,
    )?;
    stream_query_digest(
        conn,
        &mut digest,
        "cached_fx",
        "SELECT id, usd_cny, usd_hkd, cny_hkd, updated_at
         FROM cached_exchange_rates ORDER BY id",
        vec![],
        5,
    )?;
    Ok(digest.finish())
}

#[derive(Debug, Clone, Copy)]
struct StableDigest(u64);

impl StableDigest {
    fn new(domain: &str) -> Self {
        let mut digest = Self(0xcbf29ce484222325);
        digest.write_frame(0x01, domain.as_bytes());
        digest
    }

    fn write_raw(&mut self, bytes: &[u8]) {
        for byte in bytes {
            self.0 ^= u64::from(*byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    fn write_frame(&mut self, tag: u8, payload: &[u8]) {
        self.write_raw(&[tag]);
        self.write_raw(&(payload.len() as u64).to_le_bytes());
        self.write_raw(payload);
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        self.write_frame(0x02, bytes);
    }

    fn write_value(&mut self, column_index: usize, value: ValueRef<'_>) {
        self.write_frame(0x13, &(column_index as u64).to_le_bytes());
        match value {
            ValueRef::Null => self.write_frame(0x20, &[]),
            ValueRef::Integer(value) => self.write_frame(0x21, &value.to_le_bytes()),
            ValueRef::Real(value) => self.write_frame(0x22, &value.to_bits().to_le_bytes()),
            ValueRef::Text(value) => self.write_frame(0x23, value),
            ValueRef::Blob(value) => self.write_frame(0x24, value),
        }
    }

    fn finish(self) -> String {
        format!("{:016x}", self.0)
    }
}

fn stream_query_digest(
    conn: &Connection,
    digest: &mut StableDigest,
    label: &str,
    sql: &str,
    values: Vec<SqlValue>,
    column_count: usize,
) -> Result<(), String> {
    digest.write_frame(0x10, label.as_bytes());
    let mut statement = conn.prepare(sql).map_err(|error| error.to_string())?;
    let mut rows = statement
        .query(params_from_iter(values.iter()))
        .map_err(|error| error.to_string())?;
    let mut row_index = 0_u64;
    while let Some(row) = rows.next().map_err(|error| error.to_string())? {
        digest.write_frame(0x11, &row_index.to_le_bytes());
        digest.write_frame(0x12, &(column_count as u64).to_le_bytes());
        for index in 0..column_count {
            digest.write_value(
                index,
                row.get_ref(index).map_err(|error| error.to_string())?,
            );
        }
        digest.write_frame(0x14, &[]);
        row_index += 1;
    }
    Ok(())
}

fn digest_scope(digest: &mut StableDigest, scope: &CandidateRevisionScope) -> Result<(), String> {
    for date in [
        scope.report_start,
        scope.report_end,
        scope.price_start,
        scope.evaluation_end,
        scope.current_horizon,
        scope.display_cutoff,
    ] {
        digest.write_bytes(date_text(date).as_bytes());
    }
    digest.write_bytes(json_string_list(&scope.account_ids)?.as_bytes());
    digest.write_bytes(json_string_list(&scope.markets)?.as_bytes());
    digest.write_bytes(
        serde_json::to_string(&scope.securities)
            .map_err(|error| error.to_string())?
            .as_bytes(),
    );
    digest.write_bytes(json_string_list(&scope.benchmark_symbols)?.as_bytes());
    digest.write_bytes(json_string_list(&scope.currencies)?.as_bytes());
    Ok(())
}

fn json_string_list(values: &[String]) -> Result<String, String> {
    serde_json::to_string(values).map_err(|error| error.to_string())
}

fn date_text(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn sorted_unique<T: Ord>(values: impl Iterator<Item = T>) -> Vec<T> {
    let mut values = values.collect::<Vec<_>>();
    values.sort();
    values.dedup();
    values
}

pub fn list_overrides(db: &Database) -> Result<StockReviewOverrideList, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    list_overrides_from_connection(&conn, None)
}

pub fn list_overrides_for_query(
    db: &Database,
    query: &StockReviewQuery,
    current_horizon: NaiveDate,
) -> Result<StockReviewOverrideList, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let scope = CandidateRevisionScope {
        report_start: query.start_date,
        report_end: query.end_date,
        price_start: query.start_date,
        evaluation_end: query.end_date,
        current_horizon,
        display_cutoff: query.end_date,
        account_ids: query.account_id.clone().into_iter().collect(),
        markets: query.market.clone().into_iter().collect(),
        securities: Vec::new(),
        benchmark_symbols: Vec::new(),
        currencies: vec![query.base_currency.clone()],
    };
    list_overrides_from_connection(&conn, Some(&scope))
}

fn list_overrides_from_connection(
    conn: &Connection,
    scope: Option<&CandidateRevisionScope>,
) -> Result<StockReviewOverrideList, String> {
    let mut result = StockReviewOverrideList {
        overrides: Vec::new(),
        stale_overrides: Vec::new(),
        issues: Vec::new(),
    };
    let mut snapshots = scoped_override_snapshots(conn, scope)?;
    snapshots.sort_by(|left, right| {
        left.record
            .created_at
            .cmp(&right.record.created_at)
            .then_with(|| left.record.id.cmp(&right.record.id))
    });
    for snapshot in snapshots {
        if snapshot.is_current {
            result.overrides.push(snapshot.record);
        } else {
            result.issues.push(stale_override_issue(&snapshot.record));
            result.stale_overrides.push(snapshot.record);
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
    annotation_economic_dates(&input.value_json)?;
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

pub(crate) fn annotation_economic_dates(
    value_json: &str,
) -> Result<AnnotationEconomicDates, String> {
    let value: Value = serde_json::from_str(value_json)
        .map_err(|error| format!("Annotation value_json must be valid JSON: {error}"))?;
    let object = value
        .as_object()
        .ok_or_else(|| "Annotation value_json must be a JSON object.".to_string())?;
    let parse_date = |key: &str| -> Result<Option<NaiveDate>, String> {
        let Some(value) = object.get(key) else {
            return Ok(None);
        };
        let text = value
            .as_str()
            .ok_or_else(|| format!("Annotation {key} must be a YYYY-MM-DD string."))?;
        let date = NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .map_err(|_| format!("Annotation {key} must be a valid YYYY-MM-DD date."))?;
        if date.format("%Y-%m-%d").to_string() != text {
            return Err(format!(
                "Annotation {key} must use exact YYYY-MM-DD formatting."
            ));
        }
        Ok(Some(date))
    };
    let dates = AnnotationEconomicDates {
        effective_date: parse_date("effective_date")?,
        effective_start: parse_date("effective_start")?,
        effective_end: parse_date("effective_end")?,
        snapshot_date: parse_date("snapshot_date")?,
    };
    if dates
        .effective_start
        .zip(dates.effective_end)
        .is_some_and(|(start, end)| start > end)
    {
        return Err("Annotation effective_start must be on or before effective_end.".to_string());
    }
    Ok(dates)
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
            && transaction
                .market
                .trim()
                .eq_ignore_ascii_case(first.market.trim())
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
        same_day_reversal_ids(conn, &first.account_id, &first.symbol, &first.market, date)?;
    if !same_position_day
        || !has_reversal
        || full_database_order_set.len() != input_ids.len()
        || !full_database_order_set
            .iter()
            .all(|id| input_ids.contains(id))
    {
        issues.push(validation_issue(
            "invalid_same_day_order",
            "Same-day order must contain the complete ordered BUY/SELL reversal set for one account, normalized symbol, normalized market, and trade date.",
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
    Ok(load_transaction_references(conn, ids)?
        .into_iter()
        .flatten()
        .collect())
}

fn load_transaction_references(
    conn: &Connection,
    ids: &[String],
) -> Result<Vec<Option<StoredTransaction>>, String> {
    ids.iter()
        .map(|id| {
            conn.query_row(
                "SELECT id, holding_id, account_id, symbol, name, market, transaction_type,
                        shares, price, total_amount, commission, currency, traded_at, notes, created_at
                 FROM transactions WHERE id = ?1",
                params![id],
                |row| {
                    Ok(StoredTransaction {
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
            .optional()
            .map_err(|error| error.to_string())
        })
        .collect()
}

fn same_day_reversal_ids(
    conn: &Connection,
    account_id: &str,
    symbol: &str,
    market: &str,
    date: chrono::NaiveDate,
) -> Result<Vec<String>, String> {
    let mut statement = conn
        .prepare(
            "SELECT id FROM transactions
             WHERE account_id = ?1
               AND UPPER(TRIM(symbol)) = UPPER(TRIM(?2))
               AND UPPER(TRIM(market)) = UPPER(TRIM(?3))
               AND substr(traded_at, 1, 10) = ?4
               AND transaction_type IN ('BUY', 'SELL')
             ORDER BY id ASC",
        )
        .map_err(|error| error.to_string())?;
    let ids = statement
        .query_map(
            params![
                account_id,
                symbol,
                market,
                date.format("%Y-%m-%d").to_string()
            ],
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
        list_annotations, list_overrides, list_overrides_for_query, monotonic_audit_timestamp,
        prepare_override_candidate, save_annotation, save_override, save_override_candidate,
        scope_candidate_to_query, set_candidate_revision_scope, validate_override,
        AnnotationSaveContext, CandidateRevisionScope, StockReviewAnnotationFilter,
    };
    use crate::db::Database;
    use crate::models::stock_review::{
        StockReviewAnnotationInput, StockReviewIssueSeverity, StockReviewOverrideInput,
        StockReviewQuery,
    };
    use chrono::NaiveDate;
    use rusqlite::{params, Connection};

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
    fn post_async_refresh_must_not_bless_an_in_scope_user_mutation() {
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
        let mut candidate = prepare_override_candidate(
            &db,
            override_input("candidate", "non_trade", &["buy"], "{}"),
        )
        .unwrap();
        db.conn.lock().unwrap().execute(
            "INSERT INTO holdings (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
             VALUES ('concurrent', 'acct-a', 'MSFT', 'MSFT', 'US', 1, 100, 'USD', '2024-02-02', '2024-02-02')",
            [],
        ).unwrap();

        assert!(
            super::pin_candidate_source_revision_after_cache_fill(&db, &mut candidate).is_err()
        );
        assert!(save_override_candidate(&db, candidate).is_err());
        assert!(list_overrides(&db).unwrap().overrides.is_empty());
    }

    #[test]
    fn quarterly_context_mutation_invalidates_a_prepared_candidate() {
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
                "INSERT INTO quarterly_snapshots (id, quarter, snapshot_date, created_at)
             VALUES ('q1', '2024Q1', '2024-03-31', '2024-03-31')",
                [],
            )
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO quarterly_holding_snapshots
                (id, quarterly_snapshot_id, account_id, symbol, name, market, notes)
             VALUES ('qh1', 'q1', 'acct-a', 'AAPL', 'AAPL', 'US', 'changed')",
                [],
            )
            .unwrap();

        assert!(save_override_candidate(&db, candidate).is_err());
        assert!(list_overrides(&db).unwrap().overrides.is_empty());
    }

    #[test]
    fn unrelated_account_mutation_does_not_invalidate_scoped_candidate() {
        let db = database();
        insert_transaction(
            &db,
            "buy-a",
            "acct-a",
            "AAPL",
            "BUY",
            1.0,
            100.0,
            "2024-02-01",
        );
        let candidate = prepare_override_candidate(
            &db,
            override_input("candidate", "non_trade", &["buy-a"], "{}"),
        )
        .unwrap();
        insert_transaction(
            &db,
            "buy-b",
            "acct-b",
            "MSFT",
            "BUY",
            1.0,
            200.0,
            "2024-02-02",
        );

        assert!(save_override_candidate(&db, candidate).is_ok());
        assert_eq!(list_overrides(&db).unwrap().overrides.len(), 1);
    }

    #[test]
    fn unrelated_account_override_does_not_invalidate_scoped_candidate() {
        let db = database();
        insert_transaction(
            &db,
            "buy-a",
            "acct-a",
            "AAPL",
            "BUY",
            1.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            &db,
            "buy-b",
            "acct-b",
            "MSFT",
            "BUY",
            1.0,
            200.0,
            "2024-02-01",
        );
        save_override(&db, override_input("other", "non_trade", &["buy-b"], "{}")).unwrap();
        let mut candidate = prepare_override_candidate(
            &db,
            override_input("candidate", "non_trade", &["buy-a"], "{}"),
        )
        .unwrap();
        scope_candidate_to_query(
            &db,
            &mut candidate,
            &StockReviewQuery {
                start_date: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
                end_date: NaiveDate::from_ymd_opt(2024, 2, 2).unwrap(),
                account_id: Some("acct-a".to_string()),
                market: Some("US".to_string()),
                benchmark_symbol: None,
                base_currency: "USD".to_string(),
            },
        )
        .unwrap();
        save_override(
            &db,
            override_input("other", "non_trade", &["buy-b"], r#"{"review":"changed"}"#),
        )
        .unwrap();

        assert!(save_override_candidate(&db, candidate).is_ok());
        assert_eq!(list_overrides(&db).unwrap().overrides.len(), 2);
    }

    #[test]
    fn unrelated_symbol_market_pair_does_not_invalidate_scoped_candidate() {
        let db = database();
        insert_transaction(
            &db,
            "buy-a",
            "acct-a",
            "AAPL",
            "BUY",
            1.0,
            100.0,
            "2024-02-01",
        );
        let mut candidate = prepare_override_candidate(
            &db,
            override_input("candidate", "non_trade", &["buy-a"], "{}"),
        )
        .unwrap();
        set_candidate_revision_scope(
            &mut candidate,
            CandidateRevisionScope {
                report_start: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
                report_end: NaiveDate::from_ymd_opt(2024, 2, 2).unwrap(),
                price_start: NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
                evaluation_end: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
                current_horizon: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
                display_cutoff: NaiveDate::from_ymd_opt(2024, 2, 2).unwrap(),
                account_ids: vec!["acct-a".to_string()],
                markets: vec!["CN".to_string(), "US".to_string()],
                securities: vec![
                    ("600000".to_string(), "CN".to_string()),
                    ("AAPL".to_string(), "US".to_string()),
                ],
                benchmark_symbols: vec![],
                currencies: vec!["CNY".to_string(), "USD".to_string()],
            },
        );
        super::pin_candidate_source_revision_after_cache_fill(&db, &mut candidate).unwrap();

        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO stock_daily_prices
                    (symbol, market, date, close, source, updated_at)
                 VALUES ('AAPL', 'CN', '2024-02-02', 100, 'unrelated', '2024-02-02')",
                [],
            )
            .unwrap();

        assert!(save_override_candidate(&db, candidate).is_ok());
    }

    #[test]
    fn historical_snapshot_symbol_outside_discovery_still_invalidates_candidate() {
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
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO daily_holding_snapshots
                    (date, account_id, symbol, market, shares, avg_cost, close_price, market_value)
                 VALUES ('2024-02-02', 'acct-a', 'MSFT', 'US', 1, 200, 200, 200)",
                [],
            )
            .unwrap();
        let mut candidate = prepare_override_candidate(
            &db,
            override_input("candidate", "non_trade", &["buy"], "{}"),
        )
        .unwrap();
        set_candidate_revision_scope(
            &mut candidate,
            CandidateRevisionScope {
                report_start: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
                report_end: NaiveDate::from_ymd_opt(2024, 2, 2).unwrap(),
                price_start: NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
                evaluation_end: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
                current_horizon: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
                display_cutoff: NaiveDate::from_ymd_opt(2024, 2, 2).unwrap(),
                account_ids: vec!["acct-a".to_string()],
                markets: vec!["US".to_string()],
                securities: vec![("AAPL".to_string(), "US".to_string())],
                benchmark_symbols: vec![],
                currencies: vec!["USD".to_string()],
            },
        );
        super::pin_candidate_source_revision_after_cache_fill(&db, &mut candidate).unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE daily_holding_snapshots SET market_value = 250
                 WHERE account_id = 'acct-a' AND symbol = 'MSFT'",
                [],
            )
            .unwrap();

        assert!(save_override_candidate(&db, candidate).is_err());
        assert!(list_overrides(&db).unwrap().overrides.is_empty());
    }

    #[test]
    fn normalized_split_symbol_mutation_invalidates_candidate() {
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
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO stock_splits
                    (stock_code, split_date, ratio_from, ratio_to, created_at)
                 VALUES (' aapl ', '2024-02-02', 1, 2, '2024-02-02')",
                [],
            )
            .unwrap();
        let mut candidate = prepare_override_candidate(
            &db,
            override_input("candidate", "non_trade", &["buy"], "{}"),
        )
        .unwrap();
        set_candidate_revision_scope(
            &mut candidate,
            CandidateRevisionScope {
                report_start: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
                report_end: NaiveDate::from_ymd_opt(2024, 2, 2).unwrap(),
                price_start: NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
                evaluation_end: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
                current_horizon: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
                display_cutoff: NaiveDate::from_ymd_opt(2024, 2, 2).unwrap(),
                account_ids: vec!["acct-a".to_string()],
                markets: vec!["US".to_string()],
                securities: vec![("AAPL".to_string(), "US".to_string())],
                benchmark_symbols: vec![],
                currencies: vec!["USD".to_string()],
            },
        );
        super::pin_candidate_source_revision_after_cache_fill(&db, &mut candidate).unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE stock_splits SET ratio_to = 3 WHERE stock_code = ' aapl '",
                [],
            )
            .unwrap();

        assert!(save_override_candidate(&db, candidate).is_err());
        assert!(list_overrides(&db).unwrap().overrides.is_empty());
    }

    #[test]
    fn annotation_rejects_invalid_economic_dates_without_writing() {
        let db = database();
        let invalid_values = [
            r#"{"effective_date":"2024-02-30"}"#,
            r#"{"effective_start":"2024-2-01"}"#,
            r#"{"effective_end":42}"#,
            r#"{"snapshot_date":"not-a-date"}"#,
            r#"{"effective_start":"2024-03-01","effective_end":"2024-02-01"}"#,
        ];
        for (index, value_json) in invalid_values.into_iter().enumerate() {
            let mut input = annotation_input(&format!("invalid-date-{index}"), value_json);
            input.value_json = value_json.to_string();
            assert!(
                save_annotation(&db, input, AnnotationSaveContext::UserInitiated).is_err(),
                "invalid annotation date fixture {index} was accepted"
            );
        }
        let count = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM stock_review_annotations", [], |row| {
                row.get::<_, i64>(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn future_annotation_mutation_is_part_of_candidate_revision() {
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
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO stock_review_annotations
                    (id, scope_type, scope_key, account_id, symbol, annotation_type,
                     value_json, source, created_at, updated_at)
                 VALUES ('future', 'stock', 'AAPL', 'acct-a', 'AAPL', 'thesis',
                         '{\"effective_date\":\"2024-03-01\",\"note\":\"first\"}',
                         'user', '2024-02-01', '2024-02-01')",
                [],
            )
            .unwrap();
        let mut candidate = prepare_override_candidate(
            &db,
            override_input("candidate", "non_trade", &["buy"], "{}"),
        )
        .unwrap();
        set_candidate_revision_scope(
            &mut candidate,
            CandidateRevisionScope {
                report_start: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
                report_end: NaiveDate::from_ymd_opt(2024, 2, 2).unwrap(),
                price_start: NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
                evaluation_end: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
                current_horizon: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
                display_cutoff: NaiveDate::from_ymd_opt(2024, 2, 2).unwrap(),
                account_ids: vec!["acct-a".to_string()],
                markets: vec!["US".to_string()],
                securities: vec![("AAPL".to_string(), "US".to_string())],
                benchmark_symbols: vec![],
                currencies: vec!["USD".to_string()],
            },
        );
        super::pin_candidate_source_revision_after_cache_fill(&db, &mut candidate).unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE stock_review_annotations
                 SET value_json = '{\"effective_date\":\"2024-03-01\",\"note\":\"changed\"}'
                 WHERE id = 'future'",
                [],
            )
            .unwrap();

        assert!(save_override_candidate(&db, candidate).is_err());
    }

    fn query_digest(sql: &str, column_count: usize) -> String {
        let conn = Connection::open_in_memory().unwrap();
        let mut digest = super::StableDigest::new("structural-test");
        super::stream_query_digest(&conn, &mut digest, "fixture", sql, vec![], column_count)
            .unwrap();
        digest.finish()
    }

    #[test]
    fn digest_distinguishes_null_from_text_null() {
        assert_ne!(
            query_digest("SELECT NULL", 1),
            query_digest("SELECT 'null'", 1)
        );
    }

    #[test]
    fn digest_distinguishes_text_from_blob_with_the_same_bytes() {
        assert_ne!(
            query_digest("SELECT 'same'", 1),
            query_digest("SELECT CAST('same' AS BLOB)", 1)
        );
    }

    #[test]
    fn digest_distinguishes_integer_from_real() {
        assert_ne!(
            query_digest("SELECT 1", 1),
            query_digest("SELECT CAST(1 AS REAL)", 1)
        );
    }

    #[test]
    fn digest_distinguishes_multi_column_boundaries() {
        assert_ne!(
            query_digest("SELECT 'ab', 'c'", 2),
            query_digest("SELECT 'a', 'bc'", 2)
        );
    }

    #[test]
    fn candidate_revisions_are_compact_streaming_digests() {
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

        assert_eq!(candidate.active_override_revision.len(), 16);
        assert_eq!(candidate.preparation_user_revision.len(), 16);
        assert_eq!(candidate.review_source_revision.user.len(), 16);
        assert_eq!(candidate.review_source_revision.cache.len(), 16);
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

    #[test]
    fn scoped_override_list_excludes_unrelated_stale_rows_and_issues() {
        let db = database();
        insert_transaction(
            &db,
            "buy-a",
            "acct-a",
            "AAPL",
            "BUY",
            1.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            &db,
            "buy-b",
            "acct-b",
            "MSFT",
            "BUY",
            1.0,
            200.0,
            "2024-02-01",
        );
        save_override(
            &db,
            override_input("stale-a", "non_trade", &["buy-a"], "{}"),
        )
        .unwrap();
        save_override(
            &db,
            override_input("stale-b", "non_trade", &["buy-b"], "{}"),
        )
        .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE transactions SET price = price + 1, total_amount = total_amount + 1",
                [],
            )
            .unwrap();
        let query = StockReviewQuery {
            start_date: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2024, 2, 2).unwrap(),
            account_id: Some("acct-a".to_string()),
            market: Some("US".to_string()),
            benchmark_symbol: None,
            base_currency: "USD".to_string(),
        };

        let result =
            list_overrides_for_query(&db, &query, NaiveDate::from_ymd_opt(2024, 3, 1).unwrap())
                .unwrap();
        assert!(result.overrides.is_empty());
        assert_eq!(result.stale_overrides.len(), 1);
        assert_eq!(result.stale_overrides[0].id, "stale-a");
        assert_eq!(result.issues.len(), 1);
        assert_eq!(result.issues[0].code, "stale_override");
        assert!(result.issues[0].message.contains("stale-a"));
    }

    fn scoped_query(account_id: &str) -> StockReviewQuery {
        StockReviewQuery {
            start_date: NaiveDate::from_ymd_opt(2024, 2, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2024, 2, 2).unwrap(),
            account_id: Some(account_id.to_string()),
            market: Some("US".to_string()),
            benchmark_symbol: None,
            base_currency: "USD".to_string(),
        }
    }

    fn save_cross_account_transfer(db: &Database, id: &str) {
        insert_transaction(
            db,
            &format!("{id}-source"),
            "acct-a",
            "AAPL",
            "SELL",
            5.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            db,
            &format!("{id}-destination"),
            "acct-b",
            "AAPL",
            "BUY",
            5.0,
            100.0,
            "2024-02-01",
        );
        save_override(
            db,
            override_input(
                id,
                "transfer",
                &[&format!("{id}-source"), &format!("{id}-destination")],
                "{}",
            ),
        )
        .unwrap();
    }

    #[test]
    fn scoped_candidate_rejects_cross_account_reference_mutation() {
        // Omitting out-of-scope transfer legs from the scoped revision would
        // let a report for acct-a save against a transfer state it never built.
        let db = database();
        save_cross_account_transfer(&db, "book-transfer");
        insert_transaction(
            &db,
            "candidate-row",
            "acct-a",
            "MSFT",
            "OPEN",
            1.0,
            200.0,
            "2024-02-02",
        );
        let mut candidate = prepare_override_candidate(
            &db,
            override_input("candidate", "non_trade", &["candidate-row"], "{}"),
        )
        .unwrap();
        scope_candidate_to_query(&db, &mut candidate, &scoped_query("acct-a")).unwrap();

        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE transactions SET shares = 4, total_amount = 400
                 WHERE id = 'book-transfer-destination'",
                [],
            )
            .unwrap();

        assert!(save_override_candidate(&db, candidate).is_err());
        assert!(!list_overrides(&db)
            .unwrap()
            .overrides
            .iter()
            .any(|record| record.id == "candidate"));
    }

    #[test]
    fn scoped_revision_keeps_original_reference_ids_after_override_row_drift() {
        // The revision set is the union of the persisted confirmation IDs and
        // the mutable override row IDs. Otherwise an altered override row can
        // orphan an original cross-account leg from candidate coherence.
        let db = database();
        save_cross_account_transfer(&db, "book-transfer");
        insert_transaction(
            &db,
            "replacement-reference",
            "acct-a",
            "MSFT",
            "OPEN",
            1.0,
            200.0,
            "2024-02-02",
        );
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE stock_review_overrides
                 SET transaction_ids_json = '[\"replacement-reference\"]'
                 WHERE id = 'book-transfer'",
                [],
            )
            .unwrap();
        let mut candidate = prepare_override_candidate(
            &db,
            override_input("candidate", "non_trade", &["replacement-reference"], "{}"),
        )
        .unwrap();
        scope_candidate_to_query(&db, &mut candidate, &scoped_query("acct-a")).unwrap();

        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE transactions SET shares = 4, total_amount = 400
                 WHERE id = 'book-transfer-destination'",
                [],
            )
            .unwrap();

        assert!(save_override_candidate(&db, candidate).is_err());
    }

    #[test]
    fn deleting_cross_account_leg_changes_revision_and_keeps_scoped_stale_issue() {
        // A missing referenced row must be represented by a tombstone rather
        // than falling out of both relevance selection and the digest.
        let db = database();
        save_cross_account_transfer(&db, "book-transfer");
        let query = scoped_query("acct-a");
        let scope = CandidateRevisionScope {
            report_start: query.start_date,
            report_end: query.end_date,
            price_start: query.start_date,
            evaluation_end: query.end_date,
            current_horizon: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
            display_cutoff: query.end_date,
            account_ids: vec!["acct-a".to_string()],
            markets: vec!["US".to_string()],
            ..CandidateRevisionScope::default()
        };
        let before = super::active_override_revision(&db.conn.lock().unwrap(), &scope).unwrap();

        db.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM transactions WHERE id = 'book-transfer-destination'",
                [],
            )
            .unwrap();

        let after = super::active_override_revision(&db.conn.lock().unwrap(), &scope).unwrap();
        assert_ne!(before, after);
        let listed =
            list_overrides_for_query(&db, &query, NaiveDate::from_ymd_opt(2024, 3, 1).unwrap())
                .unwrap();
        assert!(listed.overrides.is_empty());
        assert_eq!(listed.stale_overrides.len(), 1);
        assert_eq!(listed.stale_overrides[0].id, "book-transfer");
        assert_eq!(listed.issues.len(), 1);
        assert_eq!(listed.issues[0].code, "stale_override");
    }

    #[test]
    fn original_reference_scope_keeps_reaccounted_source_leg_query_relevant() {
        // Relevance cannot be recomputed solely from mutable current rows:
        // moving the last acct-a leg must surface a stale audit issue in acct-a.
        let db = database();
        save_cross_account_transfer(&db, "book-transfer");
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE transactions SET account_id = 'acct-b'
                 WHERE id = 'book-transfer-source'",
                [],
            )
            .unwrap();

        let listed = list_overrides_for_query(
            &db,
            &scoped_query("acct-a"),
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        )
        .unwrap();
        assert!(listed.overrides.is_empty());
        assert_eq!(listed.stale_overrides.len(), 1);
        assert_eq!(listed.stale_overrides[0].id, "book-transfer");
    }

    #[test]
    fn unrelated_original_and_current_override_scope_stays_excluded() {
        let db = database();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
                 VALUES ('acct-c', 'Account C', 'US', '2024-01-01', '2024-01-01')",
                [],
            )
            .unwrap();
        insert_transaction(
            &db,
            "bc-source",
            "acct-b",
            "AAPL",
            "SELL",
            5.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            &db,
            "bc-destination",
            "acct-c",
            "AAPL",
            "BUY",
            5.0,
            100.0,
            "2024-02-01",
        );
        save_override(
            &db,
            override_input(
                "unrelated-transfer",
                "transfer",
                &["bc-source", "bc-destination"],
                "{}",
            ),
        )
        .unwrap();

        let listed = list_overrides_for_query(
            &db,
            &scoped_query("acct-a"),
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        )
        .unwrap();
        assert!(listed.overrides.is_empty());
        assert!(listed.stale_overrides.is_empty());
        assert!(listed.issues.is_empty());
    }

    #[test]
    fn ambiguous_legacy_reference_fingerprint_is_audit_only() {
        // Version-1 fingerprints had no market identity. They must remain
        // visible for audit but must never silently authorize replay.
        let db = database();
        insert_transaction(
            &db,
            "buy-a",
            "acct-a",
            "AAPL",
            "BUY",
            1.0,
            100.0,
            "2024-02-01",
        );
        save_override(&db, override_input("legacy", "non_trade", &["buy-a"], "{}")).unwrap();
        let legacy = serde_json::json!([{
            "id": "buy-a",
            "account_id": "acct-a",
            "symbol": "AAPL",
            "transaction_type": "BUY",
            "shares": 1.0,
            "price": 100.0,
            "total_amount": 100.0,
            "commission": 0.0,
            "currency": "USD",
            "traded_at": "2024-02-01"
        }]);
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE stock_review_overrides SET reference_fingerprint_json = ?1
                 WHERE id = 'legacy'",
                params![legacy.to_string()],
            )
            .unwrap();

        let listed = list_overrides_for_query(
            &db,
            &scoped_query("acct-a"),
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        )
        .unwrap();
        assert!(listed.overrides.is_empty());
        assert_eq!(listed.stale_overrides.len(), 1);
        assert_eq!(listed.stale_overrides[0].id, "legacy");
        assert_eq!(listed.issues[0].code, "stale_override");
    }

    #[test]
    fn empty_migrated_reference_snapshot_remains_conservatively_audit_visible() {
        // The migration default `[]` has neither account nor market authority.
        // Once its current row is deleted it must not silently disappear.
        let db = database();
        insert_transaction(
            &db,
            "buy-a",
            "acct-a",
            "AAPL",
            "BUY",
            1.0,
            100.0,
            "2024-02-01",
        );
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO stock_review_overrides
                    (id, override_type, transaction_ids_json, value_json,
                     reference_fingerprint_json, created_at, updated_at)
                 VALUES ('migrated', 'non_trade', '[\"buy-a\"]', '{}', '[]',
                         '2024-02-01', '2024-02-01')",
                [],
            )
            .unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM transactions WHERE id = 'buy-a'", [])
            .unwrap();

        let listed = list_overrides_for_query(
            &db,
            &scoped_query("acct-a"),
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        )
        .unwrap();
        assert_eq!(listed.stale_overrides.len(), 1);
        assert_eq!(listed.stale_overrides[0].id, "migrated");
        assert_eq!(listed.issues[0].code, "stale_override");
    }

    #[test]
    fn holding_snapshot_identity_is_part_of_candidate_revision() {
        // Duplicate logical snapshot keys are legal, so row identity must be
        // both hashed and used as the final ordering key.
        let db = database();
        insert_transaction(
            &db,
            "buy-a",
            "acct-a",
            "AAPL",
            "BUY",
            1.0,
            100.0,
            "2024-02-01",
        );
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO daily_holding_snapshots
                    (id, date, account_id, symbol, market, shares, avg_cost, close_price, market_value)
                 VALUES (10, '2024-02-01', 'acct-a', 'AAPL', 'US', 1, 100, 100, 100)",
                [],
            )
            .unwrap();
        let mut candidate = prepare_override_candidate(
            &db,
            override_input("candidate", "non_trade", &["buy-a"], "{}"),
        )
        .unwrap();
        scope_candidate_to_query(&db, &mut candidate, &scoped_query("acct-a")).unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE daily_holding_snapshots SET id = 20 WHERE id = 10",
                [],
            )
            .unwrap();

        assert!(save_override_candidate(&db, candidate).is_err());
    }

    fn save_us_same_day_order(db: &Database) {
        insert_transaction(
            db,
            "order-buy",
            "acct-a",
            "AAPL",
            "BUY",
            2.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            db,
            "order-sell",
            "acct-a",
            "aapl",
            "SELL",
            2.0,
            100.0,
            "2024-02-01",
        );
        save_override(
            db,
            override_input(
                "us-order",
                "same_day_order",
                &["order-buy", "order-sell"],
                r#"["order-buy","order-sell"]"#,
            ),
        )
        .unwrap();
    }

    fn prepared_us_candidate(db: &Database) -> super::ValidatedOverrideCandidate {
        insert_transaction(
            db,
            "candidate-row",
            "acct-a",
            "MSFT",
            "OPEN",
            1.0,
            200.0,
            "2024-02-02",
        );
        let mut candidate = prepare_override_candidate(
            db,
            override_input("candidate", "non_trade", &["candidate-row"], "{}"),
        )
        .unwrap();
        scope_candidate_to_query(db, &mut candidate, &scoped_query("acct-a")).unwrap();
        candidate
    }

    #[test]
    fn cross_market_same_day_reversals_do_not_stale_us_order_or_candidate() {
        // Removing market from position identity makes an unrelated CN order
        // silently flip a confirmed US ordering correction to stale.
        let db = database();
        save_us_same_day_order(&db);
        let candidate = prepared_us_candidate(&db);
        insert_transaction(
            &db,
            "cn-buy",
            "acct-a",
            "AAPL",
            "BUY",
            1.0,
            50.0,
            "2024-02-01",
        );
        insert_transaction(
            &db,
            "cn-sell",
            "acct-a",
            "AAPL",
            "SELL",
            1.0,
            50.0,
            "2024-02-01",
        );
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE transactions SET market = 'CN', currency = 'CNY'
                 WHERE id IN ('cn-buy', 'cn-sell')",
                [],
            )
            .unwrap();

        assert!(save_override_candidate(&db, candidate).is_ok());
        let listed = list_overrides_for_query(
            &db,
            &scoped_query("acct-a"),
            NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
        )
        .unwrap();
        assert_eq!(listed.overrides.len(), 2);
        assert!(listed
            .overrides
            .iter()
            .any(|record| record.id == "us-order"));
        assert!(listed.stale_overrides.is_empty());
    }

    #[test]
    fn same_market_extra_reversal_changes_override_revision_and_rejects_candidate() {
        // The active override digest must frame derived validation/currentness;
        // otherwise a complete-set change can flip active to stale invisibly.
        let db = database();
        save_us_same_day_order(&db);
        let candidate = prepared_us_candidate(&db);
        let query = scoped_query("acct-a");
        let scope = CandidateRevisionScope {
            report_start: query.start_date,
            report_end: query.end_date,
            price_start: query.start_date,
            evaluation_end: query.end_date,
            current_horizon: NaiveDate::from_ymd_opt(2024, 3, 1).unwrap(),
            display_cutoff: query.end_date,
            account_ids: vec!["acct-a".to_string()],
            markets: vec!["US".to_string()],
            ..CandidateRevisionScope::default()
        };
        let before = super::active_override_revision(&db.conn.lock().unwrap(), &scope).unwrap();
        insert_transaction(
            &db,
            "extra-buy",
            "acct-a",
            "AAPL",
            "BUY",
            1.0,
            100.0,
            "2024-02-01",
        );
        insert_transaction(
            &db,
            "extra-sell",
            "acct-a",
            "AAPL",
            "SELL",
            1.0,
            100.0,
            "2024-02-01",
        );

        let after = super::active_override_revision(&db.conn.lock().unwrap(), &scope).unwrap();
        assert_ne!(before, after);
        assert!(save_override_candidate(&db, candidate).is_err());
        let listed =
            list_overrides_for_query(&db, &query, NaiveDate::from_ymd_opt(2024, 3, 1).unwrap())
                .unwrap();
        assert!(listed.overrides.is_empty());
        assert_eq!(listed.stale_overrides.len(), 1);
        assert_eq!(listed.stale_overrides[0].id, "us-order");
    }
}
