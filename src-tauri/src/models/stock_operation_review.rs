use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockOperationReviewQuery {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub account_id: Option<String>,
    pub market: Option<String>,
    pub base_currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockOperationReviewReport {
    pub query: StockOperationReviewQuery,
    pub summary: StockOperationReviewSummary,
    pub securities: Vec<StockOperationSecuritySummary>,
    pub actions: Vec<StockOperationEffect>,
    pub data_quality: StockOperationDataQuality,
    pub generated_at: String,
    pub algorithm_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct StockOperationReviewSummary {
    pub total: StockOperationGroupSummary,
    pub buys: StockOperationGroupSummary,
    pub sells: StockOperationGroupSummary,
    pub position_impact: StockPositionImpactSummary,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct StockOperationGroupSummary {
    pub action_count: usize,
    pub positive_count: usize,
    pub negative_count: usize,
    pub missing_effect_count: usize,
    pub price_effect_base: Option<f64>,
    pub positive_notional_ratio: Option<f64>,
    pub weighted_excess_return: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct StockPositionImpactSummary {
    pub invested_amount_base: Option<f64>,
    pub recovered_amount_base: Option<f64>,
    pub largest_absolute_weight_change: Option<f64>,
    pub total_fees_base: Option<f64>,
    pub missing_weight_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockOperationEffect {
    pub action_id: String,
    pub transaction_ids: Vec<String>,
    pub account_id: String,
    pub account_name: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub action_type: String,
    pub trade_date: NaiveDate,
    pub quantity: f64,
    pub trade_price: f64,
    pub trade_notional_local: f64,
    pub trade_notional_base: Option<f64>,
    pub fee_local: f64,
    pub fee_base: Option<f64>,
    pub currency: String,
    pub shares_before: f64,
    pub shares_after: f64,
    pub prior_nav_date: Option<NaiveDate>,
    pub prior_nav_base: Option<f64>,
    pub weight_before: Option<f64>,
    pub weight_after: Option<f64>,
    pub weight_change: Option<f64>,
    pub operation_size_ratio: Option<f64>,
    pub evaluation_date: Option<NaiveDate>,
    pub end_price: Option<f64>,
    pub price_effect_local: Option<f64>,
    pub price_effect_base: Option<f64>,
    pub price_effect_percent: Option<f64>,
    pub benchmark_symbol: Option<String>,
    pub benchmark_start_date: Option<NaiveDate>,
    pub benchmark_end_date: Option<NaiveDate>,
    pub benchmark_return: Option<f64>,
    pub directional_excess_return: Option<f64>,
    pub fact_labels: Vec<String>,
    pub issues: Vec<StockOperationFieldIssue>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockOperationSecuritySummary {
    pub account_id: String,
    pub account_name: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub currency: String,
    pub open_count: usize,
    pub add_count: usize,
    pub reduce_count: usize,
    pub close_count: usize,
    pub net_shares: f64,
    pub buy_notional_local: f64,
    pub sell_notional_local: f64,
    pub price_effect_local: Option<f64>,
    pub price_effect_base: Option<f64>,
    pub weighted_excess_return: Option<f64>,
    pub largest_absolute_weight_change: Option<f64>,
    pub positive_count: usize,
    pub negative_count: usize,
    pub missing_effect_count: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StockOperationFieldIssue {
    pub code: String,
    pub field: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct StockOperationDataQuality {
    pub action_count: usize,
    pub missing_end_price_count: usize,
    pub missing_benchmark_count: usize,
    pub missing_fx_count: usize,
    pub missing_weight_count: usize,
    pub notes: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::{StockOperationFieldIssue, StockOperationReviewQuery};
    use chrono::NaiveDate;

    #[test]
    fn query_contract_has_no_manual_benchmark_field() {
        let query = StockOperationReviewQuery {
            start_date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            end_date: NaiveDate::from_ymd_opt(2026, 8, 30).unwrap(),
            account_id: None,
            market: Some("CN".to_string()),
            base_currency: "CNY".to_string(),
        };
        let value = serde_json::to_value(query).unwrap();
        assert_eq!(value["market"], "CN");
        assert!(value.get("benchmark_symbol").is_none());
        assert!(value.get("availability").is_none());
    }

    #[test]
    fn field_issue_identifies_only_the_affected_field() {
        let issue = StockOperationFieldIssue {
            code: "missing_weight".to_string(),
            field: "weight_change".to_string(),
            message: "缺少操作前总资产快照。".to_string(),
        };
        let value = serde_json::to_value(issue).unwrap();
        assert_eq!(value["field"], "weight_change");
        assert!(value.get("severity").is_none());
        assert!(value.get("blocking").is_none());
    }
}
