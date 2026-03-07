use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Transaction {
    pub id: String,
    pub holding_id: Option<String>,
    pub account_id: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    #[serde(rename = "type")]
    pub tx_type: String,
    pub shares: f64,
    pub price: f64,
    pub total_amount: f64,
    pub commission: f64,
    pub currency: String,
    pub traded_at: String,
    pub notes: String,
    pub created_at: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateTransactionRequest {
    pub account_id: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    #[serde(rename = "type")]
    pub tx_type: String,
    pub shares: f64,
    pub price: f64,
    pub commission: Option<f64>,
    pub currency: String,
    pub traded_at: String,
    pub notes: Option<String>,
}
