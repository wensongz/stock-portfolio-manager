use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionReviewReport {
    pub account_id: String,
    pub currency: String,
    pub period_days: Option<i64>,
    pub generated_at: String,
    pub summary: OptionReviewSummary,
    pub underlyings: Vec<OptionUnderlyingReview>,
    pub data_quality: OptionReviewDataQuality,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionReviewSummary {
    pub completed_campaigns: usize,
    pub active_campaigns: usize,
    /// Opening premium across all filtered Campaigns, including active ones.
    pub gross_premium: f64,
    /// Cash net across all filtered Campaigns, including active ones.
    pub net_premium_pnl: f64,
    /// Opening premium used as the denominator of completed-only retention.
    pub completed_gross_premium: f64,
    /// Cash net used as the numerator of completed-only performance metrics.
    pub completed_net_premium_pnl: f64,
    pub retention_rate: Option<f64>,
    pub annualized_yield_on_notional: Option<f64>,
    pub worst_campaign: Option<OptionWorstCampaign>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionWorstCampaign {
    pub campaign_id: String,
    pub underlying: String,
    pub started_at: String,
    pub ended_at: String,
    pub strategy_path: Vec<String>,
    pub net_premium_pnl: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionUnderlyingReview {
    pub underlying: String,
    pub completed_campaigns: usize,
    pub active_campaigns: usize,
    /// Opening premium across all filtered Campaigns, including active ones.
    pub gross_premium: f64,
    /// Cash net across all filtered Campaigns, including active ones.
    pub net_premium_pnl: f64,
    /// Opening premium used as the denominator of completed-only retention.
    pub completed_gross_premium: f64,
    /// Cash net used as the numerator of completed-only performance metrics.
    pub completed_net_premium_pnl: f64,
    pub retention_rate: Option<f64>,
    pub annualized_yield_on_notional: Option<f64>,
    pub worst_campaign_pnl: Option<f64>,
    pub flags: Vec<String>,
    pub campaigns: Vec<OptionCampaign>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionCampaign {
    pub id: String,
    pub underlying: String,
    pub option_symbol: String,
    pub expiry_date: String,
    pub contracts: i64,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: String,
    pub inferred: bool,
    pub strategy_path: Vec<String>,
    pub gross_premium: f64,
    pub close_cost: f64,
    pub fees: f64,
    pub net_premium_pnl: Option<f64>,
    pub secured_notional: f64,
    pub capital_days: f64,
    pub retention_rate: Option<f64>,
    pub annualized_yield_on_notional: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionReviewDataQuality {
    pub excluded_open_campaigns: usize,
    pub unmatched_records: usize,
    pub missing_trade_dates: usize,
    pub notes: Vec<String>,
}
