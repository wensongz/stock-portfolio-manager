use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuarterlyHoldingStatus {
    pub snapshot_id: String,
    pub quarter: String,
    pub shares: f64,
    pub avg_cost: f64,
    pub close_price: f64,
    pub pnl_percent: f64,
    pub notes: Option<String>,
    pub decision_quality: Option<String>, // "correct" | "wrong" | "pending"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HoldingReview {
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub is_current_holding: bool,
    pub quarterly_timeline: Vec<QuarterlyHoldingStatus>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionStatistics {
    pub total_decisions: usize,
    pub correct_count: usize,
    pub wrong_count: usize,
    pub pending_count: usize,
    pub accuracy_rate: f64,
}
