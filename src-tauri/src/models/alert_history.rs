use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AlertHistoryDetail {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AlertHistoryMessage {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub message: String,
    pub scope_name: String,
    pub account_name: Option<String>,
    pub triggered_at: String,
    pub details: Vec<AlertHistoryDetail>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AlertHistoryQuery {
    pub kind: Option<String>,
    pub search: Option<String>,
    pub page: Option<u32>,
    pub page_size: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AlertHistoryPage {
    pub items: Vec<AlertHistoryMessage>,
    pub total: u64,
    pub page: u32,
    pub page_size: u32,
}

#[cfg(test)]
mod tests {
    use super::{AlertHistoryDetail, AlertHistoryMessage, AlertHistoryPage, AlertHistoryQuery};

    #[test]
    fn alert_history_contract_serializes_all_fields_as_camel_case() {
        let page = AlertHistoryPage {
            items: vec![AlertHistoryMessage {
                id: "message-1".into(),
                kind: "PRICE".into(),
                title: "Apple reached target".into(),
                message: "AAPL crossed 120".into(),
                scope_name: "Apple (AAPL)".into(),
                account_name: Some("Long term".into()),
                triggered_at: "2026-09-19T10:00:00Z".into(),
                details: vec![AlertHistoryDetail {
                    label: "Current price".into(),
                    value: "123.45 USD".into(),
                }],
            }],
            total: 1,
            page: 2,
            page_size: 20,
        };

        assert_eq!(
            serde_json::to_value(page).unwrap(),
            serde_json::json!({
                "items": [{
                    "id": "message-1",
                    "kind": "PRICE",
                    "title": "Apple reached target",
                    "message": "AAPL crossed 120",
                    "scopeName": "Apple (AAPL)",
                    "accountName": "Long term",
                    "triggeredAt": "2026-09-19T10:00:00Z",
                    "details": [{"label": "Current price", "value": "123.45 USD"}]
                }],
                "total": 1,
                "page": 2,
                "pageSize": 20
            })
        );

        let query: AlertHistoryQuery = serde_json::from_value(serde_json::json!({
            "kind": "PORTFOLIO",
            "search": "Retirement",
            "page": 3,
            "pageSize": 25
        }))
        .unwrap();
        assert_eq!(query.kind.as_deref(), Some("PORTFOLIO"));
        assert_eq!(query.search.as_deref(), Some("Retirement"));
        assert_eq!(query.page, Some(3));
        assert_eq!(query.page_size, Some(25));
    }
}
