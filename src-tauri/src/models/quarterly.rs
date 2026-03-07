use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuarterlySnapshot {
    pub id: String,
    pub quarter: String,
    pub snapshot_date: String,
    pub total_value: f64,
    pub total_cost: f64,
    pub total_pnl: f64,
    pub us_value: f64,
    pub us_cost: f64,
    pub cn_value: f64,
    pub cn_cost: f64,
    pub hk_value: f64,
    pub hk_cost: f64,
    pub exchange_rates: String,
    pub overall_notes: Option<String>,
    pub created_at: String,
    pub holding_count: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuarterlyHoldingSnapshot {
    pub id: String,
    pub quarterly_snapshot_id: String,
    pub account_id: String,
    pub account_name: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub category_name: String,
    pub category_color: String,
    pub shares: f64,
    pub avg_cost: f64,
    pub close_price: f64,
    pub market_value: f64,
    pub cost_value: f64,
    pub pnl: f64,
    pub pnl_percent: f64,
    pub weight: f64,
    pub notes: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuarterlySnapshotDetail {
    pub snapshot: QuarterlySnapshot,
    pub holdings: Vec<QuarterlyHoldingSnapshot>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ComparisonOverview {
    pub q1_total_value: f64,
    pub q2_total_value: f64,
    pub value_change: f64,
    pub value_change_percent: f64,
    pub q1_total_cost: f64,
    pub q2_total_cost: f64,
    pub q1_pnl: f64,
    pub q2_pnl: f64,
    pub q1_holding_count: usize,
    pub q2_holding_count: usize,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MarketComparison {
    pub market: String,
    pub q1_value: f64,
    pub q2_value: f64,
    pub value_change: f64,
    pub value_change_percent: f64,
    pub q1_cost: f64,
    pub q2_cost: f64,
    pub q1_pnl: f64,
    pub q2_pnl: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CategoryComparison {
    pub category_name: String,
    pub category_color: String,
    pub q1_value: f64,
    pub q2_value: f64,
    pub value_change: f64,
    pub value_change_percent: f64,
    pub q1_cost: f64,
    pub q2_cost: f64,
    pub q1_pnl: f64,
    pub q2_pnl: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HoldingChangeItem {
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub category_name: String,
    pub q1_shares: Option<f64>,
    pub q2_shares: Option<f64>,
    pub q1_value: Option<f64>,
    pub q2_value: Option<f64>,
    pub shares_change: f64,
    pub value_change: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HoldingChanges {
    pub new_holdings: Vec<HoldingChangeItem>,
    pub closed_holdings: Vec<HoldingChangeItem>,
    pub increased: Vec<HoldingChangeItem>,
    pub decreased: Vec<HoldingChangeItem>,
    pub unchanged: Vec<HoldingChangeItem>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuarterComparison {
    pub quarter1: String,
    pub quarter2: String,
    pub overview: ComparisonOverview,
    pub by_market: Vec<MarketComparison>,
    pub by_category: Vec<CategoryComparison>,
    pub holding_changes: HoldingChanges,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HoldingNoteHistory {
    pub quarter: String,
    pub snapshot_date: String,
    pub shares: f64,
    pub avg_cost: f64,
    pub close_price: f64,
    pub pnl_percent: f64,
    pub notes: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuarterlyNotesSummary {
    pub snapshot_id: String,
    pub quarter: String,
    pub snapshot_date: String,
    pub overall_notes: String,
    pub total_value: f64,
    pub total_pnl: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct QuarterlyTrends {
    pub quarters: Vec<String>,
    pub total_values: Vec<f64>,
    pub total_costs: Vec<f64>,
    pub total_pnls: Vec<f64>,
    pub market_values: HashMap<String, Vec<f64>>,
    pub category_values: HashMap<String, Vec<f64>>,
    pub holding_counts: Vec<usize>,
}
