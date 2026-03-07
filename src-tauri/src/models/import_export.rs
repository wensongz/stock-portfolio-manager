use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExportFilters {
    pub market: Option<String>,
    pub account_id: Option<String>,
    pub category_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportError {
    pub row: usize,
    pub column: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportPreview {
    pub total_rows: usize,
    pub valid_rows: usize,
    pub error_rows: Vec<ImportError>,
    pub preview_data: Vec<serde_json::Value>,
    pub column_mapping: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportData {
    pub data_type: String, // "holdings" | "transactions"
    pub rows: Vec<serde_json::Value>,
    pub column_mapping: HashMap<String, String>,
    pub account_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImportResult {
    pub imported_count: usize,
    pub skipped_count: usize,
    pub errors: Vec<ImportError>,
}
