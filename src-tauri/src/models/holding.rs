use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Holding {
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
}

#[derive(Debug, Deserialize)]
pub struct CreateHoldingRequest {
    pub account_id: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub category_id: Option<String>,
    pub shares: f64,
    pub avg_cost: f64,
    pub currency: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateHoldingRequest {
    pub name: Option<String>,
    pub category_id: Option<String>,
    pub shares: Option<f64>,
    pub avg_cost: Option<f64>,
}
