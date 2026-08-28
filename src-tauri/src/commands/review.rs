use crate::db::Database;
use crate::models::review::{DecisionStatistics, HoldingReview};
use crate::models::stock_review::{
    StockCampaignDetail, StockReviewAnnotation, StockReviewAnnotationInput,
    StockReviewOverrideInput, StockReviewQuery, StockReviewReport,
};
use crate::services::stock_review_persistence::AnnotationSaveContext;
use crate::services::{review_service, stock_review_service};
use chrono::NaiveDate;
use tauri::State;

fn query(
    start_date: &str,
    end_date: &str,
    account_id: Option<String>,
    market: Option<String>,
    benchmark_symbol: Option<String>,
    base_currency: String,
) -> Result<StockReviewQuery, String> {
    let start_date = NaiveDate::parse_from_str(start_date, "%Y-%m-%d")
        .map_err(|_| "开始日期格式无效，请使用 YYYY-MM-DD。".to_string())?;
    let end_date = NaiveDate::parse_from_str(end_date, "%Y-%m-%d")
        .map_err(|_| "结束日期格式无效，请使用 YYYY-MM-DD。".to_string())?;
    let query = StockReviewQuery {
        start_date,
        end_date,
        account_id,
        market,
        benchmark_symbol,
        base_currency: base_currency.trim().to_ascii_uppercase(),
    };
    stock_review_service::validate_query(&query)?;
    Ok(query)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_stock_review_report(
    start_date: String,
    end_date: String,
    account_id: Option<String>,
    market: Option<String>,
    benchmark_symbol: Option<String>,
    base_currency: String,
    db: State<'_, Database>,
) -> Result<StockReviewReport, String> {
    let query = query(
        &start_date,
        &end_date,
        account_id,
        market,
        benchmark_symbol,
        base_currency,
    )?;
    stock_review_service::get_stock_review_report(&db, query).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_stock_campaign_detail(
    start_date: String,
    end_date: String,
    account_id: Option<String>,
    market: Option<String>,
    benchmark_symbol: Option<String>,
    base_currency: String,
    campaign_id: String,
    db: State<'_, Database>,
) -> Result<StockCampaignDetail, String> {
    if campaign_id.trim().is_empty() {
        return Err("Campaign ID 不能为空。".to_string());
    }
    let query = query(
        &start_date,
        &end_date,
        account_id,
        market,
        benchmark_symbol,
        base_currency,
    )?;
    stock_review_service::get_stock_campaign_detail(&db, query, campaign_id.trim()).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_stock_review_annotation(
    input: StockReviewAnnotationInput,
    ai_confirmed: Option<bool>,
    db: State<'_, Database>,
) -> Result<StockReviewAnnotation, String> {
    let context = if ai_confirmed.unwrap_or(false) {
        AnnotationSaveContext::AiAfterExplicitUserConfirmation
    } else {
        AnnotationSaveContext::UserInitiated
    };
    stock_review_service::save_stock_review_annotation(&db, input, context)
}

#[tauri::command(rename_all = "camelCase")]
pub async fn confirm_stock_review_override(
    start_date: String,
    end_date: String,
    account_id: Option<String>,
    market: Option<String>,
    benchmark_symbol: Option<String>,
    base_currency: String,
    input: StockReviewOverrideInput,
    db: State<'_, Database>,
) -> Result<StockReviewReport, String> {
    let query = query(
        &start_date,
        &end_date,
        account_id,
        market,
        benchmark_symbol,
        base_currency,
    )?;
    stock_review_service::confirm_stock_review_override(&db, query, input).await
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
    use super::query;

    #[test]
    fn stock_review_query_boundary_validates_and_normalizes_displayable_inputs() {
        assert!(query(
            "2024-13-01",
            "2024-01-02",
            None,
            None,
            None,
            "USD".to_string()
        )
        .is_err());
        assert!(query(
            "2024-02-01",
            "2024-01-02",
            None,
            None,
            None,
            "USD".to_string()
        )
        .is_err());
        assert!(query(
            "2024-01-01",
            "2024-01-02",
            None,
            Some("EU".to_string()),
            None,
            "USD".to_string()
        )
        .is_err());
        assert!(query(
            "2024-01-01",
            "2024-01-02",
            None,
            None,
            None,
            "EUR".to_string()
        )
        .is_err());
        assert!(query(
            "2024-01-01",
            "2024-01-02",
            Some(" ".to_string()),
            None,
            None,
            "USD".to_string()
        )
        .is_err());
        assert!(query(
            "2024-01-01",
            "2024-01-02",
            None,
            None,
            Some(" ".to_string()),
            "USD".to_string()
        )
        .is_err());

        let parsed = query(
            "2024-01-01",
            "2024-01-02",
            Some("acct".to_string()),
            Some("US".to_string()),
            Some("^GSPC".to_string()),
            " usd ".to_string(),
        )
        .unwrap();
        assert_eq!(parsed.base_currency, "USD");
        assert_eq!(parsed.account_id.as_deref(), Some("acct"));
    }
}
