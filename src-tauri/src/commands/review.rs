use crate::db::Database;
use crate::models::review::{DecisionStatistics, HoldingReview};
use crate::models::stock_operation_review::{
    StockOperationReviewQuery, StockOperationReviewReport,
};
use crate::models::stock_review::{
    StockCampaignDetail, StockReviewAnnotation, StockReviewAnnotationInput,
    StockReviewOverrideInput, StockReviewQuery, StockReviewReport,
};
use crate::services::exchange_rate_service::ExchangeRateCache;
use crate::services::{
    review_service, snapshot_service, stock_operation_review_service, stock_review_service,
};
use chrono::{Duration, NaiveDate};
use tauri::State;
use tracing::warn;

fn stock_review_snapshot_range(query: &StockReviewQuery) -> (NaiveDate, NaiveDate) {
    (
        query
            .start_date
            .checked_sub_signed(Duration::days(10))
            .unwrap_or(query.start_date),
        query.end_date,
    )
}

async fn backfill_stock_review_snapshots(
    db: &Database,
    cache: &ExchangeRateCache,
    query: &StockReviewQuery,
) {
    let (start_date, end_date) = stock_review_snapshot_range(query);
    if let Err(error) =
        snapshot_service::backfill_snapshots(db, cache, start_date, end_date, false).await
    {
        // Match the performance page: a refresh failure must not hide a cached
        // review. Exact snapshot coverage will keep affected metrics unavailable.
        warn!("stock review snapshot backfill failed for {start_date}/{end_date}: {error}");
    }
}

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

fn normalize_user_annotation(mut input: StockReviewAnnotationInput) -> StockReviewAnnotationInput {
    input.source = "user".to_string();
    input
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub async fn get_stock_review_report(
    start_date: String,
    end_date: String,
    account_id: Option<String>,
    market: Option<String>,
    benchmark_symbol: Option<String>,
    base_currency: String,
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
) -> Result<StockReviewReport, String> {
    let query = query(
        &start_date,
        &end_date,
        account_id,
        market,
        benchmark_symbol,
        base_currency,
    )?;
    backfill_stock_review_snapshots(&db, &cache, &query).await;
    stock_review_service::get_stock_review_report(&db, query).await
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
#[allow(clippy::too_many_arguments)]
pub async fn get_stock_campaign_detail(
    start_date: String,
    end_date: String,
    account_id: Option<String>,
    market: Option<String>,
    benchmark_symbol: Option<String>,
    base_currency: String,
    campaign_id: String,
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
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
    backfill_stock_review_snapshots(&db, &cache, &query).await;
    stock_review_service::get_stock_campaign_detail(&db, query, campaign_id.trim()).await
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_stock_review_annotation(
    input: StockReviewAnnotationInput,
    db: State<'_, Database>,
) -> Result<StockReviewAnnotation, String> {
    stock_review_service::save_user_stock_review_annotation(&db, normalize_user_annotation(input))
}

#[tauri::command(rename_all = "camelCase")]
#[allow(clippy::too_many_arguments)]
pub async fn confirm_stock_review_override(
    start_date: String,
    end_date: String,
    account_id: Option<String>,
    market: Option<String>,
    benchmark_symbol: Option<String>,
    base_currency: String,
    input: StockReviewOverrideInput,
    db: State<'_, Database>,
    cache: State<'_, ExchangeRateCache>,
) -> Result<StockReviewReport, String> {
    let query = query(
        &start_date,
        &end_date,
        account_id,
        market,
        benchmark_symbol,
        base_currency,
    )?;
    backfill_stock_review_snapshots(&db, &cache, &query).await;
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
    use super::{
        normalize_user_annotation, query, stock_operation_query, stock_review_snapshot_range,
    };
    use crate::models::stock_review::StockReviewAnnotationInput;
    use chrono::NaiveDate;

    #[test]
    fn general_annotation_command_always_uses_user_provenance() {
        // Caller data must not be able to self-authorize ai_confirmed provenance.
        let normalized = normalize_user_annotation(StockReviewAnnotationInput {
            id: "note".to_string(),
            scope_type: "period".to_string(),
            scope_key: "2024".to_string(),
            account_id: None,
            symbol: None,
            annotation_type: "note".to_string(),
            value_json: "{}".to_string(),
            source: "ai_confirmed".to_string(),
        });
        assert_eq!(normalized.source, "user");
    }

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

    #[test]
    fn stock_review_snapshot_range_includes_a_prior_session_baseline() {
        let query = query(
            "2026-06-30",
            "2026-08-28",
            Some("acct".to_string()),
            None,
            None,
            "USD".to_string(),
        )
        .unwrap();

        assert_eq!(
            stock_review_snapshot_range(&query),
            (
                NaiveDate::from_ymd_opt(2026, 6, 20).unwrap(),
                NaiveDate::from_ymd_opt(2026, 8, 28).unwrap(),
            )
        );
    }

    #[test]
    fn lightweight_stock_operation_query_has_no_manual_benchmark() {
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
        let value = serde_json::to_value(parsed).unwrap();
        assert!(value.get("benchmark_symbol").is_none());
        assert!(value.get("campaign_id").is_none());
    }
}
