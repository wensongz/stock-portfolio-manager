use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub market: String,
    pub description: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}
