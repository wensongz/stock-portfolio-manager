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
