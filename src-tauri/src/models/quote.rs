use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StockQuote {
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub current_price: f64,
    pub previous_close: f64,
    pub change: f64,
    pub change_percent: f64,
    pub high: f64,
    pub low: f64,
    pub volume: u64,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct StockMetadata {
    pub symbol: String,
    pub name: String,
    pub market: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct HoldingWithQuote {
    pub id: String,
    pub account_id: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub category_id: Option<String>,
    pub shares: f64,
    pub avg_cost: f64,
    pub currency: String,
    pub created_at: String,
    pub updated_at: String,
    pub quote: Option<StockQuote>,
    pub market_value: Option<f64>,
    pub total_cost: Option<f64>,
    pub unrealized_pnl: Option<f64>,
    pub unrealized_pnl_percent: Option<f64>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ExchangeRates {
    pub usd_cny: f64,
    pub usd_hkd: f64,
    pub cny_hkd: f64,
    pub updated_at: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DailyPortfolioValue {
    pub id: i64,
    pub date: String,
    pub total_cost: f64,
    pub total_value: f64,
    pub us_cost: f64,
    pub us_value: f64,
    pub cn_cost: f64,
    pub cn_value: f64,
    pub hk_cost: f64,
    pub hk_value: f64,
    pub exchange_rates: String,
    pub daily_pnl: f64,
    pub cumulative_pnl: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct DailyHoldingSnapshot {
    pub id: i64,
    pub date: String,
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub category_name: Option<String>,
    pub shares: f64,
    pub avg_cost: f64,
    pub close_price: f64,
    pub market_value: f64,
}
