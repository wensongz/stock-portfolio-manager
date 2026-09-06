use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBatchRowInput {
    pub key: String,
    pub raw: Value,
    pub external_id: Option<String>,
    pub data: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBatchRequest {
    pub request_id: String,
    pub account_id: String,
    pub source: String,
    pub file_name: String,
    pub source_content: String,
    pub parser_version: String,
    pub kind: String,
    pub rows: Vec<ImportBatchRowInput>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBatchRow {
    pub key: String,
    pub raw: Value,
    pub external_id: Option<String>,
    pub data: Value,
    pub status: String,
    pub error: Option<String>,
    pub record_id: Option<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReconciliationRow {
    pub symbol: String,
    pub currency: String,
    pub before_shares: f64,
    pub after_shares: f64,
    pub expected_shares: Option<f64>,
    pub difference: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExpectedBalance {
    pub symbol: String,
    pub expected_shares: f64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportBatch {
    pub id: String,
    pub account_id: String,
    pub source: String,
    pub file_name: String,
    pub parser_version: String,
    pub kind: String,
    pub status: String,
    pub created_at: String,
    pub rows: Vec<ImportBatchRow>,
    pub reconciliation: Vec<ReconciliationRow>,
    pub can_undo: bool,
    pub conflict: Option<String>,
}
