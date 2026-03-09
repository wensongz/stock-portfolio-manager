use serde::{Deserialize, Serialize};
use crate::models::ExchangeRates;

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DashboardSummary {
    pub total_market_value: f64,
    pub total_cost: f64,
    pub total_pnl: f64,
    pub total_pnl_percent: f64,
    pub daily_pnl: f64,
    pub us_market_value: f64,
    pub cn_market_value: f64,
    pub hk_market_value: f64,
    pub exchange_rates: ExchangeRates,
    pub base_currency: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HoldingDetail {
    pub id: String,
    pub account_id: String,
    pub account_name: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub category_name: String,
    pub category_color: String,
    pub shares: f64,
    pub avg_cost: f64,
    pub current_price: f64,
    pub market_value: f64,
    pub cost_value: f64,
    pub pnl: f64,
    pub pnl_percent: f64,
    pub daily_pnl: f64,
    pub currency: String,
}
