use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PieSlice {
    pub name: String,
    pub value: f64,
    pub color: Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PnlItem {
    pub symbol: String,
    pub name: String,
    pub pnl: f64,
    pub pnl_percent: Option<f64>,
    pub market_value: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StatisticsOverview {
    pub total_market_value: f64,
    pub total_cost: f64,
    pub total_pnl: f64,
    pub total_pnl_percent: f64,
    pub market_distribution: Vec<PieSlice>,
    pub category_distribution: Vec<PieSlice>,
    pub account_distribution: Vec<PieSlice>,
    pub stock_distribution: Vec<PieSlice>,
    pub top_gainers: Vec<PnlItem>,
    pub top_losers: Vec<PnlItem>,
    pub holdings: Vec<crate::models::dashboard::HoldingDetail>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct MarketStatistics {
    pub market: String,
    pub total_market_value: f64,
    pub total_cost: f64,
    pub total_pnl: f64,
    pub total_pnl_percent: f64,
    pub account_distribution: Vec<PieSlice>,
    pub category_distribution: Vec<PieSlice>,
    pub stock_distribution: Vec<PieSlice>,
    pub holdings: Vec<crate::models::dashboard::HoldingDetail>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct AccountStatistics {
    pub account_id: String,
    pub account_name: String,
    pub market: String,
    pub total_market_value: f64,
    pub total_cost: f64,
    pub total_pnl: f64,
    pub total_pnl_percent: f64,
    pub category_distribution: Vec<PieSlice>,
    pub stock_distribution: Vec<PieSlice>,
    pub holdings: Vec<crate::models::dashboard::HoldingDetail>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CategoryStatistics {
    pub category_id: String,
    pub category_name: String,
    pub category_color: String,
    pub total_market_value: f64,
    pub total_cost: f64,
    pub total_pnl: f64,
    pub total_pnl_percent: f64,
    pub market_distribution: Vec<PieSlice>,
    pub holdings: Vec<crate::models::dashboard::HoldingDetail>,
}
