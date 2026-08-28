#![allow(dead_code)]

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricStatus {
    Available,
    Degraded,
    Pending,
    Unavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricAvailability {
    pub status: MetricStatus,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewQuery {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub account_id: Option<String>,
    pub market: Option<String>,
    pub benchmark_symbol: Option<String>,
    pub base_currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewMethodology {
    pub query: StockReviewQuery,
    pub actual_return_method: String,
    pub shadow_return_method: String,
    pub benchmark_return_method: String,
    pub fixed_weights: Vec<FixedWeight>,
    pub benchmark_symbol: Option<String>,
    pub market_data_coverage: DataCoverage,
    pub exchange_rate_coverage: DataCoverage,
    pub algorithm_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FixedWeight {
    pub key: String,
    pub weight: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DataCoverage {
    pub availability: MetricAvailability,
    pub covered_days: Option<u32>,
    pub expected_days: Option<u32>,
    pub coverage_ratio: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewReport {
    pub methodology: StockReviewMethodology,
    pub summary: StockReviewSummary,
    pub curves: Vec<ReviewCurvePoint>,
    pub attribution: RebalanceAttributionSummary,
    pub risk_structure: RiskStructureDetail,
    pub actions: Vec<StockActionReview>,
    pub campaigns: Vec<StockCampaignSummary>,
    pub data_quality: StockReviewDataQuality,
    pub annotations: Vec<StockReviewAnnotation>,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewSummary {
    pub result_quality: ResultQualityMetric,
    pub max_drawdown: MaxDrawdownMetric,
    pub rebalance_value_add: RebalanceValueAddMetric,
    pub forward_effect: ForwardEffectMetric,
    pub risk_structure: RiskStructureMetric,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResultQualityMetric {
    pub availability: MetricAvailability,
    pub portfolio_return: Option<f64>,
    pub shadow_return: Option<f64>,
    pub benchmark_return: Option<f64>,
    pub active_return: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MaxDrawdownMetric {
    pub availability: MetricAvailability,
    pub max_drawdown: Option<f64>,
    pub peak_date: Option<NaiveDate>,
    pub trough_date: Option<NaiveDate>,
    pub recovery_date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RebalanceValueAddMetric {
    pub availability: MetricAvailability,
    pub value_add: Option<f64>,
    pub actual_return: Option<f64>,
    pub shadow_return: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForwardEffectMetric {
    pub availability: MetricAvailability,
    pub day_60: ForwardEffectWindow,
    pub day_120: ForwardEffectWindow,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForwardEffectWindow {
    pub trading_days: u32,
    pub return_value: Option<f64>,
    pub benchmark_return: Option<f64>,
    pub excess_return: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskStructureMetric {
    pub availability: MetricAvailability,
    pub concentration: Option<f64>,
    pub diversification_score: Option<f64>,
    pub largest_position_weight: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ReviewCurvePoint {
    pub date: NaiveDate,
    pub portfolio_return: Option<f64>,
    pub shadow_return: Option<f64>,
    pub benchmark_return: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RebalanceAttributionSummary {
    pub availability: MetricAvailability,
    pub total_value_add: Option<f64>,
    pub buy_value_add: Option<f64>,
    pub sell_value_add: Option<f64>,
    pub fees: Option<f64>,
    pub action_contributions: Vec<RebalanceAttributionItem>,
    pub contributors: Vec<RebalanceAttributionItem>,
    pub detractors: Vec<RebalanceAttributionItem>,
    pub dividend_contribution: Option<f64>,
    pub fee_contribution: Option<f64>,
    pub currency_contribution: Option<f64>,
    pub cash_contribution: Option<f64>,
    pub explained_value_difference: Option<f64>,
    pub ending_value_difference: Option<f64>,
    pub residual: Option<f64>,
    pub residual_to_average_nav: Option<f64>,
    pub percentage_basis_label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RebalanceAttributionItem {
    pub market: String,
    pub symbol: String,
    pub action_type: String,
    pub action_id: String,
    pub amount: f64,
    pub percentage_of_average_nav: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskStructureDetail {
    pub availability: MetricAvailability,
    pub market_weights: Vec<RiskStructureWeight>,
    pub category_weights: Vec<RiskStructureWeight>,
    pub top_position_weights: Vec<RiskStructureWeight>,
    pub concentration: Option<f64>,
    pub diversification_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskStructureWeight {
    pub key: String,
    pub weight: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockActionReview {
    pub action_id: String,
    pub transaction_ids: Vec<String>,
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub action_type: String,
    pub traded_at: String,
    pub weighted_average_price: Option<f64>,
    pub gross_amount: Option<f64>,
    pub currency: Option<String>,
    pub shares_before: Option<f64>,
    pub shares_after: Option<f64>,
    pub portfolio_weight_before: Option<f64>,
    pub portfolio_weight_after: Option<f64>,
    pub fees: Option<f64>,
    pub contribution: Option<f64>,
    pub observation_windows: Vec<ForwardEffectWindow>,
    pub status: MetricStatus,
    pub fact_labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockCampaignSummary {
    pub campaign_id: String,
    pub account_ids: Vec<String>,
    pub action_ids: Vec<String>,
    pub fragments: Vec<AccountCampaignFragment>,
    pub campaign_status: StockCampaignStatus,
    pub availability: MetricAvailability,
    pub symbol: String,
    pub market: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub contribution: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StockCampaignStatus {
    Active,
    Completed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockCampaignTransferFact {
    pub transaction_id: String,
    pub action_id: Option<String>,
    pub traded_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AccountCampaignFragment {
    pub fragment_id: String,
    pub logical_campaign_id: String,
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: StockCampaignStatus,
    pub action_ids: Vec<String>,
    pub transfer_in: Option<StockCampaignTransferFact>,
    pub transfer_out: Option<StockCampaignTransferFact>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockCampaignDetail {
    pub summary: StockCampaignSummary,
    pub actions: Vec<StockActionReview>,
    pub forward_effect_20d: ForwardEffectWindow,
    pub annotations: Vec<StockReviewAnnotation>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewDataQuality {
    pub availability: MetricAvailability,
    pub actual_result_availability: MetricAvailability,
    pub shadow_value_add_availability: MetricAvailability,
    pub attribution_availability: MetricAvailability,
    pub forward_effect_availability: MetricAvailability,
    pub issues: Vec<StockReviewIssue>,
    pub market_data_coverage: Option<f64>,
    pub exchange_rate_coverage: Option<f64>,
    pub interval_drawdown_only: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StockReviewIssueSeverity {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewIssue {
    pub code: String,
    pub severity: StockReviewIssueSeverity,
    pub message: String,
    pub affected_symbol: Option<String>,
    pub affected_date: Option<NaiveDate>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewAnnotation {
    pub id: String,
    pub scope_type: String,
    pub scope_key: String,
    pub account_id: Option<String>,
    pub symbol: Option<String>,
    pub annotation_type: String,
    pub value_json: String,
    pub source: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewAnnotationInput {
    pub scope_type: String,
    pub scope_key: String,
    pub account_id: Option<String>,
    pub symbol: Option<String>,
    pub annotation_type: String,
    pub value_json: String,
    pub source: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewOverride {
    pub id: String,
    pub override_type: String,
    pub transaction_ids_json: String,
    pub value_json: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewOverrideInput {
    pub override_type: String,
    pub transaction_ids_json: String,
    pub value_json: String,
}
