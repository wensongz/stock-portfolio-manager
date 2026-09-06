use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortfolioAlertScopeKind {
    Overall,
    Market,
    Account,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioAlertScope {
    pub kind: PortfolioAlertScopeKind,
    pub market: Option<String>,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortfolioAlertDataStatus {
    Ready,
    Empty,
    Incomplete,
    InvalidConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortfolioAlertBreachKind {
    CategoryDeviation,
    Concentration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AllocationDirection {
    Overweight,
    Underweight,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortfolioAlertBreachDirection {
    Overweight,
    Underweight,
    AboveLimit,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioAlertTarget {
    pub category_id: String,
    pub target_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioAlertConfig {
    pub id: String,
    pub scope: PortfolioAlertScope,
    pub base_currency: String,
    pub deviation_threshold: f64,
    pub concentration_threshold: f64,
    pub is_active: bool,
    pub targets: Vec<PortfolioAlertTarget>,
    pub last_snapshot: Option<PortfolioAlertSnapshot>,
    pub last_evaluated_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SavePortfolioAlertConfigInput {
    pub id: Option<String>,
    pub scope: PortfolioAlertScope,
    pub base_currency: String,
    pub deviation_threshold: f64,
    pub concentration_threshold: f64,
    pub is_active: bool,
    pub targets: Vec<PortfolioAlertTarget>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct CategoryAllocation {
    pub category_id: Option<String>,
    pub category_name: String,
    pub category_color: String,
    pub category_icon: String,
    pub target_percent: f64,
    pub current_percent: f64,
    pub relative_deviation_percent: Option<f64>,
    pub current_market_value: f64,
    pub target_market_value: f64,
    pub rebalance_amount: f64,
    pub direction: Option<AllocationDirection>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ConcentrationAlert {
    pub market: String,
    pub symbol: String,
    pub normalized_symbol: String,
    pub name: String,
    pub category_id: Option<String>,
    pub market_value: f64,
    pub position_percent: f64,
    pub threshold_percent: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioAlertSnapshot {
    pub config_id: String,
    pub scope: PortfolioAlertScope,
    pub base_currency: String,
    pub evaluated_at: String,
    pub total_market_value: f64,
    pub categories: Vec<CategoryAllocation>,
    pub concentrations: Vec<ConcentrationAlert>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MissingPortfolioAlertData {
    pub market: Option<String>,
    pub symbol: Option<String>,
    pub currency: Option<String>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioAlertBreach {
    pub config_id: String,
    pub breach_key: String,
    pub breach_kind: PortfolioAlertBreachKind,
    pub direction: PortfolioAlertBreachDirection,
    pub first_triggered_at: String,
    pub last_seen_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioAlertNotification {
    pub config_id: String,
    pub scope: PortfolioAlertScope,
    pub breach: PortfolioAlertBreach,
    pub message: String,
    pub triggered_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioAlertEvaluation {
    pub status: PortfolioAlertDataStatus,
    pub snapshot: Option<PortfolioAlertSnapshot>,
    pub stale: bool,
    pub missing_data: Vec<MissingPortfolioAlertData>,
    pub active_breaches: Vec<PortfolioAlertBreach>,
    pub newly_triggered: Vec<PortfolioAlertBreach>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioAlertView {
    pub config: Option<PortfolioAlertConfig>,
    pub evaluation: Option<PortfolioAlertEvaluation>,
}

#[cfg(test)]
mod tests {
    use super::{PortfolioAlertBreachDirection, PortfolioAlertScope, PortfolioAlertScopeKind};
    use serde_json::json;

    #[test]
    fn portfolio_alert_contract_serializes_camel_case_fields_and_uppercase_enums() {
        let scope = PortfolioAlertScope {
            kind: PortfolioAlertScopeKind::Market,
            market: Some("US".to_string()),
            account_id: None,
        };
        assert_eq!(
            serde_json::to_value(scope).unwrap(),
            json!({ "kind": "MARKET", "market": "US", "accountId": null })
        );
        assert_eq!(
            serde_json::to_value(PortfolioAlertBreachDirection::AboveLimit).unwrap(),
            json!("ABOVE_LIMIT")
        );
    }
}
