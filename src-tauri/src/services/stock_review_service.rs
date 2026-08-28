#![allow(dead_code)]

use crate::db::Database;
use crate::models::stock_review::*;
use crate::models::Transaction;
use crate::services::rebalance_attribution::{
    calculate_rebalance_attribution, AttributionBatch, AttributionCashBalance,
    AttributionCashReturn, AttributionDividend, AttributionFee, AttributionFxPoint,
    AttributionInput, AttributionPositionBalance, AttributionPricePoint, AttributionSplit,
    AttributionValuationPoint,
};
use crate::services::shadow_portfolio_engine::{
    build_shadow_series, DividendEvent, OpeningCashBalance, OpeningPosition, ShadowDataIssue,
    ShadowFxPoint, ShadowPortfolioInput, ShadowPortfolioResult, ShadowPricePoint,
    ShadowReturnMethod, SplitEvent,
};
use crate::services::stock_action_builder::build_stock_actions;
use crate::services::stock_campaign_builder::build_stock_campaigns;
use crate::services::stock_review_market_data::{
    default_benchmark_symbol, ensure_stock_price_cache, load_benchmark_series,
    load_stock_price_series, DailyMarketPoint, MarketReturnMode,
};
use crate::services::stock_review_metrics::*;
use crate::services::stock_review_persistence::{
    self, AnnotationSaveContext, StockReviewAnnotationFilter,
};
use crate::services::stock_review_quality::{
    build_stock_review_quality, ObservationWindowMaturity, QualityInput,
};
use chrono::{Duration, NaiveDate, Utc};
use rusqlite::params;
use std::collections::{BTreeMap, BTreeSet};

const ALGORITHM_VERSION: &str = "stock-review-v1";

#[derive(Debug, Clone)]
pub struct CachedCampaignData {
    pub campaign_id: String,
    pub account_id: Option<String>,
    pub symbol: String,
    pub cash_flows: Vec<CampaignCashFlow>,
    pub daily_prices: Vec<CampaignPricePoint>,
    pub expected_session_dates: Vec<NaiveDate>,
    pub benchmark_prices: Vec<LocalPricePoint>,
    pub current_price_local: Option<f64>,
    pub current_fx_to_base: Option<f64>,
    pub issues: Vec<StockReviewIssue>,
}

/// Fully materialized deterministic dependency boundary. Network and SQLite
/// reads belong in `prepare_cached_stock_review_input`; every report consumer
/// calls the same synchronous core below.
#[derive(Debug, Clone)]
pub struct CachedStockReviewInput {
    pub query: StockReviewQuery,
    /// Includes pre-period records required to reconstruct opening positions.
    pub transactions: Vec<Transaction>,
    /// Active, revalidated overrides only.
    pub overrides: Vec<StockReviewOverride>,
    pub persisted_override_issues: Vec<StockReviewIssue>,
    /// Deterministic source/materialization issues discovered before replay.
    pub preparation_issues: Vec<StockReviewIssue>,
    pub result_quality_input: ResultQualityInput,
    pub shadow_input: ShadowPortfolioInput,
    pub actual_comparable: ComparableCurveInput,
    pub comparison_mode: MarketReturnMode,
    pub forward_actions: Vec<ForwardActionInput>,
    pub risk_input: RiskStructureInput,
    pub attribution_input: AttributionInput,
    pub campaign_data: Vec<CachedCampaignData>,
    pub annotations: Vec<StockReviewAnnotation>,
    pub market_data_coverage: Option<f64>,
    pub exchange_rate_coverage: Option<f64>,
    pub benchmark_symbol: Option<String>,
    pub generated_at: String,
}

struct StockReviewArtifacts {
    report: StockReviewReport,
    campaign_details: Vec<StockCampaignDetail>,
}

pub fn build_stock_review_report_from_cached_data(
    input: &CachedStockReviewInput,
) -> Result<StockReviewReport, String> {
    Ok(build_stock_review_artifacts(input)?.report)
}

pub fn build_stock_campaign_detail_from_cached_data(
    input: &CachedStockReviewInput,
    campaign_id: &str,
) -> Result<StockCampaignDetail, String> {
    build_stock_review_artifacts(input)?
        .campaign_details
        .into_iter()
        .find(|detail| detail.summary.campaign_id == campaign_id)
        .ok_or_else(|| format!("Campaign '{campaign_id}' was not found in this report."))
}

fn build_stock_review_artifacts(
    input: &CachedStockReviewInput,
) -> Result<StockReviewArtifacts, String> {
    validate_query(&input.query)?;
    let opening_cash_incomplete = input
        .preparation_issues
        .iter()
        .any(|issue| issue.code == "opening_cash_incomplete");

    // 1. Pre-period transactions remain present for opening reconstruction.
    let scoped_transactions = input
        .transactions
        .iter()
        .filter(|transaction| {
            input
                .query
                .account_id
                .as_ref()
                .is_none_or(|account| transaction.account_id == *account)
                && input
                    .query
                    .market
                    .as_ref()
                    .is_none_or(|market| transaction.market == *market)
                && transaction_date(transaction).is_some_and(|date| date <= input.query.end_date)
        })
        .cloned()
        .collect::<Vec<_>>();

    // 2. Only active overrides reach replay. Stale rows are represented solely
    // by `persisted_override_issues` and therefore cannot affect calculations.
    let action_build = build_stock_actions(&scoped_transactions, &input.overrides);
    // 3. Campaign construction sees the full reconstructed event history.
    let campaign_build = build_stock_campaigns(
        &action_build.position_events,
        &action_build.actions,
        &input.overrides,
        input.query.end_date,
    );

    let period_action_ids = action_build
        .actions
        .iter()
        .filter(|action| !action.fact_labels.iter().any(|label| label == "transfer"))
        .filter(|action| {
            action_date(action)
                .is_some_and(|date| date >= input.query.start_date && date <= input.query.end_date)
        })
        .map(|action| action.action_id.clone())
        .collect::<BTreeSet<_>>();
    let mut actions = action_build
        .actions
        .iter()
        .filter(|action| period_action_ids.contains(&action.action_id))
        .cloned()
        .collect::<Vec<_>>();

    // 4. Actual, shadow and benchmark curves are built before all dependent
    // metrics. The shadow curve is injected into the shared result calculator.
    let shadow = build_shadow_series(&input.shadow_input);
    let exchange_rate_coverage = if shadow.fx_forward_fills.is_empty() {
        input.exchange_rate_coverage
    } else {
        Some(input.exchange_rate_coverage.unwrap_or(0.94).min(0.94))
    };
    let shadow_curve = shadow
        .twr_return_series
        .iter()
        .filter_map(|point| {
            NaiveDate::parse_from_str(&point.date, "%Y-%m-%d")
                .ok()
                .map(|date| CurveReturnPoint {
                    date,
                    cumulative_return: point.cumulative_return / 100.0,
                })
        })
        .collect::<Vec<_>>();
    let mut result_input = input.result_quality_input.clone();
    result_input.shadow_curve = shadow_curve;
    let mut result = calculate_result_quality(&result_input);
    let shadow_return = (!opening_cash_incomplete)
        .then(|| {
            shadow
                .twr_return_series
                .last()
                .map(|point| point.cumulative_return / 100.0)
        })
        .flatten();
    result.metric.shadow_return = shadow_return;
    if opening_cash_incomplete {
        for point in &mut result.normalized_curve {
            point.shadow_index = None;
        }
        if matches!(
            result_input.benchmark_selection,
            BenchmarkSelection::AutomaticMixed
        ) {
            result.metric.benchmark_return = None;
            result.fixed_weights.clear();
            for point in &mut result.normalized_curve {
                point.benchmark_index = None;
            }
        }
    }

    let actual_and_shadow_are_identical = period_action_ids.is_empty()
        && result
            .metric
            .portfolio_return
            .zip(shadow_return)
            .is_some_and(|(actual, shadow)| (actual - shadow).abs() <= 1e-9)
        && result_input
            .actual_values
            .last()
            .map(|point| point.value_base)
            .zip(shadow.ending_value)
            .is_some_and(|(actual, shadow)| (actual - shadow).abs() <= 1e-6);
    let actual_comparable = if input.comparison_mode == MarketReturnMode::TotalReturn {
        ComparableCurveInput {
            mode: MarketReturnMode::TotalReturn,
            return_value: result.metric.portfolio_return,
            ending_value_base: input.actual_comparable.ending_value_base.or_else(|| {
                result_input
                    .actual_values
                    .last()
                    .map(|point| point.value_base)
            }),
        }
    } else if actual_and_shadow_are_identical {
        ComparableCurveInput {
            mode: MarketReturnMode::PriceOnly,
            return_value: shadow_return,
            ending_value_base: shadow.ending_value,
        }
    } else {
        input.actual_comparable.clone()
    };
    let value_add = calculate_rebalance_value_add(&RebalanceValueAddInput {
        actual_recorded_twr: result.metric.portfolio_return,
        actual_comparable,
        shadow_comparable: ComparableCurveInput {
            mode: shadow.return_mode.clone(),
            return_value: shadow_return,
            ending_value_base: shadow.ending_value,
        },
        comparison_mode: input.comparison_mode.clone(),
        availability: if opening_cash_incomplete {
            MetricAvailability {
                status: MetricStatus::Unavailable,
                note: Some(
                    "Shadow value-add is unavailable because opening cash is incomplete."
                        .to_string(),
                ),
            }
        } else {
            shadow.twr_availability.clone()
        },
    });

    let forward_inputs = input
        .forward_actions
        .iter()
        .filter(|action| period_action_ids.contains(&action.action_id))
        .cloned()
        .collect::<Vec<_>>();
    let forward = calculate_forward_effect(&forward_inputs, &[60, 120]);
    enrich_actions(&mut actions, &forward);
    let attribution = calculate_rebalance_attribution(&input.attribution_input);
    attach_action_contributions(&mut actions, &attribution);
    let mut campaign_action_reviews = action_build.actions.clone();
    enrich_actions(&mut campaign_action_reviews, &forward);
    attach_action_contributions(&mut campaign_action_reviews, &attribution);

    // 5. Independent risk/quality regions stay visible under unrelated gaps.
    let risk_structure = calculate_risk_structure(&input.risk_input);
    let risk_metric = risk_metric_from_detail(&risk_structure);
    let mut issues = input.persisted_override_issues.clone();
    issues.extend(input.preparation_issues.clone());
    issues.extend(action_build.issues);
    issues.extend(campaign_build.issues);
    issues.extend(shadow.issues.iter().map(shadow_issue));
    if let Some(fill) = shadow.fx_forward_fills.first() {
        issues.push(StockReviewIssue {
            code: "fx_forward_fill".to_string(),
            severity: StockReviewIssueSeverity::Warning,
            message: format!(
                "{} FX on {} uses the explicitly resolved prior observation from {} ({} calendar days).",
                fill.currency, fill.date, fill.source_date, fill.forward_fill_days
            ),
            affected_symbol: None,
            affected_date: Some(fill.date),
        });
    }
    if actions.is_empty() {
        issues.push(StockReviewIssue {
            code: "no_evaluable_actions".to_string(),
            severity: StockReviewIssueSeverity::Info,
            message: "本期无可评价操作。".to_string(),
            affected_symbol: None,
            affected_date: None,
        });
    }
    let forward_60 = aggregate_window(&forward, 60);
    let forward_120 = aggregate_window(&forward, 120);
    let quality = build_stock_review_quality(&QualityInput {
        market_data_coverage: input.market_data_coverage,
        exchange_rate_coverage,
        attribution_residual: attribution.residual,
        average_portfolio_nav: input.attribution_input.average_portfolio_nav,
        observation_windows: [&forward_60, &forward_120]
            .into_iter()
            .map(|window| ObservationWindowMaturity {
                required_market_sessions: u32::from(window.trading_days),
                elapsed_market_sessions: if window.status.status == MetricStatus::Pending {
                    0
                } else {
                    u32::from(window.trading_days)
                },
                status: window.status.status.clone(),
            })
            .collect(),
        issues,
        actual_result_status: result.metric.availability.status.clone(),
        shadow_value_add_status: value_add.metric.availability.status.clone(),
        attribution_status: attribution.availability.status.clone(),
        interval_drawdown_only: true,
    });
    let summary = StockReviewSummary {
        result_quality: result.metric,
        max_drawdown: result.max_drawdown,
        rebalance_value_add: value_add.metric,
        forward_effect: ForwardEffectMetric {
            availability: merge_window_availability(&forward_60, &forward_120),
            day_60: forward_60,
            day_120: forward_120,
        },
        risk_structure: risk_metric,
    };

    let mut campaigns = campaign_build.campaigns;
    for campaign in &mut campaigns {
        campaign.contribution = campaign
            .action_ids
            .iter()
            .filter_map(|id| {
                attribution
                    .action_contributions
                    .iter()
                    .find(|item| &item.action_id == id)
                    .map(|item| item.amount)
            })
            .reduce(|left, right| left + right);
    }
    let campaign_details = campaigns
        .iter()
        .map(|campaign| {
            let cached = input
                .campaign_data
                .iter()
                .find(|cached| cached.campaign_id == campaign.campaign_id);
            let campaign_actions = campaign_action_reviews
                .iter()
                .filter(|action| campaign.action_ids.contains(&action.action_id))
                .cloned()
                .collect::<Vec<_>>();
            let campaign_forward = input
                .forward_actions
                .iter()
                .filter(|action| campaign.action_ids.contains(&action.action_id))
                .cloned()
                .collect::<Vec<_>>();
            let mut detail = calculate_campaign_detail(&CampaignDetailInput {
                summary: campaign.clone(),
                cash_flows: cached
                    .map(|value| value.cash_flows.clone())
                    .unwrap_or_default(),
                daily_prices: cached
                    .map(|value| value.daily_prices.clone())
                    .unwrap_or_default(),
                expected_session_dates: cached
                    .map(|value| value.expected_session_dates.clone())
                    .unwrap_or_default(),
                benchmark_prices: cached
                    .map(|value| value.benchmark_prices.clone())
                    .unwrap_or_default(),
                current_price_local: cached.and_then(|value| value.current_price_local),
                current_fx_to_base: cached.and_then(|value| value.current_fx_to_base),
                actions: campaign_actions,
                forward_actions: campaign_forward,
                annotations: input
                    .annotations
                    .iter()
                    .filter(|annotation| {
                        annotation.scope_type == "campaign"
                            && annotation.scope_key == campaign.campaign_id
                            || annotation
                                .symbol
                                .as_ref()
                                .is_some_and(|symbol| stock_symbols_equal(symbol, &campaign.symbol))
                    })
                    .cloned()
                    .collect(),
            });
            detail.issues = cached.map(|value| value.issues.clone()).unwrap_or_default();
            detail.issues.extend(
                input
                    .preparation_issues
                    .iter()
                    .filter(|issue| {
                        issue
                            .affected_symbol
                            .as_ref()
                            .is_some_and(|symbol| stock_symbols_equal(symbol, &campaign.symbol))
                    })
                    .cloned(),
            );
            detail
        })
        .collect::<Vec<_>>();

    let fixed_weights = result
        .fixed_weights
        .into_iter()
        .map(|(key, weight)| FixedWeight {
            key,
            weight: Some(weight),
        })
        .collect();
    let curves = result
        .normalized_curve
        .into_iter()
        .map(|point| ReviewCurvePoint {
            date: point.date,
            portfolio_return: point.portfolio_index,
            shadow_return: point.shadow_index,
            benchmark_return: point.benchmark_index,
        })
        .collect();
    let report = StockReviewReport {
        methodology: StockReviewMethodology {
            query: input.query.clone(),
            actual_return_method: "recorded_ledger_twr".to_string(),
            shadow_return_method: match shadow.return_method {
                ShadowReturnMethod::ExplicitDividends => "explicit_dividend_total_return",
                ShadowReturnMethod::AdjustedClose => "adjusted_close_total_return",
                ShadowReturnMethod::PriceOnly => "comparable_price_only",
            }
            .to_string(),
            benchmark_return_method: if input.query.benchmark_symbol.is_some() {
                "fixed_selected_benchmark"
            } else {
                "fixed_opening_weight_mixed_benchmark"
            }
            .to_string(),
            fixed_weights,
            benchmark_symbol: input.benchmark_symbol.clone(),
            market_data_coverage: coverage(input.market_data_coverage),
            exchange_rate_coverage: coverage(exchange_rate_coverage),
            algorithm_version: ALGORITHM_VERSION.to_string(),
        },
        summary,
        curves,
        attribution,
        risk_structure,
        actions,
        campaigns,
        data_quality: quality,
        // Display-only: annotations and imported historical manual assessments
        // never enter any calculator above this line.
        annotations: input.annotations.clone(),
        generated_at: input.generated_at.clone(),
    };
    Ok(StockReviewArtifacts {
        report,
        campaign_details,
    })
}

fn enrich_actions(actions: &mut [StockActionReview], forward: &ForwardEffectOutput) {
    for action in actions {
        let Some(effect) = forward
            .actions
            .iter()
            .find(|effect| effect.action_id == action.action_id)
        else {
            action.status = MetricStatus::Unavailable;
            continue;
        };
        action.observation_windows = effect
            .windows
            .iter()
            .map(|window| ForwardEffectWindow {
                trading_days: window.trading_days,
                status: window.status.clone(),
                matured_actions: usize::from(window.effect.is_some()),
                pending_actions: usize::from(window.status.status == MetricStatus::Pending),
                amount_weighted_excess_return: window.effect,
                positive_notional_ratio: window.effect.map(|effect| f64::from(effect > 0.0)),
            })
            .collect();
        action.status = effect
            .windows
            .iter()
            .map(|window| window.status.status.clone())
            .max_by_key(status_rank)
            .unwrap_or(MetricStatus::Unavailable);
        action.fact_labels.extend(effect.fact_labels.clone());
        action.fact_labels.sort();
        action.fact_labels.dedup();
    }
}

fn attach_action_contributions(
    actions: &mut [StockActionReview],
    attribution: &RebalanceAttributionSummary,
) {
    for action in actions {
        action.contribution = attribution
            .action_contributions
            .iter()
            .find(|item| item.action_id == action.action_id)
            .map(|item| item.amount);
    }
}

fn aggregate_window(forward: &ForwardEffectOutput, days: u16) -> ForwardEffectWindow {
    forward
        .windows
        .iter()
        .find(|window| window.trading_days == days)
        .cloned()
        .unwrap_or_else(|| ForwardEffectWindow {
            trading_days: days,
            status: MetricAvailability {
                status: MetricStatus::Unavailable,
                note: Some("本期无可评价操作。".to_string()),
            },
            matured_actions: 0,
            pending_actions: 0,
            amount_weighted_excess_return: None,
            positive_notional_ratio: None,
        })
}

fn merge_window_availability(
    day_60: &ForwardEffectWindow,
    day_120: &ForwardEffectWindow,
) -> MetricAvailability {
    let status = [day_60.status.status.clone(), day_120.status.status.clone()]
        .into_iter()
        .max_by_key(status_rank)
        .unwrap_or(MetricStatus::Unavailable);
    MetricAvailability {
        note: (status == MetricStatus::Unavailable
            && day_60.matured_actions == 0
            && day_60.pending_actions == 0)
            .then(|| "本期无可评价操作。".to_string()),
        status,
    }
}

fn status_rank(status: &MetricStatus) -> u8 {
    match status {
        MetricStatus::Available => 0,
        MetricStatus::Pending => 1,
        MetricStatus::Degraded => 2,
        MetricStatus::Unavailable => 3,
    }
}

fn risk_metric_from_detail(detail: &RiskStructureDetail) -> RiskStructureMetric {
    RiskStructureMetric {
        availability: detail.availability.clone(),
        opening_max_stock_weight: detail.opening.max_stock_weight,
        ending_max_stock_weight: detail.ending.max_stock_weight,
        opening_cr5: detail.opening.cr5,
        ending_cr5: detail.ending.cr5,
        opening_cash_ratio: detail.opening.cash_ratio,
        ending_cash_ratio: detail.ending.cash_ratio,
        one_way_turnover: detail.one_way_turnover,
        fee_drag: detail.fee_drag,
    }
}

fn coverage(ratio: Option<f64>) -> DataCoverage {
    let status = crate::services::stock_review_quality::classify_coverage_status(ratio);
    DataCoverage {
        availability: MetricAvailability { status, note: None },
        covered_days: ratio.map(|value| (value * 100.0).round() as u32),
        expected_days: ratio.map(|_| 100),
        coverage_ratio: ratio,
    }
}

fn shadow_issue(issue: &ShadowDataIssue) -> StockReviewIssue {
    StockReviewIssue {
        code: format!("shadow_{:?}", issue.kind).to_ascii_lowercase(),
        severity: StockReviewIssueSeverity::Warning,
        message: issue.message.clone(),
        affected_symbol: issue.symbol.clone(),
        affected_date: issue.date,
    }
}

fn action_date(action: &StockActionReview) -> Option<NaiveDate> {
    action
        .traded_at
        .get(..10)
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
}

fn transaction_date(transaction: &Transaction) -> Option<NaiveDate> {
    transaction
        .traded_at
        .get(..10)
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
}

pub fn validate_query(query: &StockReviewQuery) -> Result<(), String> {
    if query.start_date > query.end_date {
        return Err("开始日期不能晚于结束日期。".to_string());
    }
    if !matches!(query.base_currency.as_str(), "USD" | "CNY" | "HKD") {
        return Err("基准币种仅支持 USD、CNY 或 HKD。".to_string());
    }
    if query
        .market
        .as_ref()
        .is_some_and(|market| !matches!(market.as_str(), "US" | "CN" | "HK"))
    {
        return Err("市场仅支持 US、CN 或 HK。".to_string());
    }
    if query
        .account_id
        .as_ref()
        .is_some_and(|id| id.trim().is_empty())
    {
        return Err("账户 ID 不能为空。".to_string());
    }
    if query
        .benchmark_symbol
        .as_ref()
        .is_some_and(|symbol| symbol.trim().is_empty())
    {
        return Err("基准代码不能为空。".to_string());
    }
    Ok(())
}

pub async fn get_stock_review_report(
    db: &Database,
    query: StockReviewQuery,
) -> Result<StockReviewReport, String> {
    let input = prepare_cached_stock_review_input(db, query).await?;
    build_stock_review_report_from_cached_data(&input)
}

pub async fn get_stock_campaign_detail(
    db: &Database,
    query: StockReviewQuery,
    campaign_id: &str,
) -> Result<StockCampaignDetail, String> {
    let input = prepare_cached_stock_review_input(db, query).await?;
    build_stock_campaign_detail_from_cached_data(&input, campaign_id)
}

pub fn save_user_stock_review_annotation(
    db: &Database,
    mut input: StockReviewAnnotationInput,
) -> Result<StockReviewAnnotation, String> {
    input.source = "user".to_string();
    stock_review_persistence::save_annotation(db, input, AnnotationSaveContext::UserInitiated)
}

/// Unconstructable outside this module until a real confirmation interaction
/// supplies the capability in Task 10. It prevents a general command request
/// field from self-authorizing AI-confirmed provenance.
pub(crate) struct ConfirmedAiAnnotationCapability {
    _private: (),
}

pub(crate) fn save_ai_confirmed_stock_review_annotation(
    db: &Database,
    mut input: StockReviewAnnotationInput,
    _capability: &ConfirmedAiAnnotationCapability,
) -> Result<StockReviewAnnotation, String> {
    input.source = "ai_confirmed".to_string();
    stock_review_persistence::save_annotation(
        db,
        input,
        AnnotationSaveContext::AiAfterExplicitUserConfirmation,
    )
}

pub async fn confirm_stock_review_override(
    db: &Database,
    query: StockReviewQuery,
    input: StockReviewOverrideInput,
) -> Result<StockReviewReport, String> {
    // Canonicalization, semantic validation, the active-set revision, and the
    // source fingerprint are captured once before preview.
    let prepared_candidate = stock_review_persistence::prepare_override_candidate(db, input)?;
    let input = prepared_candidate.input.clone();
    let mut cached = prepare_cached_stock_review_input(db, query).await?;
    cached.overrides.retain(|record| record.id != input.id);
    cached.overrides.push(StockReviewOverride {
        id: input.id.clone(),
        override_type: input.override_type.clone(),
        transaction_ids_json: input.transaction_ids_json.clone(),
        value_json: input.value_json.clone(),
        created_at: cached.generated_at.clone(),
        updated_at: cached.generated_at.clone(),
    });
    let referenced_ids = serde_json::from_str::<Vec<String>>(&input.transaction_ids_json)
        .map_err(|error| error.to_string())?;
    if referenced_ids.iter().any(|id| {
        !cached
            .transactions
            .iter()
            .any(|transaction| &transaction.id == id)
    }) {
        return Err(
            "The correction is outside the prepared report scope and cannot be reflected in the candidate report."
                .to_string(),
        );
    }
    let candidate_report = build_stock_review_report_from_cached_data(&cached)?;
    stock_review_persistence::save_override_candidate(db, prepared_candidate)?;
    Ok(candidate_report)
}

async fn prepare_cached_stock_review_input(
    db: &Database,
    query: StockReviewQuery,
) -> Result<CachedStockReviewInput, String> {
    validate_query(&query)?;
    validate_account_exists(db, query.account_id.as_deref())?;
    let override_list = stock_review_persistence::list_overrides(db)?;
    let transactions = load_transactions_for_review(db, query.end_date)?;
    let scoped = transactions
        .iter()
        .filter(|transaction| {
            query
                .account_id
                .as_ref()
                .is_none_or(|account| transaction.account_id == *account)
                && query
                    .market
                    .as_ref()
                    .is_none_or(|market| transaction.market == *market)
        })
        .cloned()
        .collect::<Vec<_>>();
    let action_build = build_stock_actions(&scoped, &override_list.overrides);
    let prepared_campaigns = build_stock_campaigns(
        &action_build.position_events,
        &action_build.actions,
        &override_list.overrides,
        query.end_date,
    );

    let provider_config = crate::services::quote_provider_service::get_quote_provider_config(db)?;
    let mut security_keys = scoped
        .iter()
        .filter(|transaction| !crate::services::quote_service::is_cash_symbol(&transaction.symbol))
        .map(|transaction| (transaction.symbol.clone(), transaction.market.clone()))
        .collect::<BTreeSet<_>>();
    for (symbol, market) in load_current_holding_keys(db, &query)? {
        security_keys.insert((symbol, market));
    }
    let price_end = query.end_date;
    let price_start = scoped
        .iter()
        .filter(|transaction| !crate::services::quote_service::is_cash_symbol(&transaction.symbol))
        .filter_map(transaction_date)
        .min()
        .map_or(query.start_date - Duration::days(10), |date| {
            date.min(query.start_date - Duration::days(10))
        });
    let mut prices_by_security = BTreeMap::new();
    for (symbol, market) in &security_keys {
        let provider = match market.as_str() {
            "US" => &provider_config.us_provider,
            "CN" => &provider_config.cn_provider,
            "HK" => &provider_config.hk_provider,
            _ => continue,
        };
        // A network error is non-fatal. The exact same cache read below decides
        // whether dependent metrics are still usable.
        let _ =
            ensure_stock_price_cache(db, symbol, market, price_start, price_end, provider).await;
        prices_by_security.insert(
            (symbol.clone(), market.clone()),
            load_stock_price_series(db, symbol, market, price_start, price_end)?,
        );
    }

    let benchmark_specs = benchmark_specs(&query);
    let mut benchmark_series = Vec::new();
    let mut benchmark_points_by_market = BTreeMap::new();
    for (market, symbol) in &benchmark_specs {
        let _ = crate::services::performance_service::fetch_benchmark_history(
            db,
            symbol,
            price_start,
            price_end,
        )
        .await;
        let points = load_benchmark_series(db, symbol, price_start, price_end)?;
        let availability = cached_point_availability(&points, query.start_date, query.end_date);
        benchmark_series.push(BenchmarkSeriesInput {
            market: market.clone(),
            availability,
            points: points
                .iter()
                .map(|point| BenchmarkPoint {
                    date: point.date,
                    value: point.close,
                })
                .collect(),
        });
        benchmark_points_by_market.insert(market.clone(), points);
    }
    let local_markets = security_keys
        .iter()
        .map(|(_, market)| market.clone())
        .collect::<BTreeSet<_>>();
    let mut local_benchmark_points_by_market = BTreeMap::new();
    for market in local_markets {
        let Some(symbol) = default_benchmark_symbol(&market) else {
            continue;
        };
        let _ = crate::services::performance_service::fetch_benchmark_history(
            db,
            symbol,
            price_start,
            price_end,
        )
        .await;
        local_benchmark_points_by_market.insert(
            market,
            load_benchmark_series(db, symbol, price_start, price_end)?,
        );
    }

    let (baseline, actual_values, mut actual_availability) = load_actual_values(db, &query)?;
    let actual_origin_date = baseline
        .as_ref()
        .map(|point| point.date)
        .or_else(|| actual_values.first().map(|point| point.date))
        .unwrap_or(query.start_date);
    let (external_flows, external_flows_complete) =
        external_flows_base_from_db(db, &scoped, &query, actual_origin_date);
    if !external_flows_complete {
        actual_availability = MetricAvailability {
            status: MetricStatus::Unavailable,
            note: Some(
                "Actual TWR is unavailable because a non-base external flow lacks cached daily FX."
                    .to_string(),
            ),
        };
    }
    let shadow_external_flows = external_flow_events(&scoped, actual_origin_date, query.end_date);
    let opening_positions = opening_positions(
        db,
        &query,
        &scoped,
        &override_list.overrides,
        actual_origin_date,
    )?;
    let (opening_cash, opening_cash_complete) =
        opening_cash(db, &scoped, &query, actual_origin_date)?;
    let mut preparation_issues = Vec::new();
    if !opening_cash_complete {
        preparation_issues.push(StockReviewIssue {
            code: "opening_cash_incomplete".to_string(),
            severity: StockReviewIssueSeverity::Error,
            message: "Opening cash cannot be reconstructed from a complete cash ledger or an authoritative current cash balance; shadow and fixed-weight benchmark outputs are unavailable.".to_string(),
            affected_symbol: None,
            affected_date: Some(actual_origin_date),
        });
    }
    for market in local_benchmark_points_by_market.keys() {
        preparation_issues.push(StockReviewIssue {
            code: "derived_market_calendar_authority".to_string(),
            severity: StockReviewIssueSeverity::Info,
            message: format!(
                "{market} sessions are derived from the designated broad-market benchmark cache; sparse stock observations never define the calendar."
            ),
            affected_symbol: default_benchmark_symbol(market).map(str::to_string),
            affected_date: None,
        });
    }
    let valuation_dates = std::iter::once(actual_origin_date)
        .chain(actual_values.iter().map(|point| point.date))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let mut fx_points = load_daily_fx_points(
        db,
        &valuation_dates,
        &query.base_currency,
        scoped
            .iter()
            .map(|transaction| transaction.currency.as_str()),
    )?;
    fx_points.extend(load_static_fx_points(
        db,
        actual_origin_date,
        &query.base_currency,
    )?);
    fx_points.sort_by(|left, right| {
        (left.date, &left.currency, &left.base_currency).cmp(&(
            right.date,
            &right.currency,
            &right.base_currency,
        ))
    });
    fx_points.dedup_by(|left, right| {
        left.date == right.date
            && left.currency == right.currency
            && left.base_currency == right.base_currency
    });
    let shadow_prices = prices_by_security
        .iter()
        .flat_map(|((symbol, market), points)| {
            let currency = market_currency(market).to_string();
            points.iter().map(move |point| ShadowPricePoint {
                date: point.date,
                symbol: symbol.clone(),
                market: market.clone(),
                currency: currency.clone(),
                close: point.close,
                adjusted_close: point.adjusted_close,
            })
        })
        .collect::<Vec<_>>();
    let split_events = load_split_events(db, &action_build.position_events, query.end_date)?;
    let dividend_events = load_dividend_events(&scoped, actual_origin_date, query.end_date);
    let return_method = if !dividend_events.is_empty() {
        ShadowReturnMethod::ExplicitDividends
    } else if !shadow_prices.is_empty()
        && shadow_prices
            .iter()
            .all(|point| point.adjusted_close.is_some())
    {
        ShadowReturnMethod::AdjustedClose
    } else {
        ShadowReturnMethod::PriceOnly
    };
    let comparison_mode = if return_method == ShadowReturnMethod::PriceOnly {
        MarketReturnMode::PriceOnly
    } else {
        MarketReturnMode::TotalReturn
    };

    let forward_actions = action_build
        .actions
        .iter()
        .filter_map(|action| {
            let date = action_date(action)?;
            let stock = prices_by_security.get(&(action.symbol.clone(), action.market.clone()))?;
            let benchmark = local_benchmark_points_by_market.get(&action.market)?;
            let fx = resolved_fx_on(
                market_currency(&action.market),
                &query.base_currency,
                &fx_points,
                date,
            );
            Some(ForwardActionInput {
                action_id: action.action_id.clone(),
                action_type: action.action_type.clone(),
                market: action.market.clone(),
                action_date: date,
                action_notional_local: action.gross_amount.unwrap_or(0.0),
                action_day_fx_to_base: fx,
                market_session_dates: benchmark.iter().map(|point| point.date).collect(),
                stock_prices_local: stock
                    .iter()
                    .map(|point| LocalPricePoint {
                        date: point.date,
                        close: point.close,
                    })
                    .collect(),
                benchmark_prices_local: benchmark
                    .iter()
                    .map(|point| LocalPricePoint {
                        date: point.date,
                        close: point.close,
                    })
                    .collect(),
                availability: cached_point_availability(stock, date, price_end),
            })
        })
        .collect::<Vec<_>>();
    let campaign_data = prepared_campaigns
        .campaigns
        .iter()
        .filter_map(|campaign| {
            let symbol = &campaign.symbol;
            let market = &campaign.market;
            let stock = prices_by_security.get(&(symbol.clone(), market.clone()))?;
            let campaign_start = campaign
                .started_at
                .get(..10)
                .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())?;
            let campaign_end = campaign
                .ended_at
                .as_deref()
                .and_then(|value| value.get(..10))
                .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
                .unwrap_or(query.end_date)
                .min(query.end_date);
            let benchmark = local_benchmark_points_by_market.get(market)?;
            let currency = market_currency(market);
            let expected_session_dates = benchmark
                .iter()
                .filter(|point| point.date >= campaign_start && point.date <= campaign_end)
                .map(|point| point.date)
                .collect::<Vec<_>>();
            let mut campaign_issues = Vec::new();
            campaign_issues.push(StockReviewIssue {
                code: "campaign_calendar_authority".to_string(),
                severity: StockReviewIssueSeverity::Info,
                message: format!(
                    "Campaign sessions use the designated {} broad-market calendar.",
                    market
                ),
                affected_symbol: Some(symbol.clone()),
                affected_date: None,
            });
            if expected_session_dates.is_empty() {
                campaign_issues.push(StockReviewIssue {
                    code: "campaign_calendar_unavailable".to_string(),
                    severity: StockReviewIssueSeverity::Error,
                    message: "No authoritative market sessions cover this Campaign lifetime."
                        .to_string(),
                    affected_symbol: Some(symbol.clone()),
                    affected_date: Some(campaign_start),
                });
            } else if let Some(missing_date) = expected_session_dates
                .iter()
                .find(|date| !stock.iter().any(|point| point.date == **date))
            {
                campaign_issues.push(StockReviewIssue {
                    code: "campaign_missing_close".to_string(),
                    severity: StockReviewIssueSeverity::Warning,
                    message: "An expected Campaign market session has no exact stock close; path-dependent metrics are degraded."
                        .to_string(),
                    affected_symbol: Some(symbol.clone()),
                    affected_date: Some(*missing_date),
                });
            }
            let as_of_point = expected_session_dates
                .last()
                .and_then(|date| stock.iter().find(|point| point.date == *date));
            Some(CachedCampaignData {
                campaign_id: campaign.campaign_id.clone(),
                account_id: query.account_id.clone(),
                symbol: symbol.clone(),
                cash_flows: campaign_cash_flows(
                    &scoped,
                    symbol,
                    market,
                    &campaign.account_ids,
                    campaign_start,
                    campaign_end,
                    &query,
                    &fx_points,
                ),
                daily_prices: stock
                    .iter()
                    .filter(|point| point.date >= campaign_start && point.date <= campaign_end)
                    .map(|point| CampaignPricePoint {
                        date: point.date,
                        currency: currency.to_string(),
                        low: point.low,
                        high: point.high,
                        close: Some(point.close),
                        fx_to_base: resolved_fx_on(
                            currency,
                            &query.base_currency,
                            &fx_points,
                            point.date,
                        ),
                    })
                    .collect(),
                expected_session_dates,
                benchmark_prices: benchmark
                    .iter()
                    .filter(|point| point.date >= campaign_start && point.date <= campaign_end)
                    .map(|point| LocalPricePoint {
                        date: point.date,
                        close: point.close,
                    })
                    .collect(),
                current_price_local: as_of_point.map(|point| point.close),
                current_fx_to_base: as_of_point.and_then(|point| {
                    resolved_fx_on(currency, &query.base_currency, &fx_points, point.date)
                }),
                issues: campaign_issues,
            })
        })
        .collect::<Vec<_>>();

    let opening_market_values_base = opening_market_values(
        &opening_positions,
        &prices_by_security,
        &fx_points,
        &query.base_currency,
        actual_origin_date,
    );
    let opening_cash_value_base = opening_cash
        .iter()
        .filter_map(|cash| {
            resolved_fx_on(
                &cash.currency,
                &query.base_currency,
                &fx_points,
                actual_origin_date,
            )
            .map(|fx| cash.amount * fx)
        })
        .sum();
    let benchmark_selection = if query.benchmark_symbol.is_some() {
        BenchmarkSelection::SingleMarket(
            benchmark_specs
                .first()
                .map(|(market, _)| market.clone())
                .unwrap_or_else(|| "CUSTOM".to_string()),
        )
    } else if let Some(market) = &query.market {
        BenchmarkSelection::SingleMarket(market.clone())
    } else {
        BenchmarkSelection::AutomaticMixed
    };
    let average_nav = if actual_values.is_empty() {
        None
    } else {
        Some(
            actual_values
                .iter()
                .map(|point| point.value_base)
                .sum::<f64>()
                / actual_values.len() as f64,
        )
    };
    let shadow_input = ShadowPortfolioInput {
        base_currency: query.base_currency.clone(),
        return_method,
        opening_positions: opening_positions.clone(),
        opening_cash: opening_cash.clone(),
        valuation_dates: valuation_dates.clone(),
        price_points: shadow_prices,
        fx_points: fx_points.clone(),
        external_flows: shadow_external_flows,
        cash_income_events: vec![],
        dividend_events: dividend_events.clone(),
        split_events: split_events.clone(),
    };
    let shadow_preview = build_shadow_series(&shadow_input);
    let attribution_input = load_attribution_input(
        db,
        &query,
        &scoped,
        &action_build.actions,
        &opening_cash,
        &actual_values,
        &prices_by_security,
        &fx_points,
        &split_events,
        &dividend_events,
        &shadow_preview,
        average_nav,
        actual_origin_date,
    )?;
    let risk_input = load_risk_input(db, &query, &action_build.actions, average_nav, &fx_points)?;
    let annotations = load_display_context(db, &query)?;
    let market_data_coverage = aggregate_market_coverage(&prices_by_security, &valuation_dates);
    let exchange_rate_coverage = fx_coverage_for_openings(
        &opening_positions,
        &opening_cash,
        &fx_points,
        &query.base_currency,
        &valuation_dates,
    );
    let actual_ending = actual_values.last().map(|point| point.value_base);

    Ok(CachedStockReviewInput {
        query: query.clone(),
        transactions,
        overrides: override_list.overrides,
        persisted_override_issues: override_list.issues,
        preparation_issues,
        result_quality_input: ResultQualityInput {
            actual_origin_date,
            actual_values,
            baseline,
            external_flows_base: external_flows,
            actual_availability,
            opening_market_values_base,
            opening_cash_value_base,
            benchmark_series,
            benchmark_selection,
            shadow_curve: vec![],
        },
        shadow_input,
        actual_comparable: ComparableCurveInput {
            mode: comparison_mode.clone(),
            return_value: None,
            ending_value_base: actual_ending,
        },
        comparison_mode,
        forward_actions,
        risk_input,
        attribution_input,
        campaign_data,
        annotations,
        market_data_coverage,
        exchange_rate_coverage,
        benchmark_symbol: query.benchmark_symbol.clone().or_else(|| {
            query
                .market
                .as_deref()
                .and_then(default_benchmark_symbol)
                .map(str::to_string)
        }),
        generated_at: Utc::now().to_rfc3339(),
    })
}

fn validate_account_exists(db: &Database, account_id: Option<&str>) -> Result<(), String> {
    let Some(account_id) = account_id else {
        return Ok(());
    };
    let exists: bool = db
        .conn
        .lock()
        .map_err(|error| error.to_string())?
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
            params![account_id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    exists
        .then_some(())
        .ok_or_else(|| format!("账户 '{account_id}' 不存在。"))
}

fn load_transactions_for_review(db: &Database, end: NaiveDate) -> Result<Vec<Transaction>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT id, holding_id, account_id, symbol, name, market, transaction_type,
                    shares, price, total_amount, commission, currency, traded_at, notes, created_at
             FROM transactions WHERE substr(traded_at, 1, 10) <= ?1
             ORDER BY traded_at ASC, created_at ASC, id ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![end.format("%Y-%m-%d").to_string()], |row| {
            Ok(Transaction {
                id: row.get(0)?,
                holding_id: row.get(1)?,
                account_id: row.get(2)?,
                symbol: row.get(3)?,
                name: row.get(4)?,
                market: row.get(5)?,
                transaction_type: row.get(6)?,
                shares: row.get(7)?,
                price: row.get(8)?,
                total_amount: row.get(9)?,
                commission: row.get(10)?,
                currency: row.get(11)?,
                traded_at: row.get(12)?,
                notes: row.get(13)?,
                created_at: row.get(14)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn load_current_holding_keys(
    db: &Database,
    query: &StockReviewQuery,
) -> Result<Vec<(String, String)>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT DISTINCT symbol, market FROM holdings
             WHERE shares > 0
               AND (?1 IS NULL OR account_id = ?1)
               AND (?2 IS NULL OR market = ?2)",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![query.account_id, query.market], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(rows)
}

fn benchmark_specs(query: &StockReviewQuery) -> Vec<(String, String)> {
    if let Some(symbol) = &query.benchmark_symbol {
        return vec![(
            query.market.clone().unwrap_or_else(|| "CUSTOM".to_string()),
            symbol.clone(),
        )];
    }
    if let Some(market) = &query.market {
        return default_benchmark_symbol(market)
            .map(|symbol| vec![(market.clone(), symbol.to_string())])
            .unwrap_or_default();
    }
    ["US", "CN", "HK"]
        .into_iter()
        .filter_map(|market| {
            default_benchmark_symbol(market).map(|symbol| (market.to_string(), symbol.to_string()))
        })
        .collect()
}

fn cached_point_availability(
    points: &[DailyMarketPoint],
    start: NaiveDate,
    end: NaiveDate,
) -> MetricAvailability {
    let exact_start = points.iter().any(|point| point.date == start);
    let exact_end = points.iter().any(|point| point.date == end);
    let (status, note) = if exact_start && exact_end {
        (MetricStatus::Available, None)
    } else if !points.is_empty() {
        (
            MetricStatus::Degraded,
            Some("Cached series does not contain both requested endpoints.".to_string()),
        )
    } else {
        (
            MetricStatus::Unavailable,
            Some("No cached market observations are available.".to_string()),
        )
    };
    MetricAvailability { status, note }
}

fn load_actual_values(
    db: &Database,
    query: &StockReviewQuery,
) -> Result<
    (
        Option<PortfolioValuePoint>,
        Vec<PortfolioValuePoint>,
        MetricAvailability,
    ),
    String,
> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    if query.account_id.is_none() && query.market.is_none() {
        let baseline = conn
            .query_row(
                "SELECT date, total_value, exchange_rates FROM daily_portfolio_values
                 WHERE date < ?1 ORDER BY date DESC LIMIT 1",
                params![query.start_date.format("%Y-%m-%d").to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, f64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .ok()
            .and_then(|(date, value, rates_json)| {
                NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                    .ok()
                    .zip(convert_snapshot_value(
                        value,
                        &rates_json,
                        &query.base_currency,
                    ))
                    .map(|(date, value_base)| PortfolioValuePoint { date, value_base })
            });
        let mut statement = conn
            .prepare(
                "SELECT date, total_value, exchange_rates FROM daily_portfolio_values
                 WHERE date BETWEEN ?1 AND ?2 ORDER BY date ASC",
            )
            .map_err(|error| error.to_string())?;
        let values = statement
            .query_map(
                params![
                    query.start_date.format("%Y-%m-%d").to_string(),
                    query.end_date.format("%Y-%m-%d").to_string()
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, f64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?
            .filter_map(|row| row.ok())
            .filter_map(|(date, value, rates_json)| {
                NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                    .ok()
                    .zip(convert_snapshot_value(
                        value,
                        &rates_json,
                        &query.base_currency,
                    ))
                    .map(|(date, value_base)| PortfolioValuePoint { date, value_base })
            })
            .collect::<Vec<_>>();
        let availability = if baseline.is_some() && !values.is_empty() {
            MetricAvailability {
                status: MetricStatus::Available,
                note: None,
            }
        } else {
            MetricAvailability {
                status: MetricStatus::Unavailable,
                note: Some(
                    "Recorded portfolio snapshots do not cover the requested period.".to_string(),
                ),
            }
        };
        return Ok((baseline, values, availability));
    }
    // Filtered daily snapshots do not contain account cash. Preserve their
    // stock value path for context, but do not claim an authoritative TWR.
    let mut statement = conn
        .prepare(
            "SELECT date, SUM(market_value) FROM daily_holding_snapshots
             WHERE date BETWEEN ?1 AND ?2
               AND (?3 IS NULL OR account_id = ?3)
               AND (?4 IS NULL OR market = ?4)
             GROUP BY date ORDER BY date ASC",
        )
        .map_err(|error| error.to_string())?;
    let values = statement
        .query_map(
            params![
                query.start_date.format("%Y-%m-%d").to_string(),
                query.end_date.format("%Y-%m-%d").to_string(),
                query.account_id,
                query.market,
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)),
        )
        .map_err(|error| error.to_string())?
        .filter_map(|row| row.ok())
        .filter_map(|(date, value)| {
            NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .ok()
                .map(|date| PortfolioValuePoint {
                    date,
                    value_base: value,
                })
        })
        .collect();
    Ok((
        None,
        values,
        MetricAvailability {
            status: MetricStatus::Unavailable,
            note: Some("Filtered snapshots lack an authoritative daily cash ledger; actual TWR is unavailable.".to_string()),
        },
    ))
}

fn convert_snapshot_value(value_usd: f64, rates_json: &str, base_currency: &str) -> Option<f64> {
    if base_currency == "USD" {
        return value_usd.is_finite().then_some(value_usd);
    }
    let rates = serde_json::from_str::<crate::models::ExchangeRates>(rates_json).ok()?;
    let converted = crate::services::exchange_rate_service::convert_currency(
        value_usd,
        "USD",
        base_currency,
        &rates,
    );
    converted.is_finite().then_some(converted)
}

fn opening_positions(
    db: &Database,
    query: &StockReviewQuery,
    transactions: &[Transaction],
    overrides: &[StockReviewOverride],
    origin: NaiveDate,
) -> Result<Vec<OpeningPosition>, String> {
    let replay = build_stock_actions(transactions, overrides);
    let mut latest =
        BTreeMap::<(String, String), &crate::services::stock_action_builder::PositionEvent>::new();
    for event in replay
        .position_events
        .iter()
        .filter(|event| event.trade_date <= origin)
    {
        latest.insert(
            (
                event.account_id.clone(),
                normalized_stock_symbol(&event.symbol).unwrap_or_default(),
            ),
            event,
        );
    }
    let mut positions = latest
        .into_values()
        .filter(|event| event.shares_after > 0.0)
        .map(|event| OpeningPosition {
            account_id: event.account_id.clone(),
            symbol: event.symbol.clone(),
            market: event.market.clone(),
            currency: market_currency(&event.market).to_string(),
            quantity: event.shares_after,
        })
        .collect::<Vec<_>>();
    if positions.is_empty()
        && replay
            .position_events
            .iter()
            .all(|event| event.trade_date >= query.start_date)
    {
        let conn = db.conn.lock().map_err(|error| error.to_string())?;
        let mut statement = conn
            .prepare(
                "SELECT account_id, symbol, market, currency, shares FROM holdings
                 WHERE shares > 0 AND symbol NOT LIKE '$CASH-%'
                   AND (?1 IS NULL OR account_id = ?1) AND (?2 IS NULL OR market = ?2)",
            )
            .map_err(|error| error.to_string())?;
        positions = statement
            .query_map(params![query.account_id, query.market], |row| {
                Ok(OpeningPosition {
                    account_id: row.get(0)?,
                    symbol: row.get(1)?,
                    market: row.get(2)?,
                    currency: row.get(3)?,
                    quantity: row.get(4)?,
                })
            })
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
    }
    Ok(positions)
}

fn opening_cash(
    db: &Database,
    transactions: &[Transaction],
    query: &StockReviewQuery,
    origin: NaiveDate,
) -> Result<(Vec<OpeningCashBalance>, bool), String> {
    let mut ledger_balances = BTreeMap::<(String, String), f64>::new();
    let mut ledger_anchor_keys = BTreeSet::<(String, String)>::new();
    let mut required_keys = BTreeSet::<(String, String)>::new();
    for transaction in transactions
        .iter()
        .filter(|transaction| transaction_date(transaction).is_some_and(|date| date <= origin))
    {
        let key = (transaction.account_id.clone(), transaction.currency.clone());
        required_keys.insert(key.clone());
        if crate::services::quote_service::is_cash_symbol(&transaction.symbol) {
            ledger_anchor_keys.insert(key.clone());
        }
        let delta = crate::commands::transactions::cash_delta(
            &transaction.transaction_type,
            &transaction.symbol,
            transaction.total_amount,
            transaction.commission,
        );
        *ledger_balances.entry(key).or_default() += delta;
    }

    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT account_id, currency, shares FROM holdings
             WHERE symbol LIKE '$CASH-%'
               AND (?1 IS NULL OR account_id = ?1)
               AND (?2 IS NULL OR market = ?2)",
        )
        .map_err(|error| error.to_string())?;
    let current_cash = statement
        .query_map(params![query.account_id, query.market], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    let mut later_statement = conn
        .prepare(
            "SELECT account_id, currency, symbol, transaction_type, total_amount, commission
             FROM transactions
             WHERE substr(traded_at, 1, 10) > ?1
               AND (?2 IS NULL OR account_id = ?2)
               AND (?3 IS NULL OR market = ?3)",
        )
        .map_err(|error| error.to_string())?;
    let later_transactions = later_statement
        .query_map(
            params![
                origin.format("%Y-%m-%d").to_string(),
                query.account_id,
                query.market
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, f64>(4)?,
                    row.get::<_, f64>(5)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(later_statement);

    let mut authoritative = BTreeMap::new();
    for (account_id, currency, current_amount) in current_cash {
        let later_delta = later_transactions
            .iter()
            .filter(|(candidate_account, candidate_currency, _, _, _, _)| {
                candidate_account == &account_id && candidate_currency == &currency
            })
            .map(
                |(_, _, symbol, transaction_type, total_amount, commission)| {
                    crate::commands::transactions::cash_delta(
                        transaction_type,
                        symbol,
                        *total_amount,
                        *commission,
                    )
                },
            )
            .sum::<f64>();
        authoritative.insert((account_id, currency), current_amount - later_delta);
    }
    drop(conn);

    let keys = required_keys
        .iter()
        .cloned()
        .chain(authoritative.keys().cloned())
        .collect::<BTreeSet<_>>();
    let complete = keys
        .iter()
        .all(|key| ledger_anchor_keys.contains(key) || authoritative.contains_key(key));
    let balances = keys
        .into_iter()
        .filter_map(|(account_id, currency)| {
            let key = (account_id.clone(), currency.clone());
            let amount = if ledger_anchor_keys.contains(&key) {
                ledger_balances.get(&key).copied()
            } else {
                authoritative.get(&key).copied()
            }?;
            Some(OpeningCashBalance {
                account_id,
                currency,
                amount,
            })
        })
        .collect();
    Ok((balances, complete))
}

fn external_flows_base(
    transactions: &[Transaction],
    query: &StockReviewQuery,
    origin: NaiveDate,
) -> Vec<ExternalFlowBase> {
    let mut flows = BTreeMap::new();
    for transaction in transactions.iter().filter(|transaction| {
        transaction_date(transaction).is_some_and(|date| date > origin && date <= query.end_date)
            && (crate::services::quote_service::is_cash_symbol(&transaction.symbol)
                || transaction.transaction_type == "OPEN")
            && transaction.currency == query.base_currency
            && query
                .account_id
                .as_ref()
                .is_none_or(|account| transaction.account_id == *account)
            && query
                .market
                .as_ref()
                .is_none_or(|market| transaction.market == *market)
    }) {
        let amount = match transaction.transaction_type.as_str() {
            "BUY" | "OPEN" => transaction.total_amount + transaction.commission,
            "SELL" => -(transaction.total_amount + transaction.commission),
            _ => continue,
        };
        *flows
            .entry(transaction_date(transaction).unwrap())
            .or_insert(0.0) += amount;
    }
    flows
        .into_iter()
        .map(|(date, amount_base)| ExternalFlowBase { date, amount_base })
        .collect()
}

fn external_flows_base_from_db(
    db: &Database,
    transactions: &[Transaction],
    query: &StockReviewQuery,
    origin: NaiveDate,
) -> (Vec<ExternalFlowBase>, bool) {
    let conn = match db.conn.lock() {
        Ok(conn) => conn,
        Err(_) => return (vec![], false),
    };
    let mut grouped = BTreeMap::new();
    let mut complete = true;
    for transaction in transactions.iter().filter(|transaction| {
        transaction_date(transaction).is_some_and(|date| date > origin && date <= query.end_date)
            && (crate::services::quote_service::is_cash_symbol(&transaction.symbol)
                || transaction.transaction_type == "OPEN")
            && query
                .account_id
                .as_ref()
                .is_none_or(|account| transaction.account_id == *account)
            && query
                .market
                .as_ref()
                .is_none_or(|market| transaction.market == *market)
    }) {
        let local_amount = match transaction.transaction_type.as_str() {
            "BUY" | "OPEN" => transaction.total_amount + transaction.commission,
            "SELL" => -(transaction.total_amount + transaction.commission),
            _ => continue,
        };
        let date = transaction_date(transaction).expect("filtered transaction has a date");
        let amount_base = if transaction.currency == query.base_currency {
            Some(local_amount)
        } else {
            conn.query_row(
                "SELECT exchange_rates FROM daily_portfolio_values
                 WHERE date >= ?1 AND date <= ?2 ORDER BY date ASC LIMIT 1",
                params![
                    date.format("%Y-%m-%d").to_string(),
                    query.end_date.format("%Y-%m-%d").to_string()
                ],
                |row| row.get::<_, String>(0),
            )
            .ok()
            .and_then(|json| serde_json::from_str::<crate::models::ExchangeRates>(&json).ok())
            .filter(|rates| rates.usd_cny > 0.0 && rates.usd_hkd > 0.0)
            .map(|rates| {
                crate::services::exchange_rate_service::convert_currency(
                    local_amount,
                    &transaction.currency,
                    &query.base_currency,
                    &rates,
                )
            })
            .filter(|amount| amount.is_finite())
        };
        if let Some(amount) = amount_base {
            *grouped.entry(date).or_insert(0.0) += amount;
        } else {
            complete = false;
        }
    }
    (
        grouped
            .into_iter()
            .map(|(date, amount_base)| ExternalFlowBase { date, amount_base })
            .collect(),
        complete,
    )
}

fn external_flow_events(
    transactions: &[Transaction],
    origin: NaiveDate,
    end: NaiveDate,
) -> Vec<crate::services::shadow_portfolio_engine::ExternalFlowEvent> {
    transactions
        .iter()
        .filter(|transaction| {
            transaction_date(transaction).is_some_and(|date| date > origin && date <= end)
                && (crate::services::quote_service::is_cash_symbol(&transaction.symbol)
                    || transaction.transaction_type == "OPEN")
        })
        .filter_map(|transaction| {
            let amount = match transaction.transaction_type.as_str() {
                "BUY" | "OPEN" => transaction.total_amount + transaction.commission,
                "SELL" => -(transaction.total_amount + transaction.commission),
                _ => return None,
            };
            Some(
                crate::services::shadow_portfolio_engine::ExternalFlowEvent {
                    date: transaction_date(transaction)?,
                    account_id: transaction.account_id.clone(),
                    currency: transaction.currency.clone(),
                    amount,
                },
            )
        })
        .collect()
}

fn market_currency(market: &str) -> &'static str {
    match market {
        "CN" => "CNY",
        "HK" => "HKD",
        _ => "USD",
    }
}

fn load_static_fx_points(
    db: &Database,
    _requested_origin: NaiveDate,
    base_currency: &str,
) -> Result<Vec<crate::services::shadow_portfolio_engine::ShadowFxPoint>, String> {
    let Some(rates) = crate::services::exchange_rate_service::load_exchange_rates_from_db(db)?
    else {
        return Ok(vec![]);
    };
    let source_date = chrono::DateTime::parse_from_rfc3339(&rates.updated_at)
        .ok()
        .map(|value| value.date_naive())
        .or_else(|| {
            rates
                .updated_at
                .get(..10)
                .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
        });
    let Some(source_date) = source_date else {
        return Ok(vec![]);
    };
    Ok(["USD", "CNY", "HKD"]
        .into_iter()
        .filter(|currency| *currency != base_currency)
        .filter_map(|currency| {
            let rate = crate::services::exchange_rate_service::convert_currency(
                1.0,
                currency,
                base_currency,
                &rates,
            );
            (rate.is_finite() && rate > 0.0)
                .then_some(rate)
                .map(
                    |rate| crate::services::shadow_portfolio_engine::ShadowFxPoint {
                        date: source_date,
                        currency: currency.to_string(),
                        base_currency: base_currency.to_string(),
                        rate,
                    },
                )
        })
        .collect::<Vec<_>>())
}

fn load_daily_fx_points<'a>(
    db: &Database,
    dates: &[NaiveDate],
    base_currency: &str,
    currencies: impl Iterator<Item = &'a str>,
) -> Result<Vec<ShadowFxPoint>, String> {
    let required = currencies
        .filter(|currency| *currency != base_currency)
        .map(str::to_string)
        .collect::<BTreeSet<_>>();
    if required.is_empty() || dates.is_empty() {
        return Ok(Vec::new());
    }
    let first = dates.first().unwrap().format("%Y-%m-%d").to_string();
    let last = dates.last().unwrap().format("%Y-%m-%d").to_string();
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT date, exchange_rates FROM daily_portfolio_values
             WHERE date BETWEEN ?1 AND ?2 ORDER BY date ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![first, last], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let date_set = dates.iter().copied().collect::<BTreeSet<_>>();
    let mut points = Vec::new();
    for (date, json) in rows {
        let Some(date) = NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok() else {
            continue;
        };
        if !date_set.contains(&date) {
            continue;
        }
        let Some(rates) = serde_json::from_str::<crate::models::ExchangeRates>(&json).ok() else {
            continue;
        };
        for currency in &required {
            let rate = crate::services::exchange_rate_service::convert_currency(
                1.0,
                currency,
                base_currency,
                &rates,
            );
            if rate.is_finite() && rate > 0.0 {
                points.push(ShadowFxPoint {
                    date,
                    currency: currency.clone(),
                    base_currency: base_currency.to_string(),
                    rate,
                });
            }
        }
    }
    Ok(points)
}

fn load_split_events(
    db: &Database,
    position_events: &[crate::services::stock_action_builder::PositionEvent],
    end: NaiveDate,
) -> Result<Vec<SplitEvent>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT stock_code, split_date, ratio_from, ratio_to FROM stock_splits
             WHERE split_date <= ?1 ORDER BY split_date ASC, id ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![end.format("%Y-%m-%d").to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, f64>(2)?,
                row.get::<_, f64>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut events = Vec::new();
    for (symbol, date, ratio_from, ratio_to) in rows {
        let Some(date) = NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok() else {
            continue;
        };
        let ratio = ratio_to / ratio_from;
        if !ratio.is_finite() || ratio <= 0.0 {
            continue;
        }
        let positions = position_events
            .iter()
            .filter(|event| stock_symbols_equal(&event.symbol, &symbol) && event.trade_date < date)
            .fold(
                BTreeMap::<
                    (String, String, String),
                    &crate::services::stock_action_builder::PositionEvent,
                >::new(),
                |mut latest, event| {
                    latest.insert(
                        (
                            event.account_id.clone(),
                            event.symbol.clone(),
                            event.market.clone(),
                        ),
                        event,
                    );
                    latest
                },
            );
        events.extend(
            positions
                .into_values()
                .filter(|event| event.shares_after > 0.0)
                .map(|event| SplitEvent {
                    date,
                    account_id: event.account_id.clone(),
                    symbol: event.symbol.clone(),
                    market: event.market.clone(),
                    ratio,
                }),
        );
    }
    Ok(events)
}

fn load_dividend_events(
    transactions: &[Transaction],
    origin: NaiveDate,
    end: NaiveDate,
) -> Vec<DividendEvent> {
    transactions
        .iter()
        .filter(|transaction| {
            transaction.transaction_type == "PAY"
                && transaction.shares > 0.0
                && transaction_date(transaction).is_some_and(|date| date > origin && date <= end)
        })
        .filter_map(|transaction| {
            let net = transaction.total_amount - transaction.commission;
            (net.is_finite() && net >= 0.0).then(|| DividendEvent {
                date: transaction_date(transaction).unwrap(),
                account_id: transaction.account_id.clone(),
                symbol: transaction.symbol.clone(),
                market: transaction.market.clone(),
                currency: transaction.currency.clone(),
                amount_per_share: net / transaction.shares,
            })
        })
        .collect()
}

fn resolved_fx_on(
    currency: &str,
    base_currency: &str,
    points: &[crate::services::shadow_portfolio_engine::ShadowFxPoint],
    date: NaiveDate,
) -> Option<f64> {
    if currency == base_currency {
        return Some(1.0);
    }
    points
        .iter()
        .filter(|point| {
            point.currency == currency && point.base_currency == base_currency && point.date <= date
        })
        .max_by_key(|point| point.date)
        .map(|point| point.rate)
}

fn campaign_cash_flows(
    transactions: &[Transaction],
    symbol: &str,
    market: &str,
    account_ids: &[String],
    start: NaiveDate,
    end: NaiveDate,
    query: &StockReviewQuery,
    fx_points: &[crate::services::shadow_portfolio_engine::ShadowFxPoint],
) -> Vec<CampaignCashFlow> {
    let mut flows = Vec::new();
    for transaction in transactions.iter().filter(|transaction| {
        stock_symbols_equal(&transaction.symbol, symbol)
            && transaction.market == market
            && account_ids.contains(&transaction.account_id)
            && transaction_date(transaction).is_some_and(|date| date >= start && date <= end)
    }) {
        let Some(date) = transaction_date(transaction) else {
            continue;
        };
        let Some(fx) = resolved_fx_on(&transaction.currency, &query.base_currency, fx_points, date)
        else {
            continue;
        };
        let action_id = match transaction.transaction_type.as_str() {
            "BUY" => Some(format!(
                "action:{}:{}:{}:buy:{}",
                transaction.account_id, transaction.symbol, date, transaction.id
            )),
            "SELL" => Some(format!(
                "action:{}:{}:{}:sell:{}",
                transaction.account_id, transaction.symbol, date, transaction.id
            )),
            _ => None,
        };
        match transaction.transaction_type.as_str() {
            "BUY" => flows.push(CampaignTimelineItem {
                date,
                kind: CampaignCashFlowKind::Buy,
                amount_base: transaction.total_amount * fx,
                shares: transaction.shares,
                account_id: transaction.account_id.clone(),
                action_id: action_id.clone(),
            }),
            "SELL" => flows.push(CampaignTimelineItem {
                date,
                kind: CampaignCashFlowKind::Sell,
                amount_base: transaction.total_amount * fx,
                shares: transaction.shares,
                account_id: transaction.account_id.clone(),
                action_id: action_id.clone(),
            }),
            "PAY" => flows.push(CampaignTimelineItem {
                date,
                kind: CampaignCashFlowKind::Dividend,
                amount_base: transaction.total_amount * fx,
                shares: 0.0,
                account_id: transaction.account_id.clone(),
                action_id: None,
            }),
            _ => {}
        }
        if transaction.commission > 0.0
            && matches!(transaction.transaction_type.as_str(), "BUY" | "SELL")
        {
            flows.push(CampaignTimelineItem {
                date,
                kind: CampaignCashFlowKind::Fee,
                amount_base: transaction.commission * fx,
                shares: 0.0,
                account_id: transaction.account_id.clone(),
                action_id,
            });
        }
    }
    flows.sort_by_key(|flow| flow.date);
    flows
}

fn opening_market_values(
    positions: &[OpeningPosition],
    prices: &BTreeMap<(String, String), Vec<DailyMarketPoint>>,
    fx_points: &[crate::services::shadow_portfolio_engine::ShadowFxPoint],
    base_currency: &str,
    date: NaiveDate,
) -> Vec<MarketValue> {
    let mut values = BTreeMap::new();
    for position in positions {
        let price = prices
            .get(&(position.symbol.clone(), position.market.clone()))
            .and_then(|points| points.iter().find(|point| point.date == date))
            .map(|point| point.close);
        if let Some(value) = price
            .zip(resolved_fx_on(
                &position.currency,
                base_currency,
                fx_points,
                date,
            ))
            .map(|(price, fx)| position.quantity * price * fx)
        {
            *values.entry(position.market.clone()).or_insert(0.0) += value;
        }
    }
    values
        .into_iter()
        .map(|(market, value_base)| MarketValue { market, value_base })
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn load_attribution_input(
    db: &Database,
    query: &StockReviewQuery,
    transactions: &[Transaction],
    actions: &[StockActionReview],
    opening_cash: &[OpeningCashBalance],
    actual_values: &[PortfolioValuePoint],
    prices_by_security: &BTreeMap<(String, String), Vec<DailyMarketPoint>>,
    fx_points: &[ShadowFxPoint],
    split_events: &[SplitEvent],
    dividend_events: &[DividendEvent],
    shadow: &ShadowPortfolioResult,
    average_nav: Option<f64>,
    origin: NaiveDate,
) -> Result<AttributionInput, String> {
    let dates = actual_values
        .iter()
        .map(|point| point.date)
        .collect::<BTreeSet<_>>();
    if dates.len() < 2 {
        return Ok(empty_attribution_input(&query.base_currency));
    }
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT date, account_id, symbol, market, shares FROM daily_holding_snapshots
             WHERE date BETWEEN ?1 AND ?2
               AND (?3 IS NULL OR account_id = ?3)
               AND (?4 IS NULL OR market = ?4)
             ORDER BY date ASC, account_id ASC, symbol ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![
                dates.first().unwrap().format("%Y-%m-%d").to_string(),
                dates.last().unwrap().format("%Y-%m-%d").to_string(),
                query.account_id,
                query.market,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, f64>(4)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    drop(statement);
    drop(conn);

    type PositionKey = (String, String, String, String);
    type CashKey = (String, String);
    let mut actual_positions = BTreeMap::<NaiveDate, BTreeMap<PositionKey, f64>>::new();
    for (date, account_id, symbol, market, shares) in rows {
        let Some(date) = NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok() else {
            continue;
        };
        if dates.contains(&date) {
            actual_positions.entry(date).or_default().insert(
                (
                    account_id,
                    symbol,
                    market.clone(),
                    market_currency(&market).to_string(),
                ),
                shares,
            );
        }
    }
    let mut shadow_positions = BTreeMap::<NaiveDate, BTreeMap<PositionKey, f64>>::new();
    let mut shadow_cash = BTreeMap::<NaiveDate, BTreeMap<CashKey, f64>>::new();
    for valuation in shadow
        .daily_valuations
        .iter()
        .filter(|valuation| dates.contains(&valuation.date))
    {
        for position in &valuation.positions {
            shadow_positions.entry(valuation.date).or_default().insert(
                (
                    position.account_id.clone(),
                    position.symbol.clone(),
                    position.market.clone(),
                    position.currency.clone(),
                ),
                position.quantity,
            );
        }
        for cash in &valuation.cash_balances {
            shadow_cash.entry(valuation.date).or_default().insert(
                (cash.account_id.clone(), cash.currency.clone()),
                cash.amount,
            );
        }
    }
    let mut actual_cash = BTreeMap::<NaiveDate, BTreeMap<CashKey, f64>>::new();
    for date in &dates {
        let mut balances = opening_cash
            .iter()
            .map(|cash| {
                (
                    (cash.account_id.clone(), cash.currency.clone()),
                    cash.amount,
                )
            })
            .collect::<BTreeMap<_, _>>();
        for transaction in transactions.iter().filter(|transaction| {
            transaction_date(transaction)
                .is_some_and(|trade_date| trade_date > origin && trade_date <= *date)
        }) {
            let delta = crate::commands::transactions::cash_delta(
                &transaction.transaction_type,
                &transaction.symbol,
                transaction.total_amount,
                transaction.commission,
            );
            *balances
                .entry((transaction.account_id.clone(), transaction.currency.clone()))
                .or_default() += delta;
        }
        actual_cash.insert(*date, balances);
    }

    let batches = actions
        .iter()
        .filter(|action| {
            !action.fact_labels.iter().any(|label| label == "transfer")
                && action_date(action)
                    .is_some_and(|date| date >= query.start_date && date <= query.end_date)
        })
        .filter_map(|action| {
            Some(AttributionBatch::new(
                &action.action_id,
                &action.account_id,
                &action.symbol,
                &action.market,
                action
                    .currency
                    .as_deref()
                    .unwrap_or(market_currency(&action.market)),
                &action.action_type,
                action_date(action)?,
                action.shares_after? - action.shares_before?,
            ))
        })
        .collect::<Vec<_>>();
    let position_keys = dates
        .iter()
        .flat_map(|date| {
            actual_positions
                .get(date)
                .into_iter()
                .flat_map(|positions| positions.keys().cloned())
                .chain(
                    shadow_positions
                        .get(date)
                        .into_iter()
                        .flat_map(|positions| positions.keys().cloned()),
                )
        })
        .chain(batches.iter().map(|batch| {
            (
                batch.account_id.clone(),
                batch.symbol.clone(),
                batch.market.clone(),
                batch.currency.clone(),
            )
        }))
        .collect::<BTreeSet<_>>();
    let cash_keys = dates
        .iter()
        .flat_map(|date| {
            actual_cash
                .get(date)
                .into_iter()
                .flat_map(|cash| cash.keys().cloned())
                .chain(
                    shadow_cash
                        .get(date)
                        .into_iter()
                        .flat_map(|cash| cash.keys().cloned()),
                )
        })
        .collect::<BTreeSet<_>>();
    let valuations = dates
        .iter()
        .map(|date| AttributionValuationPoint {
            date: *date,
            positions: position_keys
                .iter()
                .map(
                    |(account_id, symbol, market, currency)| AttributionPositionBalance {
                        account_id: account_id.clone(),
                        symbol: symbol.clone(),
                        market: market.clone(),
                        currency: currency.clone(),
                        actual_quantity: actual_positions
                            .get(date)
                            .and_then(|positions| {
                                positions.get(&(
                                    account_id.clone(),
                                    symbol.clone(),
                                    market.clone(),
                                    currency.clone(),
                                ))
                            })
                            .copied()
                            .unwrap_or(0.0),
                        shadow_quantity: shadow_positions
                            .get(date)
                            .and_then(|positions| {
                                positions.get(&(
                                    account_id.clone(),
                                    symbol.clone(),
                                    market.clone(),
                                    currency.clone(),
                                ))
                            })
                            .copied()
                            .unwrap_or(0.0),
                    },
                )
                .collect(),
            cash_balances: cash_keys
                .iter()
                .map(|(account_id, currency)| AttributionCashBalance {
                    account_id: account_id.clone(),
                    currency: currency.clone(),
                    actual_amount: actual_cash
                        .get(date)
                        .and_then(|cash| cash.get(&(account_id.clone(), currency.clone())))
                        .copied()
                        .unwrap_or(0.0),
                    shadow_amount: shadow_cash
                        .get(date)
                        .and_then(|cash| cash.get(&(account_id.clone(), currency.clone())))
                        .copied()
                        .unwrap_or(0.0),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let prices = position_keys
        .iter()
        .flat_map(|(_, symbol, market, currency)| {
            prices_by_security
                .get(&(symbol.clone(), market.clone()))
                .into_iter()
                .flat_map(|points| {
                    points
                        .iter()
                        .filter(|point| dates.contains(&point.date))
                        .map(|point| {
                            AttributionPricePoint::new(
                                point.date,
                                symbol,
                                market,
                                currency,
                                point.close,
                            )
                        })
                })
        })
        .collect::<Vec<_>>();
    let fx_rates = fx_points
        .iter()
        .filter(|point| dates.contains(&point.date))
        .map(|point| {
            AttributionFxPoint::new(
                point.date,
                &point.currency,
                &point.base_currency,
                point.rate,
            )
        })
        .collect::<Vec<_>>();
    let splits = split_events
        .iter()
        .map(|event| {
            AttributionSplit::new(
                event.date,
                &event.account_id,
                &event.symbol,
                &event.market,
                event.ratio,
            )
        })
        .collect();
    let dividends = dividend_events
        .iter()
        .map(|event| {
            AttributionDividend::new(
                event.date,
                &event.symbol,
                &event.market,
                &event.currency,
                event.amount_per_share,
            )
        })
        .collect();
    let fees = actions
        .iter()
        .filter(|action| {
            batches
                .iter()
                .any(|batch| batch.action_id == action.action_id)
        })
        .filter_map(|action| {
            Some(AttributionFee::new(
                action_date(action)?,
                &action.action_id,
                action
                    .currency
                    .as_deref()
                    .unwrap_or(market_currency(&action.market)),
                action.fees?,
            ))
        })
        .collect();
    let cash_currencies = cash_keys
        .iter()
        .map(|(_, currency)| currency.clone())
        .collect::<BTreeSet<_>>();
    let cash_returns = dates
        .iter()
        .skip(1)
        .flat_map(|date| {
            cash_currencies
                .iter()
                .map(|currency| AttributionCashReturn::new(*date, currency, 0.0))
        })
        .collect();
    Ok(AttributionInput {
        base_currency: query.base_currency.clone(),
        average_portfolio_nav: average_nav,
        valuations,
        prices,
        fx_rates,
        batches,
        splits,
        dividends,
        fees,
        cash_returns,
    })
}

fn empty_attribution_input(base_currency: &str) -> AttributionInput {
    AttributionInput {
        base_currency: base_currency.to_string(),
        average_portfolio_nav: None,
        valuations: vec![],
        prices: vec![],
        fx_rates: vec![],
        batches: vec![],
        splits: vec![],
        dividends: vec![],
        fees: vec![],
        cash_returns: vec![],
    }
}

fn load_risk_input(
    db: &Database,
    query: &StockReviewQuery,
    actions: &[StockActionReview],
    average_nav: Option<f64>,
    fx_points: &[ShadowFxPoint],
) -> Result<RiskStructureInput, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT date, symbol, market, market_value FROM daily_holding_snapshots
             WHERE date BETWEEN ?1 AND ?2 AND (?3 IS NULL OR account_id = ?3) AND (?4 IS NULL OR market = ?4)
             ORDER BY date ASC, symbol ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![
                query.start_date.format("%Y-%m-%d").to_string(),
                query.end_date.format("%Y-%m-%d").to_string(),
                query.account_id,
                query.market
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, f64>(3)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut by_date = BTreeMap::<NaiveDate, Vec<StockValueBase>>::new();
    let mut conversion_complete = BTreeMap::<NaiveDate, bool>::new();
    for (date, symbol, market, value) in rows {
        let Some(date) = NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok() else {
            continue;
        };
        let Some(fx) = resolved_fx_on(
            market_currency(&market),
            &query.base_currency,
            fx_points,
            date,
        ) else {
            conversion_complete.insert(date, false);
            continue;
        };
        conversion_complete.entry(date).or_insert(true);
        by_date.entry(date).or_default().push(StockValueBase {
            symbol,
            value_base: value * fx,
        });
    }
    let portfolio_totals = if query.account_id.is_none() && query.market.is_none() {
        let mut statement = conn
            .prepare(
                "SELECT date, total_value, exchange_rates FROM daily_portfolio_values
                 WHERE date BETWEEN ?1 AND ?2 ORDER BY date ASC",
            )
            .map_err(|error| error.to_string())?;
        let totals = statement
            .query_map(
                params![
                    query.start_date.format("%Y-%m-%d").to_string(),
                    query.end_date.format("%Y-%m-%d").to_string()
                ],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, f64>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .map_err(|error| error.to_string())?
            .filter_map(|row| row.ok())
            .filter_map(|(date, value, rates)| {
                NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                    .ok()
                    .zip(convert_snapshot_value(value, &rates, &query.base_currency))
            })
            .collect::<BTreeMap<_, _>>();
        totals
    } else {
        BTreeMap::new()
    };
    let snapshots = by_date
        .into_iter()
        .map(|(date, stock_values_base)| {
            let stock_total = stock_values_base
                .iter()
                .map(|stock| stock.value_base)
                .sum::<f64>();
            let cash_value_base = portfolio_totals.get(&date).map(|total| total - stock_total);
            RiskSnapshotInput {
                date,
                stock_values_base,
                cash_value_base,
                reliable: conversion_complete.get(&date).copied().unwrap_or(false)
                    && cash_value_base.is_some_and(f64::is_finite),
            }
        })
        .collect();
    let mut total_fees = 0.0;
    let mut changes = Vec::new();
    for action in actions.iter().filter(|action| {
        action_date(action).is_some_and(|date| date >= query.start_date && date <= query.end_date)
            && !action.fact_labels.iter().any(|label| label == "transfer")
    }) {
        let date = action_date(action).unwrap();
        let currency = action
            .currency
            .as_deref()
            .unwrap_or(market_currency(&action.market));
        let fx = resolved_fx_on(currency, &query.base_currency, fx_points, date);
        if let Some(fx) = fx {
            total_fees += action.fees.unwrap_or(0.0) * fx;
            if let Some(notional) = action.gross_amount {
                changes.push(StockChangeBase::trade(notional * fx));
            }
        }
    }
    Ok(RiskStructureInput {
        snapshots,
        stock_changes: changes,
        total_stock_trading_fees_base: Some(total_fees),
        average_portfolio_nav_base: average_nav,
    })
}

fn load_display_context(
    db: &Database,
    query: &StockReviewQuery,
) -> Result<Vec<StockReviewAnnotation>, String> {
    let mut annotations = stock_review_persistence::list_annotations(
        db,
        &StockReviewAnnotationFilter {
            account_id: query.account_id.clone(),
            ..Default::default()
        },
    )?;
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn.prepare(
        "SELECT qh.id, qh.account_id, qh.symbol, qh.notes, qh.decision_quality, qs.snapshot_date
         FROM quarterly_holding_snapshots qh JOIN quarterly_snapshots qs ON qs.id = qh.quarterly_snapshot_id
         WHERE (?1 IS NULL OR qh.account_id = ?1) AND (?2 IS NULL OR qh.market = ?2)
         ORDER BY qs.snapshot_date ASC, qh.id ASC"
    ).map_err(|error| error.to_string())?;
    let historical = statement
        .query_map(params![query.account_id, query.market], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for (id, account_id, symbol, notes, decision_quality, snapshot_date) in historical {
        if notes.is_none() && decision_quality.is_none() {
            continue;
        }
        annotations.push(StockReviewAnnotation {
            id: format!("quarterly:{id}"), scope_type: "stock".to_string(), scope_key: symbol.clone(),
            account_id: Some(account_id), symbol: Some(symbol),
            annotation_type: "historical_manual_assessment".to_string(),
            value_json: serde_json::json!({"snapshot_date": snapshot_date, "notes": notes, "decision_quality": decision_quality, "display_only": true, "label": "历史手工评价"}).to_string(),
            source: "user".to_string(), created_at: snapshot_date.clone(), updated_at: snapshot_date,
        });
    }
    Ok(annotations)
}

fn aggregate_market_coverage(
    prices: &BTreeMap<(String, String), Vec<DailyMarketPoint>>,
    dates: &[NaiveDate],
) -> Option<f64> {
    if prices.is_empty() || dates.is_empty() {
        return None;
    }
    let required = prices.len() * dates.len();
    let present = prices
        .values()
        .map(|points| {
            dates
                .iter()
                .filter(|date| points.iter().any(|point| point.date == **date))
                .count()
        })
        .sum::<usize>();
    Some(present as f64 / required as f64)
}

fn fx_coverage_for_openings(
    positions: &[OpeningPosition],
    cash: &[OpeningCashBalance],
    fx_points: &[crate::services::shadow_portfolio_engine::ShadowFxPoint],
    base_currency: &str,
    dates: &[NaiveDate],
) -> Option<f64> {
    let currencies = positions
        .iter()
        .map(|position| position.currency.as_str())
        .chain(cash.iter().map(|balance| balance.currency.as_str()))
        .collect::<BTreeSet<_>>();
    if currencies.is_empty() || dates.is_empty() {
        return Some(1.0);
    }
    let required = currencies.len() * dates.len();
    let present = currencies
        .iter()
        .map(|currency| {
            dates
                .iter()
                .filter(|date| resolved_fx_on(currency, base_currency, fx_points, **date).is_some())
                .count()
        })
        .sum::<usize>();
    Some(present as f64 / required as f64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::rebalance_attribution::{
        AttributionBatch, AttributionCashBalance, AttributionCashReturn,
        AttributionPositionBalance, AttributionPricePoint, AttributionValuationPoint,
    };

    fn day(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn available() -> MetricAvailability {
        MetricAvailability {
            status: MetricStatus::Available,
            note: None,
        }
    }

    fn live_query(start: &str, end: &str, market: Option<&str>) -> StockReviewQuery {
        StockReviewQuery {
            start_date: day(start),
            end_date: day(end),
            account_id: None,
            market: market.map(str::to_string),
            benchmark_symbol: None,
            base_currency: "USD".to_string(),
        }
    }

    fn insert_account(db: &Database, id: &str, market: &str) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at) VALUES (?1, ?1, ?2, '2024-01-01', '2024-01-01')",
                params![id, market],
            )
            .unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_live_transaction(
        db: &Database,
        id: &str,
        account_id: &str,
        symbol: &str,
        market: &str,
        transaction_type: &str,
        shares: f64,
        price: f64,
        total_amount: f64,
        commission: f64,
        currency: &str,
        traded_at: &str,
    ) {
        db.conn.lock().unwrap().execute(
            "INSERT INTO transactions (id, account_id, symbol, name, market, transaction_type, shares, price, total_amount, commission, currency, traded_at, created_at)
             VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?11)",
            params![id, account_id, symbol, market, transaction_type, shares, price, total_amount, commission, currency, traded_at],
        ).unwrap();
    }

    fn insert_holding(
        db: &Database,
        id: &str,
        account_id: &str,
        symbol: &str,
        market: &str,
        currency: &str,
        shares: f64,
    ) {
        db.conn.lock().unwrap().execute(
            "INSERT INTO holdings (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3, ?4, ?5, 1, ?6, '2024-01-01', '2024-01-01')",
            params![id, account_id, symbol, market, shares, currency],
        ).unwrap();
    }

    fn exchange_rates(date: &str, usd_cny: f64) -> crate::models::ExchangeRates {
        crate::models::ExchangeRates {
            usd_cny,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / usd_cny,
            updated_at: format!("{date}T00:00:00Z"),
        }
    }

    fn insert_portfolio_value(db: &Database, date: &str, value_usd: f64, usd_cny: f64) {
        let rates = serde_json::to_string(&exchange_rates(date, usd_cny)).unwrap();
        db.conn.lock().unwrap().execute(
            "INSERT OR REPLACE INTO daily_portfolio_values (date, total_value, exchange_rates) VALUES (?1, ?2, ?3)",
            params![date, value_usd, rates],
        ).unwrap();
    }

    #[allow(clippy::too_many_arguments)]
    fn insert_holding_snapshot(
        db: &Database,
        date: &str,
        account_id: &str,
        symbol: &str,
        market: &str,
        shares: f64,
        close: f64,
        market_value_local: f64,
    ) {
        db.conn.lock().unwrap().execute(
            "INSERT INTO daily_holding_snapshots (date, account_id, symbol, market, shares, avg_cost, close_price, market_value)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6, ?7)",
            params![date, account_id, symbol, market, shares, close, market_value_local],
        ).unwrap();
    }

    fn insert_stock_price(db: &Database, symbol: &str, market: &str, date: NaiveDate, close: f64) {
        db.conn.lock().unwrap().execute(
            "INSERT OR REPLACE INTO stock_daily_prices
                (symbol, market, date, open, high, low, close, volume, adjusted_close, dividend, source, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4, ?4, ?4, 1, NULL, NULL, 'fixture', '2024-01-01')",
            params![symbol, market, date.format("%Y-%m-%d").to_string(), close],
        ).unwrap();
    }

    fn seed_stock_cache_bounds(
        db: &Database,
        symbol: &str,
        market: &str,
        start: NaiveDate,
        points: &[(NaiveDate, f64)],
    ) {
        insert_stock_price(
            db,
            symbol,
            market,
            start,
            points.first().map_or(100.0, |p| p.1),
        );
        for (date, close) in points {
            insert_stock_price(db, symbol, market, *date, *close);
        }
        insert_stock_price(
            db,
            symbol,
            market,
            Utc::now().date_naive(),
            points.last().map_or(100.0, |p| p.1 + 900.0),
        );
    }

    fn seed_benchmark_cache(
        db: &Database,
        symbol: &str,
        start: NaiveDate,
        omitted: Option<NaiveDate>,
        close: f64,
    ) {
        let mut date = start;
        let end = Utc::now().date_naive();
        let conn = db.conn.lock().unwrap();
        while date <= end {
            if Some(date) != omitted {
                conn.execute(
                    "INSERT OR REPLACE INTO benchmark_daily_prices (symbol, date, close_price, change_percent) VALUES (?1, ?2, ?3, 0)",
                    params![symbol, date.format("%Y-%m-%d").to_string(), close],
                ).unwrap();
            }
            date += Duration::days(1);
        }
    }

    fn seed_default_benchmarks(db: &Database, start: NaiveDate) {
        for symbol in ["^GSPC", "000300.SS", "^HSI"] {
            seed_benchmark_cache(db, symbol, start, None, 100.0);
        }
    }

    fn complete_cached_fixture(with_annotation: bool) -> CachedStockReviewInput {
        let action_id = "action:acct:AAPL:2024-01-01:buy:buy-1";
        let mut sessions = Vec::new();
        let mut cursor = day("2024-01-01");
        for _ in 0..120 {
            cursor += Duration::days(1);
            sessions.push(cursor);
        }
        let day_60 = sessions[59];
        let day_120 = sessions[119];
        CachedStockReviewInput {
            query: StockReviewQuery {
                start_date: day("2024-01-01"),
                end_date: day("2024-01-02"),
                account_id: None,
                market: Some("US".to_string()),
                benchmark_symbol: None,
                base_currency: "USD".to_string(),
            },
            transactions: vec![Transaction {
                id: "buy-1".to_string(),
                holding_id: None,
                account_id: "acct".to_string(),
                symbol: "AAPL".to_string(),
                name: "Apple".to_string(),
                market: "US".to_string(),
                transaction_type: "BUY".to_string(),
                shares: 1.0,
                price: 100.0,
                total_amount: 100.0,
                commission: 0.0,
                currency: "USD".to_string(),
                traded_at: "2024-01-01T09:30:00Z".to_string(),
                notes: None,
                created_at: "2024-01-01T09:30:00Z".to_string(),
            }],
            overrides: vec![],
            persisted_override_issues: vec![],
            preparation_issues: vec![],
            result_quality_input: ResultQualityInput {
                actual_origin_date: day("2023-12-31"),
                actual_values: vec![
                    PortfolioValuePoint {
                        date: day("2024-01-01"),
                        value_base: 1_000.0,
                    },
                    PortfolioValuePoint {
                        date: day("2024-01-02"),
                        value_base: 1_010.0,
                    },
                ],
                baseline: Some(PortfolioValuePoint {
                    date: day("2023-12-31"),
                    value_base: 1_000.0,
                }),
                external_flows_base: vec![],
                actual_availability: available(),
                opening_market_values_base: vec![],
                opening_cash_value_base: 1_000.0,
                benchmark_series: vec![BenchmarkSeriesInput {
                    market: "US".to_string(),
                    availability: available(),
                    points: vec![
                        BenchmarkPoint {
                            date: day("2023-12-31"),
                            value: 100.0,
                        },
                        BenchmarkPoint {
                            date: day("2024-01-01"),
                            value: 100.0,
                        },
                        BenchmarkPoint {
                            date: day("2024-01-02"),
                            value: 100.0,
                        },
                    ],
                }],
                benchmark_selection: BenchmarkSelection::SingleMarket("US".to_string()),
                shadow_curve: vec![],
            },
            shadow_input: ShadowPortfolioInput {
                base_currency: "USD".to_string(),
                return_method: ShadowReturnMethod::ExplicitDividends,
                opening_positions: vec![],
                opening_cash: vec![OpeningCashBalance {
                    account_id: "acct".to_string(),
                    currency: "USD".to_string(),
                    amount: 1_000.0,
                }],
                valuation_dates: vec![day("2023-12-31"), day("2024-01-01"), day("2024-01-02")],
                price_points: vec![],
                fx_points: vec![],
                external_flows: vec![],
                cash_income_events: vec![],
                dividend_events: vec![],
                split_events: vec![],
            },
            actual_comparable: ComparableCurveInput {
                mode: MarketReturnMode::TotalReturn,
                return_value: Some(0.01),
                ending_value_base: Some(1_010.0),
            },
            comparison_mode: MarketReturnMode::TotalReturn,
            forward_actions: vec![ForwardActionInput {
                action_id: action_id.to_string(),
                action_type: "open".to_string(),
                market: "US".to_string(),
                action_date: day("2024-01-01"),
                action_notional_local: 100.0,
                action_day_fx_to_base: Some(1.0),
                market_session_dates: sessions,
                stock_prices_local: vec![
                    LocalPricePoint {
                        date: day("2024-01-01"),
                        close: 100.0,
                    },
                    LocalPricePoint {
                        date: day_60,
                        close: 110.0,
                    },
                    LocalPricePoint {
                        date: day_120,
                        close: 120.0,
                    },
                ],
                benchmark_prices_local: vec![
                    LocalPricePoint {
                        date: day("2024-01-01"),
                        close: 100.0,
                    },
                    LocalPricePoint {
                        date: day_60,
                        close: 100.0,
                    },
                    LocalPricePoint {
                        date: day_120,
                        close: 100.0,
                    },
                ],
                availability: available(),
            }],
            risk_input: RiskStructureInput {
                snapshots: vec![
                    RiskSnapshotInput {
                        date: day("2024-01-01"),
                        stock_values_base: vec![StockValueBase {
                            symbol: "AAPL".to_string(),
                            value_base: 100.0,
                        }],
                        cash_value_base: Some(900.0),
                        reliable: true,
                    },
                    RiskSnapshotInput {
                        date: day("2024-01-02"),
                        stock_values_base: vec![StockValueBase {
                            symbol: "AAPL".to_string(),
                            value_base: 110.0,
                        }],
                        cash_value_base: Some(900.0),
                        reliable: true,
                    },
                ],
                stock_changes: vec![StockChangeBase::trade(100.0)],
                total_stock_trading_fees_base: Some(0.0),
                average_portfolio_nav_base: Some(1_005.0),
            },
            attribution_input: AttributionInput {
                base_currency: "USD".to_string(),
                average_portfolio_nav: Some(1_005.0),
                valuations: vec![
                    AttributionValuationPoint {
                        date: day("2024-01-01"),
                        positions: vec![AttributionPositionBalance {
                            account_id: "acct".to_string(),
                            symbol: "AAPL".to_string(),
                            market: "US".to_string(),
                            currency: "USD".to_string(),
                            actual_quantity: 1.0,
                            shadow_quantity: 0.0,
                        }],
                        cash_balances: vec![AttributionCashBalance {
                            account_id: "acct".to_string(),
                            currency: "USD".to_string(),
                            actual_amount: 900.0,
                            shadow_amount: 1_000.0,
                        }],
                    },
                    AttributionValuationPoint {
                        date: day("2024-01-02"),
                        positions: vec![AttributionPositionBalance {
                            account_id: "acct".to_string(),
                            symbol: "AAPL".to_string(),
                            market: "US".to_string(),
                            currency: "USD".to_string(),
                            actual_quantity: 1.0,
                            shadow_quantity: 0.0,
                        }],
                        cash_balances: vec![AttributionCashBalance {
                            account_id: "acct".to_string(),
                            currency: "USD".to_string(),
                            actual_amount: 900.0,
                            shadow_amount: 1_000.0,
                        }],
                    },
                ],
                prices: vec![
                    AttributionPricePoint::new(day("2024-01-01"), "AAPL", "US", "USD", 100.0),
                    AttributionPricePoint::new(day("2024-01-02"), "AAPL", "US", "USD", 110.0),
                ],
                fx_rates: vec![],
                batches: vec![AttributionBatch::new(
                    action_id,
                    "acct",
                    "AAPL",
                    "US",
                    "USD",
                    "open",
                    day("2024-01-01"),
                    1.0,
                )],
                splits: vec![],
                dividends: vec![],
                fees: vec![],
                cash_returns: vec![AttributionCashReturn::new(day("2024-01-02"), "USD", 0.0)],
            },
            campaign_data: vec![CachedCampaignData {
                campaign_id: "campaign:acct:AAPL:buy-1".to_string(),
                account_id: Some("acct".to_string()),
                symbol: "AAPL".to_string(),
                cash_flows: vec![CampaignTimelineItem {
                    date: day("2024-01-01"),
                    kind: CampaignCashFlowKind::Buy,
                    amount_base: 100.0,
                    shares: 1.0,
                    account_id: "acct".to_string(),
                    action_id: Some(action_id.to_string()),
                }],
                daily_prices: vec![
                    CampaignPricePoint::complete(day("2024-01-01"), 99.0, 101.0, 100.0),
                    CampaignPricePoint::complete(day("2024-01-02"), 109.0, 111.0, 110.0),
                ],
                expected_session_dates: vec![day("2024-01-01"), day("2024-01-02")],
                benchmark_prices: vec![
                    LocalPricePoint {
                        date: day("2024-01-01"),
                        close: 100.0,
                    },
                    LocalPricePoint {
                        date: day("2024-01-02"),
                        close: 100.0,
                    },
                ],
                current_price_local: Some(110.0),
                current_fx_to_base: Some(1.0),
                issues: vec![],
            }],
            annotations: with_annotation
                .then(|| StockReviewAnnotation {
                    id: "note-1".to_string(),
                    scope_type: "period".to_string(),
                    scope_key: "2024-01-01:2024-01-02".to_string(),
                    account_id: None,
                    symbol: None,
                    annotation_type: "historical_manual_assessment".to_string(),
                    value_json: r#"{"decision_quality":"good","display_only":true}"#.to_string(),
                    source: "user".to_string(),
                    created_at: "2024-01-01T00:00:00Z".to_string(),
                    updated_at: "2024-01-01T00:00:00Z".to_string(),
                })
                .into_iter()
                .collect(),
            market_data_coverage: Some(1.0),
            exchange_rate_coverage: Some(1.0),
            benchmark_symbol: Some("^GSPC".to_string()),
            generated_at: "2024-01-03T00:00:00Z".to_string(),
        }
    }

    fn no_trade_fixture() -> CachedStockReviewInput {
        let mut input = complete_cached_fixture(false);
        input.transactions.clear();
        input.forward_actions.clear();
        input.result_quality_input.actual_values[1].value_base = 1_000.0;
        input.actual_comparable.ending_value_base = Some(1_000.0);
        input.risk_input.stock_changes.clear();
        input.attribution_input.average_portfolio_nav = None;
        input.attribution_input.valuations.clear();
        input.attribution_input.prices.clear();
        input.attribution_input.batches.clear();
        input.attribution_input.cash_returns.clear();
        input.campaign_data.clear();
        input
    }

    fn sell_fixture(end_price: f64) -> CachedStockReviewInput {
        let mut input = complete_cached_fixture(false);
        let action_id = "action:acct:AAPL:2024-01-01:sell:sell-1";
        input.transactions = vec![
            Transaction {
                id: "open-0".to_string(),
                holding_id: None,
                account_id: "acct".to_string(),
                symbol: "AAPL".to_string(),
                name: "Apple".to_string(),
                market: "US".to_string(),
                transaction_type: "OPEN".to_string(),
                shares: 1.0,
                price: 100.0,
                total_amount: 100.0,
                commission: 0.0,
                currency: "USD".to_string(),
                traded_at: "2023-12-31T09:30:00Z".to_string(),
                notes: None,
                created_at: "2023-12-31T09:30:00Z".to_string(),
            },
            Transaction {
                id: "sell-1".to_string(),
                holding_id: None,
                account_id: "acct".to_string(),
                symbol: "AAPL".to_string(),
                name: "Apple".to_string(),
                market: "US".to_string(),
                transaction_type: "SELL".to_string(),
                shares: 1.0,
                price: 100.0,
                total_amount: 100.0,
                commission: 0.0,
                currency: "USD".to_string(),
                traded_at: "2024-01-01T09:30:00Z".to_string(),
                notes: None,
                created_at: "2024-01-01T09:30:00Z".to_string(),
            },
        ];
        input.result_quality_input.actual_values[0].value_base = 100.0;
        input.result_quality_input.actual_values[1].value_base = 100.0;
        input
            .result_quality_input
            .baseline
            .as_mut()
            .unwrap()
            .value_base = 100.0;
        input.result_quality_input.opening_cash_value_base = 0.0;
        input.result_quality_input.opening_market_values_base = vec![MarketValue {
            market: "US".to_string(),
            value_base: 100.0,
        }];
        input.actual_comparable.ending_value_base = Some(100.0);
        input.shadow_input.opening_cash.clear();
        input.shadow_input.opening_positions = vec![OpeningPosition {
            account_id: "acct".to_string(),
            symbol: "AAPL".to_string(),
            market: "US".to_string(),
            currency: "USD".to_string(),
            quantity: 1.0,
        }];
        input.shadow_input.price_points = vec![
            ShadowPricePoint {
                date: day("2023-12-31"),
                symbol: "AAPL".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                close: 100.0,
                adjusted_close: None,
            },
            ShadowPricePoint {
                date: day("2024-01-01"),
                symbol: "AAPL".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                close: 100.0,
                adjusted_close: None,
            },
            ShadowPricePoint {
                date: day("2024-01-02"),
                symbol: "AAPL".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                close: end_price,
                adjusted_close: None,
            },
        ];
        input.shadow_input.return_method = ShadowReturnMethod::PriceOnly;
        input.comparison_mode = MarketReturnMode::PriceOnly;
        input.actual_comparable = ComparableCurveInput {
            mode: MarketReturnMode::PriceOnly,
            return_value: Some(0.0),
            ending_value_base: Some(100.0),
        };
        input.forward_actions[0].action_id = action_id.to_string();
        input.forward_actions[0].action_type = "close".to_string();
        input.forward_actions[0].stock_prices_local[1].close = end_price;
        input.forward_actions[0].stock_prices_local[2].close = end_price;
        for valuation in &mut input.attribution_input.valuations {
            valuation.positions[0].actual_quantity = 0.0;
            valuation.positions[0].shadow_quantity = 1.0;
            valuation.cash_balances[0].actual_amount = 100.0;
            valuation.cash_balances[0].shadow_amount = 0.0;
        }
        input.attribution_input.prices[1].close = end_price;
        input.attribution_input.batches[0] = AttributionBatch::new(
            action_id,
            "acct",
            "AAPL",
            "US",
            "USD",
            "close",
            day("2024-01-01"),
            -1.0,
        );
        input.campaign_data[0]
            .cash_flows
            .push(CampaignTimelineItem {
                date: day("2024-01-01"),
                kind: CampaignCashFlowKind::Sell,
                amount_base: 100.0,
                shares: 1.0,
                account_id: "acct".to_string(),
                action_id: Some(action_id.to_string()),
            });
        input.campaign_data[0].current_price_local = None;
        input.campaign_data[0].current_fx_to_base = None;
        input
    }

    fn transfer_fixture() -> CachedStockReviewInput {
        let mut input = no_trade_fixture();
        let record = |id: &str, account: &str, kind: &str, traded_at: &str| Transaction {
            id: id.to_string(),
            holding_id: None,
            account_id: account.to_string(),
            symbol: "AAPL".to_string(),
            name: "Apple".to_string(),
            market: "US".to_string(),
            transaction_type: kind.to_string(),
            shares: 1.0,
            price: 100.0,
            total_amount: 100.0,
            commission: 0.0,
            currency: "USD".to_string(),
            traded_at: traded_at.to_string(),
            notes: None,
            created_at: traded_at.to_string(),
        };
        input.transactions = vec![
            record("open-source", "source", "OPEN", "2023-12-31T09:30:00Z"),
            record("transfer-out", "source", "SELL", "2024-01-01T09:30:00Z"),
            record("transfer-in", "destination", "BUY", "2024-01-01T09:31:00Z"),
        ];
        input.overrides = vec![StockReviewOverride {
            id: "transfer-1".to_string(),
            override_type: "transfer".to_string(),
            transaction_ids_json: r#"["transfer-out","transfer-in"]"#.to_string(),
            value_json: "{}".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }];
        input.risk_input.stock_changes = vec![StockChangeBase::transfer(100.0)];
        input
    }

    fn split_fixture() -> CachedStockReviewInput {
        let mut input = no_trade_fixture();
        input.transactions = vec![Transaction {
            id: "open-0".to_string(),
            holding_id: None,
            account_id: "acct".to_string(),
            symbol: "AAPL".to_string(),
            name: "Apple".to_string(),
            market: "US".to_string(),
            transaction_type: "OPEN".to_string(),
            shares: 1.0,
            price: 100.0,
            total_amount: 100.0,
            commission: 0.0,
            currency: "USD".to_string(),
            traded_at: "2023-12-31T09:30:00Z".to_string(),
            notes: None,
            created_at: "2023-12-31T09:30:00Z".to_string(),
        }];
        input.result_quality_input.actual_values[0].value_base = 100.0;
        input.result_quality_input.actual_values[1].value_base = 100.0;
        input
            .result_quality_input
            .baseline
            .as_mut()
            .unwrap()
            .value_base = 100.0;
        input.actual_comparable.ending_value_base = Some(100.0);
        input.shadow_input.opening_cash.clear();
        input.shadow_input.opening_positions = vec![OpeningPosition {
            account_id: "acct".to_string(),
            symbol: "AAPL".to_string(),
            market: "US".to_string(),
            currency: "USD".to_string(),
            quantity: 1.0,
        }];
        input.shadow_input.price_points = vec![
            ShadowPricePoint {
                date: day("2023-12-31"),
                symbol: "AAPL".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                close: 100.0,
                adjusted_close: None,
            },
            ShadowPricePoint {
                date: day("2024-01-01"),
                symbol: "AAPL".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                close: 50.0,
                adjusted_close: None,
            },
            ShadowPricePoint {
                date: day("2024-01-02"),
                symbol: "AAPL".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                close: 50.0,
                adjusted_close: None,
            },
        ];
        input.shadow_input.split_events =
            vec![crate::services::shadow_portfolio_engine::SplitEvent {
                date: day("2024-01-01"),
                account_id: "acct".to_string(),
                symbol: "AAPL".to_string(),
                market: "US".to_string(),
                ratio: 2.0,
            }];
        input.risk_input.stock_changes = vec![StockChangeBase::split(0.0)];
        input
    }

    #[test]
    fn covers_twelve_deterministic_acceptance_scenarios() {
        // Each literal expectation catches a distinct orchestration regression.
        let no_trade = build_stock_review_report_from_cached_data(&no_trade_fixture()).unwrap();
        assert_eq!(no_trade.actions.len(), 0);
        assert_eq!(no_trade.campaigns.len(), 0);
        assert_eq!(no_trade.summary.rebalance_value_add.value_add, Some(0.0));
        assert_eq!(no_trade.summary.risk_structure.one_way_turnover, Some(0.0));
        assert!(no_trade
            .data_quality
            .issues
            .iter()
            .any(|issue| issue.code == "no_evaluable_actions"));

        let buy =
            build_stock_review_report_from_cached_data(&complete_cached_fixture(false)).unwrap();
        assert!(
            buy.summary
                .forward_effect
                .day_60
                .amount_weighted_excess_return
                .unwrap()
                > 0.0
        );
        assert!(buy.actions[0].contribution.unwrap() > 0.0);
        assert_eq!(buy.actions.len(), 1);
        assert_eq!(buy.campaigns.len(), 1);

        let sell_down = build_stock_review_report_from_cached_data(&sell_fixture(80.0)).unwrap();
        assert!(sell_down.actions[0].contribution.unwrap() > 0.0);
        assert!(sell_down.actions[0]
            .fact_labels
            .iter()
            .any(|label| label == "effective_avoidance"));
        assert_eq!(sell_down.actions.len(), 1);

        let sell_up = build_stock_review_report_from_cached_data(&sell_fixture(120.0)).unwrap();
        assert!(sell_up.actions[0].contribution.unwrap() < 0.0);
        assert!(sell_up.actions[0]
            .fact_labels
            .iter()
            .any(|label| label == "ex_post_opportunity_loss"));
        assert!(!sell_up.actions[0]
            .fact_labels
            .iter()
            .any(|label| label == "wrong"));

        let mut deposit_input = no_trade_fixture();
        deposit_input.result_quality_input.actual_values[1].value_base = 1_500.0;
        deposit_input.result_quality_input.external_flows_base = vec![ExternalFlowBase {
            date: day("2024-01-02"),
            amount_base: 500.0,
        }];
        deposit_input.shadow_input.external_flows = vec![
            crate::services::shadow_portfolio_engine::ExternalFlowEvent {
                date: day("2024-01-02"),
                account_id: "acct".to_string(),
                currency: "USD".to_string(),
                amount: 500.0,
            },
        ];
        let deposit = build_stock_review_report_from_cached_data(&deposit_input).unwrap();
        assert_eq!(deposit.summary.result_quality.portfolio_return, Some(0.0));
        assert_eq!(deposit.summary.result_quality.shadow_return, Some(0.0));

        let transfer = build_stock_review_report_from_cached_data(&transfer_fixture()).unwrap();
        assert_eq!(transfer.actions.len(), 0);
        assert_eq!(transfer.campaigns.len(), 1);
        assert_eq!(transfer.summary.risk_structure.one_way_turnover, Some(0.0));

        let split = build_stock_review_report_from_cached_data(&split_fixture()).unwrap();
        assert_eq!(split.actions.len(), 0);
        assert_eq!(split.summary.result_quality.shadow_return, Some(0.0));
        assert_eq!(split.summary.risk_structure.one_way_turnover, Some(0.0));

        let mut dividend_input = split_fixture();
        dividend_input.result_quality_input.actual_values[1].value_base = 110.0;
        dividend_input
            .shadow_input
            .price_points
            .iter_mut()
            .for_each(|point| point.close = 100.0);
        dividend_input.shadow_input.split_events.clear();
        dividend_input.shadow_input.dividend_events =
            vec![crate::services::shadow_portfolio_engine::DividendEvent {
                date: day("2024-01-02"),
                account_id: "acct".to_string(),
                symbol: "AAPL".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                amount_per_share: 10.0,
            }];
        let dividend = build_stock_review_report_from_cached_data(&dividend_input).unwrap();
        assert!((dividend.summary.result_quality.portfolio_return.unwrap() - 0.10).abs() < 1e-12);
        assert!((dividend.summary.result_quality.shadow_return.unwrap() - 0.10).abs() < 1e-12);

        let mut recent_input = complete_cached_fixture(false);
        recent_input.forward_actions[0]
            .market_session_dates
            .truncate(10);
        let recent = build_stock_review_report_from_cached_data(&recent_input).unwrap();
        assert_eq!(recent.actions[0].status, MetricStatus::Pending);
        assert_eq!(
            recent.data_quality.forward_effect_availability.status,
            MetricStatus::Pending
        );
        assert_eq!(recent.summary.forward_effect.day_60.pending_actions, 1);

        let mut fx_input = complete_cached_fixture(false);
        fx_input.attribution_input.base_currency = "USD".to_string();
        fx_input.attribution_input.average_portfolio_nav = Some(100_000.0);
        fx_input
            .attribution_input
            .valuations
            .iter_mut()
            .for_each(|valuation| {
                valuation.positions[0].currency = "CNY".to_string();
                valuation.cash_balances[0].actual_amount = 0.0;
                valuation.cash_balances[0].shadow_amount = 0.0;
                valuation.cash_balances[0].currency = "CNY".to_string();
            });
        fx_input
            .attribution_input
            .prices
            .iter_mut()
            .for_each(|price| price.currency = "CNY".to_string());
        fx_input.attribution_input.batches[0].currency = "CNY".to_string();
        fx_input.attribution_input.cash_returns[0].currency = "CNY".to_string();
        fx_input.attribution_input.fx_rates = vec![
            crate::services::rebalance_attribution::AttributionFxPoint::new(
                day("2024-01-01"),
                "CNY",
                "USD",
                0.14,
            ),
            crate::services::rebalance_attribution::AttributionFxPoint::new(
                day("2024-01-02"),
                "CNY",
                "USD",
                0.15,
            ),
        ];
        let fx = build_stock_review_report_from_cached_data(&fx_input).unwrap();
        assert!((fx.attribution.action_contributions[0].amount - 1.5).abs() < 1e-12);
        assert!((fx.attribution.currency_contribution.unwrap() - 1.0).abs() < 1e-12);

        let mut missing_input = complete_cached_fixture(false);
        missing_input.forward_actions[0]
            .stock_prices_local
            .truncate(1);
        let missing = build_stock_review_report_from_cached_data(&missing_input).unwrap();
        assert_eq!(
            missing.summary.result_quality.availability.status,
            MetricStatus::Available
        );
        assert_eq!(
            missing.summary.forward_effect.availability.status,
            MetricStatus::Unavailable
        );
        assert_eq!(
            missing.summary.risk_structure.availability.status,
            MetricStatus::Available
        );

        assert_eq!(buy.attribution.residual, Some(0.0));
        assert_eq!(buy.attribution.ending_value_difference, Some(10.0));
        assert_eq!(buy.attribution.explained_value_difference, Some(10.0));
    }

    #[test]
    fn market_filter_excludes_other_market_external_cash_flow() {
        // Removing the market predicate would make a CN deposit look like US investment return.
        let cash = |id: &str, market: &str, amount: f64| Transaction {
            id: id.to_string(),
            holding_id: None,
            account_id: "acct".to_string(),
            symbol: "$CASH-USD".to_string(),
            name: "Cash".to_string(),
            market: market.to_string(),
            transaction_type: "BUY".to_string(),
            shares: amount,
            price: 1.0,
            total_amount: amount,
            commission: 0.0,
            currency: "USD".to_string(),
            traded_at: "2024-01-02T09:30:00Z".to_string(),
            notes: None,
            created_at: "2024-01-02T09:30:00Z".to_string(),
        };
        let mut query = complete_cached_fixture(false).query;
        query.market = Some("US".to_string());
        let flows = external_flows_base(
            &[cash("us", "US", 100.0), cash("cn", "CN", 900.0)],
            &query,
            day("2024-01-01"),
        );
        assert_eq!(
            flows,
            vec![ExternalFlowBase {
                date: day("2024-01-02"),
                amount_base: 100.0
            }]
        );
    }

    #[test]
    fn covers_same_day_duplicate_fixed_benchmark_halt_and_zero_fee_boundaries() {
        let mut same_day = no_trade_fixture();
        let trade = |id: &str, kind: &str| Transaction {
            id: id.to_string(),
            holding_id: None,
            account_id: "acct".to_string(),
            symbol: "AAPL".to_string(),
            name: "Apple".to_string(),
            market: "US".to_string(),
            transaction_type: kind.to_string(),
            shares: 1.0,
            price: 100.0,
            total_amount: 100.0,
            commission: 0.0,
            currency: "USD".to_string(),
            traded_at: "2024-01-01".to_string(),
            notes: None,
            created_at: id.to_string(),
        };
        same_day.transactions = vec![trade("a-buy", "BUY"), trade("b-sell", "SELL")];
        let same_day_report = build_stock_review_report_from_cached_data(&same_day).unwrap();
        assert!(same_day_report
            .data_quality
            .issues
            .iter()
            .any(|issue| issue.code == "same_day_order_uncertain"));
        assert_eq!(same_day_report.actions.len(), 2);

        let mut duplicate = no_trade_fixture();
        duplicate.transactions = vec![trade("dup-a", "BUY"), trade("dup-b", "BUY")];
        duplicate.overrides = vec![StockReviewOverride {
            id: "dup-override".to_string(),
            override_type: "duplicate".to_string(),
            transaction_ids_json: r#"["dup-a","dup-b"]"#.to_string(),
            value_json: "{}".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }];
        let duplicate_report = build_stock_review_report_from_cached_data(&duplicate).unwrap();
        assert_eq!(
            duplicate_report
                .data_quality
                .actual_result_availability
                .status,
            MetricStatus::Available
        );
        assert_eq!(
            duplicate_report
                .data_quality
                .shadow_value_add_availability
                .status,
            MetricStatus::Unavailable
        );
        assert_eq!(
            duplicate_report
                .data_quality
                .attribution_availability
                .status,
            MetricStatus::Unavailable
        );
        assert!(duplicate_report
            .data_quality
            .issues
            .iter()
            .any(|issue| issue.code == "source_ledger_conflict"));

        let mut mixed = complete_cached_fixture(false);
        mixed.query.market = None;
        mixed.result_quality_input.benchmark_selection = BenchmarkSelection::AutomaticMixed;
        mixed.result_quality_input.opening_market_values_base = vec![
            MarketValue {
                market: "US".to_string(),
                value_base: 500.0,
            },
            MarketValue {
                market: "CN".to_string(),
                value_base: 500.0,
            },
        ];
        mixed.result_quality_input.opening_cash_value_base = 0.0;
        mixed.result_quality_input.benchmark_series = vec![
            BenchmarkSeriesInput {
                market: "US".to_string(),
                availability: available(),
                points: vec![
                    BenchmarkPoint {
                        date: day("2023-12-31"),
                        value: 100.0,
                    },
                    BenchmarkPoint {
                        date: day("2024-01-01"),
                        value: 100.0,
                    },
                    BenchmarkPoint {
                        date: day("2024-01-02"),
                        value: 110.0,
                    },
                ],
            },
            BenchmarkSeriesInput {
                market: "CN".to_string(),
                availability: available(),
                points: vec![
                    BenchmarkPoint {
                        date: day("2023-12-31"),
                        value: 100.0,
                    },
                    BenchmarkPoint {
                        date: day("2024-01-01"),
                        value: 100.0,
                    },
                    BenchmarkPoint {
                        date: day("2024-01-02"),
                        value: 100.0,
                    },
                ],
            },
        ];
        let mixed_report = build_stock_review_report_from_cached_data(&mixed).unwrap();
        assert!(
            (mixed_report
                .summary
                .result_quality
                .benchmark_return
                .unwrap()
                - 0.05)
                .abs()
                < 1e-12
        );
        assert_eq!(mixed_report.methodology.fixed_weights.len(), 2);

        let mut halted = complete_cached_fixture(false);
        halted.forward_actions[0].stock_prices_local.truncate(1);
        let halted_report = build_stock_review_report_from_cached_data(&halted).unwrap();
        assert_eq!(halted_report.actions[0].status, MetricStatus::Unavailable);
        assert!(halted_report.actions[0]
            .observation_windows
            .iter()
            .all(|window| window.amount_weighted_excess_return.is_none()));

        let fee_zero =
            build_stock_review_report_from_cached_data(&complete_cached_fixture(false)).unwrap();
        assert_eq!(fee_zero.summary.risk_structure.fee_drag, Some(0.0));
        assert!(fee_zero
            .risk_structure
            .data_hints
            .iter()
            .any(|hint| hint == "fees_may_be_incompletely_imported"));
    }

    #[test]
    fn campaign_detail_is_derived_from_the_same_cached_core() {
        let input = complete_cached_fixture(false);
        let report = build_stock_review_report_from_cached_data(&input).unwrap();
        let detail =
            build_stock_campaign_detail_from_cached_data(&input, &report.campaigns[0].campaign_id)
                .unwrap();
        assert_eq!(detail.summary, report.campaigns[0]);
        assert_eq!(detail.actions[0].action_id, report.actions[0].action_id);
        assert_eq!(
            detail.actions[0].contribution,
            report.actions[0].contribution
        );
        assert_eq!(
            detail.actions[0].observation_windows[1],
            report.actions[0].observation_windows[0]
        );
        assert_eq!(
            detail.forward_effect_60d,
            report.summary.forward_effect.day_60
        );
    }

    #[tokio::test]
    async fn live_opening_cash_replays_stock_cash_effects_and_fees() {
        // Omitting a stock BUY from opening-cash reconstruction changes the
        // literal expected balance from 399 to 1,000.
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_live_transaction(
            &db,
            "deposit",
            "acct",
            "$CASH-USD",
            "US",
            "BUY",
            1_000.0,
            1.0,
            1_000.0,
            0.0,
            "USD",
            "2024-01-01T09:00:00Z",
        );
        insert_live_transaction(
            &db,
            "buy",
            "acct",
            "AAPL",
            "US",
            "BUY",
            6.0,
            100.0,
            600.0,
            1.0,
            "USD",
            "2024-01-02T09:30:00Z",
        );
        for (date, value) in [("2024-01-09", 999.0), ("2024-01-10", 999.0)] {
            insert_portfolio_value(&db, date, value, 7.0);
        }
        let cache_start = day("2023-12-31");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[(day("2024-01-09"), 100.0), (day("2024-01-10"), 100.0)],
        );
        seed_benchmark_cache(&db, "^GSPC", cache_start, None, 100.0);

        let prepared = prepare_cached_stock_review_input(
            &db,
            live_query("2024-01-10", "2024-01-10", Some("US")),
        )
        .await
        .unwrap();
        assert_eq!(prepared.shadow_input.opening_cash.len(), 1);
        assert_eq!(prepared.shadow_input.opening_cash[0].amount, 399.0);
        assert_eq!(prepared.result_quality_input.opening_cash_value_base, 399.0);
    }

    #[test]
    fn authoritative_current_cash_unwinds_transactions_after_historical_report_cutoff() {
        // Using only transactions loaded through the report end would leave
        // the historical origin at today's 800 instead of the literal 1,000.
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_holding(&db, "cash", "acct", "$CASH-USD", "US", "USD", 800.0);
        insert_live_transaction(
            &db,
            "future-buy",
            "acct",
            "AAPL",
            "US",
            "BUY",
            2.0,
            100.0,
            200.0,
            0.0,
            "USD",
            "2024-01-20T09:30:00Z",
        );
        let query = live_query("2024-01-10", "2024-01-10", Some("US"));
        let loaded = load_transactions_for_review(&db, query.end_date).unwrap();
        let (cash, complete) = opening_cash(&db, &loaded, &query, day("2024-01-09")).unwrap();
        assert!(complete);
        assert_eq!(cash[0].amount, 1_000.0);
    }

    #[tokio::test]
    async fn incomplete_live_opening_cash_disables_shadow_and_fixed_benchmark_without_hiding_actual(
    ) {
        // A stock trade without a cash-ledger anchor must never be treated as
        // an exact negative opening cash balance.
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_live_transaction(
            &db,
            "buy",
            "acct",
            "AAPL",
            "US",
            "BUY",
            1.0,
            100.0,
            100.0,
            0.0,
            "USD",
            "2024-01-01T09:30:00Z",
        );
        for (date, value) in [
            ("2024-01-09", 100.0),
            ("2024-01-10", 100.0),
            ("2024-01-11", 100.0),
        ] {
            insert_portfolio_value(&db, date, value, 7.0);
        }
        let cache_start = day("2023-12-22");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[
                (day("2024-01-09"), 100.0),
                (day("2024-01-10"), 100.0),
                (day("2024-01-11"), 100.0),
            ],
        );
        seed_default_benchmarks(&db, cache_start);

        let report = get_stock_review_report(&db, live_query("2024-01-10", "2024-01-11", None))
            .await
            .unwrap();
        assert_eq!(report.summary.result_quality.portfolio_return, Some(0.0));
        assert_eq!(report.summary.result_quality.shadow_return, None);
        assert_eq!(report.summary.result_quality.benchmark_return, None);
        assert_eq!(report.summary.rebalance_value_add.value_add, None);
        assert!(report
            .data_quality
            .issues
            .iter()
            .any(|issue| issue.code == "opening_cash_incomplete"));
    }

    #[tokio::test]
    async fn live_split_materialization_preserves_shadow_value() {
        // Removing the persisted split event halves the literal ending value.
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_live_transaction(
            &db,
            "cash-zero",
            "acct",
            "$CASH-USD",
            "US",
            "BUY",
            0.0,
            1.0,
            0.0,
            0.0,
            "USD",
            "2024-01-01T08:00:00Z",
        );
        insert_live_transaction(
            &db,
            "opening",
            "acct",
            "AAPL",
            "US",
            "OPEN",
            10.0,
            100.0,
            1_000.0,
            0.0,
            "USD",
            "2024-01-01T09:30:00Z",
        );
        db.conn.lock().unwrap().execute(
            "INSERT INTO stock_splits (stock_code, split_date, ratio_from, ratio_to, created_at) VALUES ('AAPL', '2024-01-10', 1, 2, '2024-01-01')",
            [],
        ).unwrap();
        for (date, value) in [
            ("2024-01-09", 1_000.0),
            ("2024-01-10", 1_000.0),
            ("2024-01-11", 1_000.0),
        ] {
            insert_portfolio_value(&db, date, value, 7.0);
        }
        let cache_start = day("2023-12-31");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[
                (day("2024-01-09"), 100.0),
                (day("2024-01-10"), 50.0),
                (day("2024-01-11"), 50.0),
            ],
        );
        seed_benchmark_cache(&db, "^GSPC", cache_start, None, 100.0);

        let prepared = prepare_cached_stock_review_input(
            &db,
            live_query("2024-01-10", "2024-01-11", Some("US")),
        )
        .await
        .unwrap();
        assert_eq!(prepared.shadow_input.split_events.len(), 1);
        let shadow = build_shadow_series(&prepared.shadow_input);
        assert_eq!(shadow.ending_value, Some(1_000.0));
        assert_eq!(
            shadow.twr_return_series.last().unwrap().cumulative_return,
            0.0
        );
    }

    #[tokio::test]
    async fn live_no_trade_holdings_produce_reliable_risk_and_known_zero_value_add() {
        // Empty transaction history must not erase authoritative holdings/cash.
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_holding(&db, "stock", "acct", "AAPL", "US", "USD", 10.0);
        insert_holding(&db, "cash", "acct", "$CASH-USD", "US", "USD", 500.0);
        for date in ["2024-01-09", "2024-01-10", "2024-01-11"] {
            insert_portfolio_value(&db, date, 1_500.0, 7.0);
            insert_holding_snapshot(&db, date, "acct", "AAPL", "US", 10.0, 100.0, 1_000.0);
        }
        let cache_start = day("2023-12-31");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[
                (day("2024-01-09"), 100.0),
                (day("2024-01-10"), 100.0),
                (day("2024-01-11"), 100.0),
            ],
        );
        seed_default_benchmarks(&db, cache_start);

        let report = get_stock_review_report(&db, live_query("2024-01-10", "2024-01-11", None))
            .await
            .unwrap();
        assert_eq!(report.actions.len(), 0);
        assert_eq!(report.summary.rebalance_value_add.value_add, Some(0.0));
        assert_eq!(report.summary.risk_structure.one_way_turnover, Some(0.0));
        assert_eq!(
            report.summary.risk_structure.availability.status,
            MetricStatus::Available
        );
        assert!(
            (report.summary.risk_structure.opening_cash_ratio.unwrap() - 1.0 / 3.0).abs() < 1e-12
        );
    }

    #[tokio::test]
    async fn live_dividend_and_fx_inputs_are_materialized_for_shadow_and_attribution() {
        // Leaving PAY, daily FX, batches, or zero cash-return keys empty makes
        // the shadow return or attribution unavailable instead of the literal values.
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "CN");
        insert_live_transaction(
            &db,
            "deposit",
            "acct",
            "$CASH-CNY",
            "CN",
            "BUY",
            1_000.0,
            1.0,
            1_000.0,
            0.0,
            "CNY",
            "2024-01-01T08:00:00Z",
        );
        insert_live_transaction(
            &db,
            "opening",
            "acct",
            "600000",
            "CN",
            "OPEN",
            1.0,
            100.0,
            100.0,
            0.0,
            "CNY",
            "2024-01-01T09:00:00Z",
        );
        insert_live_transaction(
            &db,
            "buy",
            "acct",
            "600000",
            "CN",
            "BUY",
            1.0,
            100.0,
            100.0,
            0.0,
            "CNY",
            "2024-01-10T09:30:00Z",
        );
        insert_live_transaction(
            &db,
            "dividend",
            "acct",
            "600000",
            "CN",
            "PAY",
            2.0,
            5.0,
            10.0,
            0.0,
            "CNY",
            "2024-01-11T09:30:00Z",
        );
        insert_portfolio_value(&db, "2024-01-09", 1_100.0 / 7.0, 7.0);
        insert_portfolio_value(&db, "2024-01-10", 1_100.0 / 7.0, 7.0);
        insert_portfolio_value(&db, "2024-01-11", 1_130.0 / (20.0 / 3.0), 20.0 / 3.0);
        insert_holding_snapshot(&db, "2024-01-10", "acct", "600000", "CN", 2.0, 100.0, 200.0);
        insert_holding_snapshot(&db, "2024-01-11", "acct", "600000", "CN", 2.0, 110.0, 220.0);
        let cache_start = day("2023-12-31");
        seed_stock_cache_bounds(
            &db,
            "600000",
            "CN",
            cache_start,
            &[
                (day("2024-01-09"), 100.0),
                (day("2024-01-10"), 100.0),
                (day("2024-01-11"), 110.0),
            ],
        );
        seed_default_benchmarks(&db, cache_start);

        let prepared =
            prepare_cached_stock_review_input(&db, live_query("2024-01-10", "2024-01-11", None))
                .await
                .unwrap();
        assert_eq!(prepared.shadow_input.dividend_events.len(), 1);
        assert!(!prepared.attribution_input.valuations.is_empty());
        assert_eq!(prepared.attribution_input.batches.len(), 1);
        assert_eq!(prepared.attribution_input.cash_returns.len(), 1);
        assert_eq!(prepared.attribution_input.cash_returns[0].return_rate, 0.0);
        assert!(prepared.attribution_input.fx_rates.iter().any(|fx| {
            fx.date == day("2024-01-11") && fx.currency == "CNY" && (fx.rate - 0.15).abs() < 1e-12
        }));
        let report = build_stock_review_report_from_cached_data(&prepared).unwrap();
        assert_eq!(
            report.attribution.availability.status,
            MetricStatus::Available
        );
        // The CNY purchase moves CNY cash into a CNY stock, so the explicit
        // currency component cancels to known zero while price/dividend action
        // contribution remains separate.
        assert!(report.attribution.currency_contribution.unwrap().abs() < 1e-12);
        assert!(report.attribution.action_contributions[0].amount > 0.0);
        assert!(report.summary.result_quality.shadow_return.unwrap() > 0.0);
    }

    #[tokio::test]
    async fn live_campaign_data_is_scoped_by_logical_cycle_account_and_historical_cutoff() {
        // Symbol-only campaign cache lookup leaks cash flows across cycles/accounts
        // and a today-based terminal quote leaks future data into historical reports.
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "a", "US");
        insert_account(&db, "b", "US");
        insert_live_transaction(
            &db,
            "a1",
            "a",
            "AAPL",
            "US",
            "BUY",
            1.0,
            100.0,
            100.0,
            0.0,
            "USD",
            "2024-01-01T09:30:00Z",
        );
        insert_live_transaction(
            &db,
            "a2",
            "a",
            "AAPL",
            "US",
            "SELL",
            1.0,
            110.0,
            110.0,
            0.0,
            "USD",
            "2024-01-03T09:30:00Z",
        );
        insert_live_transaction(
            &db,
            "b1",
            "b",
            "AAPL",
            "US",
            "BUY",
            2.0,
            100.0,
            200.0,
            0.0,
            "USD",
            "2024-01-02T09:30:00Z",
        );
        insert_live_transaction(
            &db,
            "a3",
            "a",
            "AAPL",
            "US",
            "BUY",
            1.0,
            120.0,
            120.0,
            0.0,
            "USD",
            "2024-01-05T09:30:00Z",
        );
        save_user_stock_review_annotation(
            &db,
            StockReviewAnnotationInput {
                id: "campaign-note".to_string(),
                scope_type: "campaign".to_string(),
                scope_key: "campaign:a:AAPL:a1".to_string(),
                account_id: Some("a".to_string()),
                symbol: None,
                annotation_type: "thesis".to_string(),
                value_json: r#"{"note":"first cycle only"}"#.to_string(),
                source: "user".to_string(),
            },
        )
        .unwrap();
        for (date, value) in [
            ("2024-01-03", 310.0),
            ("2024-01-04", 310.0),
            ("2024-01-05", 430.0),
            ("2024-01-06", 440.0),
        ] {
            insert_portfolio_value(&db, date, value, 7.0);
        }
        let cache_start = day("2023-12-22");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[
                (day("2024-01-01"), 100.0),
                (day("2024-01-02"), 100.0),
                (day("2024-01-03"), 110.0),
                (day("2024-01-04"), 110.0),
                (day("2024-01-05"), 120.0),
                (day("2024-01-06"), 125.0),
            ],
        );
        seed_benchmark_cache(&db, "^GSPC", cache_start, None, 100.0);

        let prepared = prepare_cached_stock_review_input(
            &db,
            live_query("2024-01-04", "2024-01-06", Some("US")),
        )
        .await
        .unwrap();
        let artifacts = build_stock_review_artifacts(&prepared).unwrap();
        assert_eq!(artifacts.report.campaigns.len(), 3);
        assert_eq!(artifacts.campaign_details.len(), 3);
        let first = artifacts
            .campaign_details
            .iter()
            .find(|detail| {
                detail
                    .summary
                    .action_ids
                    .contains(&"action:a:AAPL:2024-01-01:buy:a1".to_string())
            })
            .unwrap();
        assert_eq!(first.actions.len(), 2);
        assert_eq!(first.annotations.len(), 1);
        assert!(!first.issues.is_empty());
        assert!(first
            .timeline
            .iter()
            .all(|item| item.account_id == "a" && item.date <= day("2024-01-03")));
        let account_b = artifacts
            .campaign_details
            .iter()
            .find(|detail| detail.summary.account_ids == vec!["b".to_string()])
            .unwrap();
        assert!(account_b.annotations.is_empty());
        assert!(account_b.timeline.iter().all(|item| item.account_id == "b"));
        let active_a = artifacts
            .campaign_details
            .iter()
            .find(|detail| {
                detail
                    .summary
                    .action_ids
                    .contains(&"action:a:AAPL:2024-01-05:buy:a3".to_string())
            })
            .unwrap();
        assert_eq!(active_a.pnl.remaining_market_value_base, Some(125.0));
    }

    #[tokio::test]
    async fn live_forward_uses_local_calendar_and_quality_matches_missing_exact_endpoint() {
        // A selected portfolio benchmark must not replace the stock's local
        // broad-market session calendar/price series.
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_live_transaction(
            &db,
            "cash",
            "acct",
            "$CASH-USD",
            "US",
            "BUY",
            1_000.0,
            1.0,
            1_000.0,
            0.0,
            "USD",
            "2023-12-01T08:00:00Z",
        );
        insert_live_transaction(
            &db,
            "buy",
            "acct",
            "AAPL",
            "US",
            "BUY",
            1.0,
            100.0,
            100.0,
            0.0,
            "USD",
            "2024-01-01T09:30:00Z",
        );
        insert_portfolio_value(&db, "2023-12-31", 1_000.0, 7.0);
        insert_portfolio_value(&db, "2024-01-01", 1_000.0, 7.0);
        insert_portfolio_value(&db, "2024-04-30", 1_000.0, 7.0);
        let cache_start = day("2023-11-21");
        let canonical_target = day("2024-03-01");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[
                (day("2024-01-01"), 100.0),
                (day("2024-03-02"), 120.0),
                (day("2024-04-30"), 120.0),
            ],
        );
        seed_benchmark_cache(&db, "^GSPC", cache_start, None, 100.0);
        seed_benchmark_cache(&db, "QQQ", cache_start, Some(canonical_target), 200.0);
        let mut query = live_query("2024-01-01", "2024-04-30", Some("US"));
        query.benchmark_symbol = Some("QQQ".to_string());

        let prepared = prepare_cached_stock_review_input(&db, query).await.unwrap();
        assert_eq!(
            prepared.forward_actions[0].benchmark_prices_local[0].close,
            100.0
        );
        assert!(prepared.forward_actions[0]
            .market_session_dates
            .contains(&canonical_target));
        let report = build_stock_review_report_from_cached_data(&prepared).unwrap();
        assert_eq!(
            report.summary.forward_effect.day_60.status.status,
            MetricStatus::Unavailable
        );
        assert_eq!(
            report.data_quality.forward_effect_availability.status,
            MetricStatus::Unavailable
        );
        assert_eq!(
            report.data_quality.actual_result_availability.status,
            report.summary.result_quality.availability.status
        );
    }

    #[tokio::test]
    async fn live_confirmed_transfer_is_excluded_from_turnover_and_stays_one_campaign() {
        // Raw BUY/SELL summation would report non-zero turnover for a confirmed transfer.
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "source", "US");
        insert_account(&db, "destination", "US");
        insert_live_transaction(
            &db,
            "cash",
            "source",
            "$CASH-USD",
            "US",
            "BUY",
            1_000.0,
            1.0,
            1_000.0,
            0.0,
            "USD",
            "2023-12-01T08:00:00Z",
        );
        insert_live_transaction(
            &db,
            "open",
            "source",
            "AAPL",
            "US",
            "OPEN",
            1.0,
            100.0,
            100.0,
            0.0,
            "USD",
            "2023-12-15T09:30:00Z",
        );
        insert_live_transaction(
            &db,
            "out",
            "source",
            "AAPL",
            "US",
            "SELL",
            1.0,
            100.0,
            100.0,
            0.0,
            "USD",
            "2024-01-02T09:30:00Z",
        );
        insert_live_transaction(
            &db,
            "in",
            "destination",
            "AAPL",
            "US",
            "BUY",
            1.0,
            100.0,
            100.0,
            0.0,
            "USD",
            "2024-01-02T10:30:00Z",
        );
        stock_review_persistence::save_override(
            &db,
            StockReviewOverrideInput {
                id: "transfer".to_string(),
                override_type: "transfer".to_string(),
                transaction_ids_json: r#"["out","in"]"#.to_string(),
                value_json: "{}".to_string(),
            },
        )
        .unwrap();
        for date in ["2024-01-01", "2024-01-02", "2024-01-03"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
            insert_holding_snapshot(&db, date, "destination", "AAPL", "US", 1.0, 100.0, 100.0);
        }
        let cache_start = day("2023-11-21");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[
                (day("2024-01-01"), 100.0),
                (day("2024-01-02"), 100.0),
                (day("2024-01-03"), 100.0),
            ],
        );
        seed_default_benchmarks(&db, cache_start);

        let prepared =
            prepare_cached_stock_review_input(&db, live_query("2024-01-02", "2024-01-03", None))
                .await
                .unwrap();
        let cash_return_keys = prepared
            .attribution_input
            .cash_returns
            .iter()
            .map(|item| (item.date, item.currency.clone()))
            .collect::<BTreeSet<_>>();
        assert_eq!(
            cash_return_keys.len(),
            prepared.attribution_input.cash_returns.len(),
            "cash-return observations must have explicit unique date/currency keys"
        );
        let report = build_stock_review_report_from_cached_data(&prepared).unwrap();
        assert_eq!(report.actions.len(), 0);
        assert_eq!(report.campaigns.len(), 1);
        assert_eq!(report.summary.risk_structure.one_way_turnover, Some(0.0));
    }

    #[tokio::test]
    async fn successful_override_preview_reflects_canonical_correction_before_persisting() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_live_transaction(
            &db,
            "cash",
            "acct",
            "$CASH-USD",
            "US",
            "BUY",
            1_000.0,
            1.0,
            1_000.0,
            0.0,
            "USD",
            "2023-12-01T08:00:00Z",
        );
        insert_live_transaction(
            &db,
            "buy",
            "acct",
            "AAPL",
            "US",
            "BUY",
            1.0,
            100.0,
            100.0,
            0.0,
            "USD",
            "2024-01-02T09:30:00Z",
        );
        for date in ["2024-01-01", "2024-01-02", "2024-01-03"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
        }
        let cache_start = day("2023-11-21");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[
                (day("2024-01-01"), 100.0),
                (day("2024-01-02"), 100.0),
                (day("2024-01-03"), 100.0),
            ],
        );
        seed_default_benchmarks(&db, cache_start);

        let report = confirm_stock_review_override(
            &db,
            live_query("2024-01-02", "2024-01-03", None),
            StockReviewOverrideInput {
                id: "candidate ".to_string(),
                override_type: " non_trade ".to_string(),
                transaction_ids_json: r#"["buy"]"#.to_string(),
                value_json: "{}".to_string(),
            },
        )
        .await
        .unwrap();
        assert_eq!(report.actions.len(), 0);
        let saved = stock_review_persistence::list_overrides(&db).unwrap();
        assert_eq!(saved.overrides.len(), 1);
        assert_eq!(saved.overrides[0].id, "candidate");
        assert_eq!(saved.overrides[0].override_type, "non_trade");
    }

    #[tokio::test]
    async fn post_insertion_candidate_failure_has_no_database_side_effect() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_live_transaction(
            &db,
            "future",
            "acct",
            "AAPL",
            "US",
            "BUY",
            1.0,
            100.0,
            100.0,
            0.0,
            "USD",
            "2024-02-01T09:30:00Z",
        );
        insert_portfolio_value(&db, "2024-01-01", 1_000.0, 7.0);
        insert_portfolio_value(&db, "2024-01-03", 1_000.0, 7.0);
        seed_default_benchmarks(&db, day("2023-12-20"));
        let result = confirm_stock_review_override(
            &db,
            live_query("2024-01-02", "2024-01-03", None),
            StockReviewOverrideInput {
                id: "candidate".to_string(),
                override_type: "non_trade".to_string(),
                transaction_ids_json: r#"["future"]"#.to_string(),
                value_json: "{}".to_string(),
            },
        )
        .await;
        assert!(result.is_err());
        let count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM stock_review_overrides", [], |row| {
                row.get(0)
            })
            .unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn cached_fx_is_not_backdated_before_its_source_date() {
        // Relabelling a current quote as the report origin would be a hidden backward fill.
        let db = Database::new(":memory:").unwrap();
        db.conn.lock().unwrap().execute(
            "INSERT OR REPLACE INTO cached_exchange_rates (id, usd_cny, usd_hkd, cny_hkd, updated_at) VALUES (1, 7.0, 7.8, 1.1, '2024-02-01T00:00:00Z')",
            [],
        ).unwrap();
        let points = load_static_fx_points(&db, day("2024-01-01"), "USD").unwrap();
        assert_eq!(
            resolved_fx_on("CNY", "USD", &points, day("2024-01-15")),
            None
        );
        assert!(resolved_fx_on("CNY", "USD", &points, day("2024-02-02")).is_some());
    }

    #[test]
    fn forward_filled_fx_is_explicit_and_degrades_only_dependent_quality() {
        let mut input = no_trade_fixture();
        input.shadow_input.opening_cash = vec![OpeningCashBalance {
            account_id: "acct".to_string(),
            currency: "CNY".to_string(),
            amount: 7_000.0,
        }];
        input.shadow_input.fx_points =
            vec![crate::services::shadow_portfolio_engine::ShadowFxPoint {
                date: day("2023-12-31"),
                currency: "CNY".to_string(),
                base_currency: "USD".to_string(),
                rate: 1.0 / 7.0,
            }];
        input.exchange_rate_coverage = Some(1.0);
        let report = build_stock_review_report_from_cached_data(&input).unwrap();
        assert_eq!(
            report.data_quality.actual_result_availability.status,
            MetricStatus::Available
        );
        assert_eq!(
            report
                .methodology
                .exchange_rate_coverage
                .availability
                .status,
            MetricStatus::Degraded
        );
        assert!(report
            .data_quality
            .issues
            .iter()
            .any(|issue| issue.code == "fx_forward_fill"));
    }

    #[test]
    fn non_base_external_flow_uses_cached_daily_fx_or_degrades_honestly() {
        let db = Database::new(":memory:").unwrap();
        let rates = crate::models::ExchangeRates {
            usd_cny: 7.0,
            usd_hkd: 7.8,
            cny_hkd: 1.1,
            updated_at: "2024-01-02T00:00:00Z".to_string(),
        };
        db.conn.lock().unwrap().execute(
            "INSERT INTO daily_portfolio_values (date, total_value, exchange_rates) VALUES ('2024-01-02', 1000, ?1)",
            params![serde_json::to_string(&rates).unwrap()],
        ).unwrap();
        let mut transaction = complete_cached_fixture(false).transactions[0].clone();
        transaction.symbol = "$CASH-CNY".to_string();
        transaction.currency = "CNY".to_string();
        transaction.total_amount = 700.0;
        transaction.traded_at = "2024-01-02T09:30:00Z".to_string();
        let query = complete_cached_fixture(false).query;
        let (flows, complete) =
            external_flows_base_from_db(&db, &[transaction], &query, day("2024-01-01"));
        assert!(complete);
        assert_eq!(flows.len(), 1);
        assert!((flows[0].amount_base - 100.0).abs() < 1e-12);
    }

    #[test]
    fn actual_snapshot_values_are_converted_to_the_requested_base_currency() {
        let db = Database::new(":memory:").unwrap();
        let rates = crate::models::ExchangeRates {
            usd_cny: 7.0,
            usd_hkd: 7.8,
            cny_hkd: 1.1,
            updated_at: "2024-01-02T00:00:00Z".to_string(),
        };
        let json = serde_json::to_string(&rates).unwrap();
        for (date, value) in [("2023-12-31", 1_000.0), ("2024-01-01", 1_100.0)] {
            db.conn.lock().unwrap().execute(
                "INSERT INTO daily_portfolio_values (date, total_value, exchange_rates) VALUES (?1, ?2, ?3)",
                params![date, value, json],
            ).unwrap();
        }
        let mut query = complete_cached_fixture(false).query;
        query.base_currency = "CNY".to_string();
        query.market = None;
        query.end_date = day("2024-01-01");
        let (baseline, values, availability) = load_actual_values(&db, &query).unwrap();
        assert_eq!(availability.status, MetricStatus::Available);
        assert_eq!(baseline.unwrap().value_base, 7_000.0);
        assert_eq!(values[0].value_base, 7_700.0);
    }

    #[test]
    fn builds_complete_report_from_cached_data() {
        // Removing any orchestration stage must break a consumer-visible report contract.
        let input = complete_cached_fixture(true);
        let report = build_stock_review_report_from_cached_data(&input).unwrap();

        assert_eq!(report.actions.len(), 1);
        assert_eq!(report.campaigns.len(), 1);
        assert_eq!(
            report.campaigns[0].action_ids,
            vec![report.actions[0].action_id.clone()]
        );
        assert_eq!(report.actions[0].status, MetricStatus::Available);
        assert_eq!(report.curves.first().unwrap().portfolio_return, Some(100.0));
        assert_eq!(report.curves.first().unwrap().shadow_return, Some(100.0));
        assert_eq!(report.curves.first().unwrap().benchmark_return, Some(100.0));
        assert!(report.curves.iter().all(|point| {
            point.portfolio_return.is_some()
                && point.shadow_return.is_some()
                && point.benchmark_return.is_some()
        }));
        assert_eq!(report.methodology.query.base_currency, "USD");
        assert_eq!(
            report.methodology.benchmark_symbol.as_deref(),
            Some("^GSPC")
        );
        assert_eq!(
            report.methodology.actual_return_method,
            "recorded_ledger_twr"
        );
        assert_eq!(report.methodology.algorithm_version, "stock-review-v1");
        assert_eq!(report.annotations.len(), 1);

        let without_annotations =
            build_stock_review_report_from_cached_data(&complete_cached_fixture(false)).unwrap();
        assert_eq!(report.summary, without_annotations.summary);
        assert_eq!(report.curves, without_annotations.curves);
        assert_eq!(report.attribution, without_annotations.attribution);
        assert_eq!(report.risk_structure, without_annotations.risk_structure);
    }
}
