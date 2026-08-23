use serde::{Deserialize, Serialize};

/// Per-account dividend total within one currency group.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountDividend {
    pub account_id: String,
    pub account_name: String,
    pub total: f64,
}

/// One company's dividend across accounts within a single currency group.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DividendRow {
    pub symbol: String,
    pub name: String,
    /// Per-account amounts keyed by account_id (0.0 if that account has none).
    pub per_account: Vec<(String, f64)>,
    pub total: f64,
}

/// Dividend summary for one actual transaction currency (CNY / USD / HKD).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrencyDividend {
    pub currency: String,
    pub accounts: Vec<AccountDividend>,
    pub rows: Vec<DividendRow>,
    pub total: f64,
}

/// Monthly dividend detail used to build the alternative summary views.
/// Amounts remain in the transaction's actual currency.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DividendEntry {
    pub account_id: String,
    pub account_name: String,
    pub account_market: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub currency: String,
    /// Calendar month in YYYYMM form.
    pub month: String,
    pub total: f64,
}

/// Annual dividend analysis: per-currency tables (row = company, column =
/// account) plus a grand total across currencies (raw amounts, not
/// converted — the frontend converts using its exchange-rate store).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DividendAnalysis {
    /// Selected year, or None when aggregating all dividend history.
    pub year: Option<i32>,
    pub currencies: Vec<CurrencyDividend>,
    pub entries: Vec<DividendEntry>,
    /// Sum of each currency group's total. Not a single-currency
    /// figure; the frontend sums converted values for the displayed total.
    pub grand_total: f64,
}
