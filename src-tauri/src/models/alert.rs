use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceAlert {
    pub id: String,
    pub holding_id: Option<String>,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub alert_type: String,
    pub threshold: f64,
    pub is_active: bool,
    pub is_triggered: bool,
    pub triggered_at: Option<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TriggeredAlert {
    pub alert: PriceAlert,
    pub current_value: f64,
    pub message: String,
}
