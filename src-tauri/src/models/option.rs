use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OptionRecord {
    pub id: String,
    pub account_id: String,
    pub option_symbol: String,
    pub underlying: String,
    pub expiry_date: String,
    pub strike_price: f64,
    pub option_type: String, // "P" or "C"
    pub action: String,      // "SELL" or "BUY"
    pub code: String,        // "O", "C;Ep", "A;C"
    pub quantity: i64,       // number of contracts (absolute)
    pub price: f64,
    pub amount: f64,
    pub commission: f64,
    pub fee: f64,
    pub traded_at: Option<String>,
    pub settled_at: Option<String>,
    pub created_at: String,
    pub contract_status: String, // "active", "expired", "assigned", "closed"
}

/// A paired option contract with status derived from matching records
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct OptionContract {
    pub id: String, // unique identifier for this contract instance
    pub option_symbol: String,
    pub underlying: String,
    pub expiry_date: String,
    pub strike_price: f64,
    pub option_type: String,       // "P" or "C"
    pub contracts: i64,            // number of contracts
    pub open_price: f64,           // sell to open price
    pub open_amount: f64,          // total premium received
    pub commission: f64,           // commission paid on open
    pub traded_at: Option<String>, // trade date of the open record
    pub close_price: Option<f64>,
    pub close_code: Option<String>, // "C;Ep" (expired), "A;C" (assigned), "C;P" (closed)
    pub status: String,             // "active", "expired", "assigned", "closed"
    pub account_id: String,
}

/// Sell Put simulation result per stock
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SellPutSimulation {
    pub underlying: String,
    pub contracts: Vec<PutContractSimulation>,
    pub total_cash_needed: f64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct PutContractSimulation {
    pub option_symbol: String,
    pub strike_price: f64,
    pub contracts: i64,
    pub would_be_assigned: bool,
    pub cash_needed: f64,
}

/// Sell Call simulation result per stock
#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct SellCallSimulation {
    pub underlying: String,
    pub contracts: Vec<CallContractSimulation>,
    pub total_shares_needed: i64,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct CallContractSimulation {
    pub option_symbol: String,
    pub strike_price: f64,
    pub contracts: i64,
    pub would_be_assigned: bool,
    pub shares_needed: i64,
}
