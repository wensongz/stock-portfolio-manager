use crate::db::Database;
use crate::models::review::{DecisionStatistics, HoldingReview};
use crate::models::stock_operation_review::{
    StockOperationReviewQuery, StockOperationReviewReport,
};
use crate::services::{review_service, stock_operation_review_service};
use chrono::NaiveDate;
use tauri::State;

fn stock_operation_query(
    start_date: &str,
    end_date: &str,
    account_id: Option<String>,
    market: Option<String>,
    base_currency: String,
) -> Result<StockOperationReviewQuery, String> {
    let start_date = NaiveDate::parse_from_str(start_date, "%Y-%m-%d")
        .map_err(|_| "开始日期格式无效，请使用 YYYY-MM-DD。".to_string())?;
    let end_date = NaiveDate::parse_from_str(end_date, "%Y-%m-%d")
        .map_err(|_| "结束日期格式无效，请使用 YYYY-MM-DD。".to_string())?;
    let account_id = account_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    let market = market
        .map(|value| value.trim().to_ascii_uppercase())
        .filter(|value| !value.is_empty());
    let query = StockOperationReviewQuery {
        start_date,
        end_date,
        account_id,
        market,
        base_currency: base_currency.trim().to_ascii_uppercase(),
    };
    stock_operation_review_service::validate_query(&query)?;
    Ok(query)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_stock_operation_review(
    start_date: String,
    end_date: String,
    account_id: Option<String>,
    market: Option<String>,
    base_currency: String,
    db: State<'_, Database>,
) -> Result<StockOperationReviewReport, String> {
    let query = stock_operation_query(&start_date, &end_date, account_id, market, base_currency)?;
    stock_operation_review_service::get_stock_operation_review(&db, query).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_holding_review(
    db: State<'_, Database>,
    symbol: String,
) -> Result<HoldingReview, String> {
    review_service::get_holding_review(&db, &symbol)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn update_decision_quality(
    db: State<'_, Database>,
    snapshot_id: String,
    symbol: String,
    quality: String,
) -> Result<bool, String> {
    review_service::update_decision_quality(&db, &snapshot_id, &symbol, &quality)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_decision_statistics(
    db: State<'_, Database>,
) -> Result<DecisionStatistics, String> {
    review_service::get_decision_statistics(&db)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_reviewed_symbols(
    db: State<'_, Database>,
) -> Result<Vec<(String, String, String)>, String> {
    review_service::get_reviewed_symbols(&db)
}

#[cfg(test)]
mod tests {
    use super::stock_operation_query;

    #[test]
    fn stock_operation_query_rejects_invalid_date_boundaries() {
        assert!(
            stock_operation_query("2026-13-01", "2026-08-30", None, None, "USD".to_string(),)
                .is_err()
        );
        assert!(
            stock_operation_query("2026-08-31", "2026-08-30", None, None, "USD".to_string(),)
                .is_err()
        );
    }

    #[test]
    fn stock_operation_query_normalizes_account_market_and_currency_boundaries() {
        let parsed = stock_operation_query(
            "2026-07-01",
            "2026-08-30",
            Some(" acct ".to_string()),
            Some(" cn ".to_string()),
            " cny ".to_string(),
        )
        .unwrap();
        assert_eq!(parsed.account_id.as_deref(), Some("acct"));
        assert_eq!(parsed.market.as_deref(), Some("CN"));
        assert_eq!(parsed.base_currency, "CNY");

        let empty_filters = stock_operation_query(
            "2026-07-01",
            "2026-08-30",
            Some("  ".to_string()),
            Some("  ".to_string()),
            " usd ".to_string(),
        )
        .unwrap();
        assert_eq!(empty_filters.account_id, None);
        assert_eq!(empty_filters.market, None);
        assert_eq!(empty_filters.base_currency, "USD");

        assert!(stock_operation_query(
            "2026-07-01",
            "2026-08-30",
            None,
            Some("EU".to_string()),
            "USD".to_string(),
        )
        .is_err());
        assert!(
            stock_operation_query("2026-07-01", "2026-08-30", None, None, "EUR".to_string(),)
                .is_err()
        );
    }
}
