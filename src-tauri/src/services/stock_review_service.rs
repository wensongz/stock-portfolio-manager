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
use crate::services::stock_action_builder::{build_stock_actions, CorrectedTransaction};
use crate::services::stock_campaign_builder::build_stock_campaigns;
use crate::services::stock_review_market_data::{
    default_benchmark_symbol, ensure_stock_price_cache, load_benchmark_series,
    load_market_sessions, load_stock_price_series, nth_market_session_after, DailyMarketPoint,
    MarketCalendar, MarketReturnMode,
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
use std::sync::atomic::{AtomicBool, Ordering};

const ALGORITHM_VERSION: &str = "stock-review-v1";

#[derive(Debug, Clone)]
pub struct CachedCampaignData {
    pub campaign_id: String,
    pub account_id: Option<String>,
    pub symbol: String,
    pub cash_flows: Vec<CampaignCashFlow>,
    pub position_events: Vec<CampaignPositionEvent>,
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

#[derive(Debug, Clone)]
struct RecordedSplit {
    symbol: String,
    date: NaiveDate,
    ratio: f64,
}

type SplitMarketAuthority = BTreeMap<String, BTreeSet<String>>;

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
    let scoped_transactions =
        derivation_transactions(&input.transactions, &input.overrides, &input.query);

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
        .filter(|action| action_is_in_query_scope(action, &input.query))
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
        result.metric.excess_return = None;
        result.metric.active_return = None;
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
    let mut value_add = calculate_rebalance_value_add(&RebalanceValueAddInput {
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
    if opening_cash_incomplete {
        value_add.metric.availability = MetricAvailability {
            status: MetricStatus::Unavailable,
            note: Some(
                "Shadow value-add is unavailable because opening cash is incomplete.".to_string(),
            ),
        };
    }

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

    let mut campaigns = project_campaigns(campaign_build.campaigns, &input.query);
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
                position_events: cached
                    .map(|value| value.position_events.clone())
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
                        annotation_applies_to_campaign(
                            annotation,
                            campaign,
                            &campaigns,
                            input.query.end_date,
                        )
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

fn annotation_applies_to_campaign(
    annotation: &StockReviewAnnotation,
    campaign: &StockCampaignSummary,
    all_campaigns: &[StockCampaignSummary],
    report_as_of: NaiveDate,
) -> bool {
    if !annotation_visible_as_of(annotation, report_as_of) {
        return false;
    }
    if annotation
        .account_id
        .as_ref()
        .is_some_and(|account_id| !campaign.account_ids.contains(account_id))
    {
        return false;
    }
    match annotation.scope_type.as_str() {
        "campaign" => annotation.scope_key == campaign.campaign_id,
        "action" => campaign.action_ids.contains(&annotation.scope_key),
        "stock" => {
            let symbol_matches = stock_symbols_equal(&annotation.scope_key, &campaign.symbol)
                && annotation
                    .symbol
                    .as_ref()
                    .is_none_or(|symbol| stock_symbols_equal(symbol, &campaign.symbol));
            if !symbol_matches {
                return false;
            }

            let Ok(dates) =
                stock_review_persistence::annotation_economic_dates(&annotation.value_json)
            else {
                return false;
            };
            let explicit_date = dates.effective_date.or(dates.snapshot_date);
            let explicit_start = dates.effective_start;
            let explicit_end = dates.effective_end;
            let campaign_start = campaign
                .started_at
                .get(..10)
                .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok());
            if campaign_start.is_none_or(|start| start > report_as_of) {
                return false;
            }
            let campaign_end = campaign
                .ended_at
                .as_deref()
                .and_then(|date| date.get(..10))
                .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
                .unwrap_or(report_as_of)
                .min(report_as_of);

            if let (Some(effective), Some(start)) = (explicit_date, campaign_start) {
                return effective >= start && effective <= campaign_end;
            }
            if explicit_start.is_some() || explicit_end.is_some() {
                let Some(start) = campaign_start else {
                    return false;
                };
                let annotation_start = explicit_start.unwrap_or(NaiveDate::MIN);
                let annotation_end = explicit_end.unwrap_or(NaiveDate::MAX);
                return annotation_start <= campaign_end && annotation_end >= start;
            }

            // An undated stock annotation is campaign-specific only when there
            // is exactly one unambiguous account/symbol lifetime. Otherwise it
            // remains visible at report/stock scope instead of leaking into
            // every same-symbol cycle.
            all_campaigns
                .iter()
                .filter(|candidate| {
                    stock_symbols_equal(&candidate.symbol, &campaign.symbol)
                        && annotation
                            .account_id
                            .as_ref()
                            .is_none_or(|account_id| candidate.account_ids.contains(account_id))
                })
                .count()
                == 1
        }
        _ => false,
    }
}

fn annotation_visible_as_of(annotation: &StockReviewAnnotation, report_as_of: NaiveDate) -> bool {
    let Ok(dates) = stock_review_persistence::annotation_economic_dates(&annotation.value_json)
    else {
        return false;
    };
    dates
        .effective_date
        .or(dates.snapshot_date)
        .is_none_or(|date| date <= report_as_of)
        && dates
            .effective_start
            .is_none_or(|date| date <= report_as_of)
}

fn transaction_date(transaction: &Transaction) -> Option<NaiveDate> {
    transaction
        .traded_at
        .get(..10)
        .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
}

fn derivation_transactions(
    transactions: &[Transaction],
    overrides: &[StockReviewOverride],
    query: &StockReviewQuery,
) -> Vec<Transaction> {
    let in_requested_scope = |transaction: &Transaction| {
        query
            .account_id
            .as_ref()
            .is_none_or(|account| transaction.account_id == *account)
            && query
                .market
                .as_ref()
                .is_none_or(|market| transaction.market == *market)
            && transaction_date(transaction).is_some_and(|date| date <= query.end_date)
    };
    let base_ids = transactions
        .iter()
        .filter(|transaction| in_requested_scope(transaction))
        .map(|transaction| transaction.id.as_str())
        .collect::<BTreeSet<_>>();
    let referenced_transfer_ids = overrides
        .iter()
        .filter(|record| record.override_type == "transfer")
        .filter_map(|record| serde_json::from_str::<Vec<String>>(&record.transaction_ids_json).ok())
        .filter(|ids| ids.iter().any(|id| base_ids.contains(id.as_str())))
        .flatten()
        .collect::<BTreeSet<_>>();
    let referenced_position_keys = transactions
        .iter()
        .filter(|transaction| referenced_transfer_ids.contains(&transaction.id))
        .map(|transaction| {
            (
                transaction.account_id.clone(),
                normalized_stock_symbol(&transaction.symbol).unwrap_or_default(),
                normalized_stock_market(&transaction.market).unwrap_or_default(),
            )
        })
        .collect::<BTreeSet<_>>();
    transactions
        .iter()
        .filter(|transaction| {
            in_requested_scope(transaction)
                || (referenced_transfer_ids.contains(&transaction.id)
                    && transaction_date(transaction).is_some_and(|date| date <= query.end_date))
                || (referenced_position_keys.contains(&(
                    transaction.account_id.clone(),
                    normalized_stock_symbol(&transaction.symbol).unwrap_or_default(),
                    normalized_stock_market(&transaction.market).unwrap_or_default(),
                )) && transaction_date(transaction).is_some_and(|date| date <= query.end_date))
        })
        .cloned()
        .collect()
}

fn action_is_in_query_scope(action: &StockActionReview, query: &StockReviewQuery) -> bool {
    query
        .account_id
        .as_ref()
        .is_none_or(|account| action.account_id == *account)
        && query
            .market
            .as_ref()
            .is_none_or(|market| action.market == *market)
}

fn transaction_is_in_query_scope(transaction: &Transaction, query: &StockReviewQuery) -> bool {
    query
        .account_id
        .as_ref()
        .is_none_or(|account| transaction.account_id == *account)
        && query
            .market
            .as_ref()
            .is_none_or(|market| transaction.market == *market)
}

fn project_campaigns(
    campaigns: Vec<StockCampaignSummary>,
    query: &StockReviewQuery,
) -> Vec<StockCampaignSummary> {
    campaigns
        .into_iter()
        .filter_map(|mut campaign| {
            campaign.fragments.retain(|fragment| {
                query
                    .account_id
                    .as_ref()
                    .is_none_or(|account| fragment.account_id == *account)
                    && query
                        .market
                        .as_ref()
                        .is_none_or(|market| fragment.market == *market)
            });
            if campaign.fragments.is_empty() {
                return None;
            }
            campaign.account_ids = campaign
                .fragments
                .iter()
                .map(|fragment| fragment.account_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            campaign.action_ids = campaign
                .fragments
                .iter()
                .flat_map(|fragment| fragment.action_ids.iter().cloned())
                .collect();
            Some(campaign)
        })
        .collect()
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

fn validate_override_query_scope(
    db: &Database,
    query: &StockReviewQuery,
    input: &StockReviewOverrideInput,
) -> Result<(), String> {
    let ids = serde_json::from_str::<Vec<String>>(&input.transaction_ids_json)
        .map_err(|error| error.to_string())?;
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    for id in ids {
        let in_scope = conn
            .query_row(
                "SELECT EXISTS(
                    SELECT 1 FROM transactions
                    WHERE id = ?1
                      AND substr(traded_at, 1, 10) <= ?2
                      AND (?3 IS NULL OR account_id = ?3)
                      AND (?4 IS NULL OR market = ?4)
                 )",
                params![
                    id,
                    query.end_date.format("%Y-%m-%d").to_string(),
                    query.account_id,
                    query.market,
                ],
                |row| row.get::<_, bool>(0),
            )
            .map_err(|error| error.to_string())?;
        if !in_scope {
            return Err(format!(
                "Correction reference '{id}' is outside the selected account, market, or report cutoff and cannot affect this report."
            ));
        }
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

/// AI projection boundary: materialize one Task 9 input snapshot and build the
/// report plus optional Campaign detail from the same artifact set.
pub(crate) async fn get_stock_review_for_ai(
    db: &Database,
    query: StockReviewQuery,
    campaign_id: Option<&str>,
) -> Result<(StockReviewReport, Option<StockCampaignDetail>), String> {
    let input = prepare_cached_stock_review_input(db, query).await?;
    let artifacts = build_stock_review_artifacts(&input)?;
    let campaign_detail = if let Some(campaign_id) = campaign_id {
        Some(
            artifacts
                .campaign_details
                .into_iter()
                .find(|detail| detail.summary.campaign_id == campaign_id)
                .ok_or_else(|| format!("Campaign '{campaign_id}' was not found in this report."))?,
        )
    } else {
        None
    };
    Ok((artifacts.report, campaign_detail))
}

pub fn save_user_stock_review_annotation(
    db: &Database,
    mut input: StockReviewAnnotationInput,
) -> Result<StockReviewAnnotation, String> {
    input.source = "user".to_string();
    stock_review_persistence::save_annotation(db, input, AnnotationSaveContext::UserInitiated)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConfirmedAnnotationDraftBinding {
    id: String,
    scope_type: String,
    scope_key: String,
    account_id: Option<String>,
    symbol: Option<String>,
    annotation_type: String,
    source: String,
    value_hash: u64,
    canonical_value: String,
}

/// Private host-issued approval artifact. Model text and tool JSON cannot
/// construct it; a future trusted UI confirmation event may be wired to the
/// private constructor in this module. The artifact is exact-draft-bound and
/// one-shot even when a model repeats the same tool call in one turn.
pub(crate) struct ConfirmedAiAnnotationCapability {
    binding: ConfirmedAnnotationDraftBinding,
    consumed: AtomicBool,
}

fn canonical_json(value: &serde_json::Value, output: &mut String) {
    match value {
        serde_json::Value::Null => output.push_str("null"),
        serde_json::Value::Bool(value) => output.push_str(if *value { "true" } else { "false" }),
        serde_json::Value::Number(value) => output.push_str(&value.to_string()),
        serde_json::Value::String(value) => {
            output.push_str(&serde_json::to_string(value).unwrap_or_else(|_| "\"\"".to_string()))
        }
        serde_json::Value::Array(values) => {
            output.push('[');
            for (index, value) in values.iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                canonical_json(value, output);
            }
            output.push(']');
        }
        serde_json::Value::Object(values) => {
            output.push('{');
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            for (index, (key, value)) in entries.into_iter().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&serde_json::to_string(key).unwrap_or_else(|_| "\"\"".to_string()));
                output.push(':');
                canonical_json(value, output);
            }
            output.push('}');
        }
    }
}

fn stable_value_hash(value: &serde_json::Value) -> u64 {
    let mut canonical = String::new();
    canonical_json(value, &mut canonical);
    canonical.bytes().fold(0xcbf29ce484222325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
    })
}

fn confirmed_annotation_binding(
    input: &StockReviewAnnotationInput,
) -> Result<ConfirmedAnnotationDraftBinding, String> {
    let value: serde_json::Value = serde_json::from_str(&input.value_json)
        .map_err(|error| format!("annotation value is invalid JSON: {error}"))?;
    if !value.is_object() {
        return Err("annotation value must be a JSON object".to_string());
    }
    let scope_type = input.scope_type.trim().to_ascii_lowercase();
    let scope_key = if scope_type == "stock" {
        normalized_stock_symbol(&input.scope_key).unwrap_or_default()
    } else {
        input.scope_key.trim().to_string()
    };
    let mut canonical_value = String::new();
    canonical_json(&value, &mut canonical_value);
    Ok(ConfirmedAnnotationDraftBinding {
        id: input.id.trim().to_string(),
        scope_type,
        scope_key,
        account_id: input
            .account_id
            .as_deref()
            .map(str::trim)
            .map(str::to_string),
        symbol: input.symbol.as_deref().and_then(normalized_stock_symbol),
        annotation_type: input.annotation_type.trim().to_string(),
        source: input.source.trim().to_ascii_lowercase(),
        value_hash: stable_value_hash(&value),
        canonical_value,
    })
}

#[cfg(test)]
pub(crate) fn confirmed_ai_annotation_capability_for_test(
    input: &StockReviewAnnotationInput,
) -> ConfirmedAiAnnotationCapability {
    let mut input = input.clone();
    input.source = "ai_confirmed".to_string();
    ConfirmedAiAnnotationCapability {
        binding: confirmed_annotation_binding(&input).expect("valid approved test draft"),
        consumed: AtomicBool::new(false),
    }
}

pub(crate) fn save_ai_confirmed_stock_review_annotation(
    db: &Database,
    mut input: StockReviewAnnotationInput,
    capability: &ConfirmedAiAnnotationCapability,
) -> Result<StockReviewAnnotation, String> {
    input.source = "ai_confirmed".to_string();
    let binding = confirmed_annotation_binding(&input)?;
    if capability.consumed.load(Ordering::SeqCst) {
        return Err("confirmation_required: this approval was already consumed".to_string());
    }
    if capability.binding != binding {
        return Err(
            "confirmation_required: approved annotation draft does not match this write"
                .to_string(),
        );
    }
    if capability.consumed.swap(true, Ordering::SeqCst) {
        return Err("confirmation_required: this approval was already consumed".to_string());
    }
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
    let mut prepared_candidate = stock_review_persistence::prepare_override_candidate(db, input)?;
    let input = prepared_candidate.input.clone();
    validate_override_query_scope(db, &query, &input)?;
    stock_review_persistence::scope_candidate_to_query(db, &mut prepared_candidate, &query)?;
    let candidate_record = StockReviewOverride {
        id: input.id.clone(),
        override_type: input.override_type.clone(),
        transaction_ids_json: input.transaction_ids_json.clone(),
        value_json: input.value_json.clone(),
        created_at: Utc::now().to_rfc3339(),
        updated_at: Utc::now().to_rfc3339(),
    };
    let cached = prepare_cached_stock_review_input_with_candidate(
        db,
        query,
        Some(candidate_record),
        Some(&mut prepared_candidate),
    )
    .await?;
    let candidate_report = build_stock_review_report_from_cached_data(&cached)?;
    if candidate_report.data_quality.issues.iter().any(|issue| {
        matches!(
            issue.code.as_str(),
            "negative_position" | "unexplained_position_path" | "campaign_unavailable"
        ) && issue.severity == StockReviewIssueSeverity::Error
    }) {
        return Err(
            "The correction makes the position replay inconsistent; no override was saved."
                .to_string(),
        );
    }
    stock_review_persistence::save_override_candidate(db, prepared_candidate)?;
    Ok(candidate_report)
}

async fn prepare_cached_stock_review_input(
    db: &Database,
    query: StockReviewQuery,
) -> Result<CachedStockReviewInput, String> {
    prepare_cached_stock_review_input_with_candidate(db, query, None, None).await
}

async fn prepare_cached_stock_review_input_with_candidate(
    db: &Database,
    query: StockReviewQuery,
    candidate: Option<StockReviewOverride>,
    candidate_revision: Option<&mut stock_review_persistence::ValidatedOverrideCandidate>,
) -> Result<CachedStockReviewInput, String> {
    prepare_cached_stock_review_input_with_candidate_and_cache_hook(
        db,
        query,
        candidate,
        candidate_revision,
        |_| Ok(()),
    )
    .await
}

async fn prepare_cached_stock_review_input_with_candidate_and_cache_hook<F>(
    db: &Database,
    query: StockReviewQuery,
    candidate: Option<StockReviewOverride>,
    mut candidate_revision: Option<&mut stock_review_persistence::ValidatedOverrideCandidate>,
    after_cache_fill: F,
) -> Result<CachedStockReviewInput, String>
where
    F: FnOnce(&Database) -> Result<(), String>,
{
    validate_query(&query)?;
    validate_account_exists(db, query.account_id.as_deref())?;
    let today = Utc::now().date_naive();
    let candidate_record = candidate.clone();
    let mut override_list = stock_review_persistence::list_overrides_for_query(db, &query, today)?;
    if let Some(candidate) = candidate.clone() {
        let candidate_id = candidate.id.clone();
        override_list
            .overrides
            .retain(|record| record.id != candidate_id);
        override_list
            .stale_overrides
            .retain(|record| record.id != candidate_id);
        let stale_message_prefix = format!("Override {candidate_id} ");
        override_list.issues.retain(|issue| {
            issue.code != "stale_override" || !issue.message.starts_with(&stale_message_prefix)
        });
        override_list.overrides.push(candidate);
    }
    let transactions = load_all_transactions_for_review_through(db, today)?;
    let discovered_split_market_authority = load_split_market_authority(db, &transactions)?;
    let derivation_ledger =
        derivation_transactions(&transactions, &override_list.overrides, &query);
    let action_build = build_stock_actions(&derivation_ledger, &override_list.overrides);
    let scoped_actions = action_build
        .actions
        .iter()
        .filter(|action| action_is_in_query_scope(action, &query))
        .cloned()
        .collect::<Vec<_>>();
    let scoped_corrected_transactions = action_build
        .corrected_transactions
        .iter()
        .filter(|corrected| transaction_is_in_query_scope(&corrected.transaction, &query))
        .cloned()
        .collect::<Vec<_>>();
    let corrected_transactions = &scoped_corrected_transactions;
    let provider_config = crate::services::quote_provider_service::get_quote_provider_config(db)?;
    let mut security_keys = corrected_transactions
        .iter()
        .map(|corrected| &corrected.transaction)
        .filter(|transaction| {
            transaction_date(transaction).is_some_and(|date| date <= query.end_date)
        })
        .filter(|transaction| !crate::services::quote_service::is_cash_symbol(&transaction.symbol))
        .map(|transaction| (transaction.symbol.clone(), transaction.market.clone()))
        .collect::<BTreeSet<_>>();
    for (symbol, market) in load_current_holding_keys(db, &query)? {
        security_keys.insert((symbol, market));
    }
    let price_start = corrected_transactions
        .iter()
        .map(|corrected| &corrected.transaction)
        .filter(|transaction| !crate::services::quote_service::is_cash_symbol(&transaction.symbol))
        .filter_map(transaction_date)
        .min()
        .map_or(query.start_date - Duration::days(10), |date| {
            date.min(query.start_date - Duration::days(10))
        });
    let mut local_markets = security_keys
        .iter()
        .map(|(_, market)| market.clone())
        .collect::<BTreeSet<_>>();
    let mut market_calendars_by_market = BTreeMap::new();
    for market in &local_markets {
        market_calendars_by_market.insert(
            market.clone(),
            load_market_sessions(db, market, price_start, today)?,
        );
    }
    let market_sessions_by_market = market_calendars_by_market
        .iter()
        .map(|(market, calendar)| (market.clone(), calendar.sessions.clone()))
        .collect::<BTreeMap<_, _>>();
    let mut evaluation_end = scoped_actions
        .iter()
        .filter(|action| action_date(action).is_some_and(|date| date <= query.end_date))
        .filter_map(|action| {
            let action_date = action_date(action)?;
            Some(
                market_sessions_by_market
                    .get(&action.market)
                    .and_then(|sessions| nth_market_session_after(sessions, action_date, 120))
                    .unwrap_or(today),
            )
        })
        .max()
        .unwrap_or(query.end_date.min(today))
        .max(query.end_date.min(today));
    let mut price_end = evaluation_end;
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
    let mut local_benchmark_points_by_market = BTreeMap::new();
    for market in &local_markets {
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
            market.clone(),
            load_benchmark_series(db, symbol, price_start, price_end)?,
        );
    }

    for (market, _) in &benchmark_specs {
        if market != "CUSTOM" {
            local_markets.insert(market.clone());
        }
    }
    let benchmark_symbols = benchmark_specs
        .iter()
        .map(|(_, symbol)| symbol.clone())
        .chain(
            local_markets
                .iter()
                .filter_map(|market| default_benchmark_symbol(market).map(str::to_string)),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let revision_currencies = corrected_transactions
        .iter()
        .map(|corrected| corrected.transaction.currency.clone())
        .chain(load_current_holding_currencies(db, &query)?.into_iter())
        .chain(
            security_keys
                .iter()
                .map(|(_, market)| market_currency(market).to_string()),
        )
        .chain(std::iter::once(query.base_currency.clone()))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    after_cache_fill(db)?;

    if let Some(candidate) = candidate_revision.as_deref() {
        stock_review_persistence::verify_candidate_discovery_revision_after_cache_fill(
            db, candidate,
        )?;
    }
    // Cache fills or concurrent cache writers may change the authoritative
    // session plan. Re-read the complete discovery horizon and derive the
    // exact 120-session endpoint before pinning the cache revision.
    let refreshed_calendars = local_markets
        .iter()
        .map(|market| {
            load_market_sessions(db, market, price_start, today)
                .map(|calendar| (market.clone(), calendar))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let refreshed_sessions = refreshed_calendars
        .iter()
        .map(|(market, calendar)| (market.clone(), calendar.sessions.clone()))
        .collect::<BTreeMap<_, _>>();
    evaluation_end = scoped_actions
        .iter()
        .filter(|action| action_date(action).is_some_and(|date| date <= query.end_date))
        .filter_map(|action| {
            let action_date = action_date(action)?;
            Some(
                refreshed_sessions
                    .get(&action.market)
                    .and_then(|sessions| nth_market_session_after(sessions, action_date, 120))
                    .unwrap_or(today),
            )
        })
        .max()
        .unwrap_or(query.end_date.min(today))
        .max(query.end_date.min(today));
    price_end = evaluation_end;
    if let Some(candidate) = candidate_revision.as_deref_mut() {
        stock_review_persistence::set_candidate_revision_scope(
            candidate,
            stock_review_persistence::CandidateRevisionScope {
                report_start: query.start_date,
                report_end: query.end_date,
                price_start,
                evaluation_end,
                current_horizon: today,
                display_cutoff: query.end_date,
                account_ids: query.account_id.clone().into_iter().collect(),
                markets: local_markets.iter().cloned().collect(),
                securities: security_keys.iter().cloned().collect(),
                benchmark_symbols,
                currencies: revision_currencies,
            },
        );
        stock_review_persistence::pin_candidate_source_revision_after_cache_fill(db, candidate)?;
    }

    // Every mutable source used by the candidate is loaded again after the
    // async cache phase has finished and the exact dependency scope is pinned.
    // The final digest check below rejects any mutation during these reads.
    let mut override_list = stock_review_persistence::list_overrides_for_query(db, &query, today)?;
    if let Some(candidate) = candidate_record {
        let candidate_id = candidate.id.clone();
        override_list
            .overrides
            .retain(|record| record.id != candidate_id);
        override_list
            .stale_overrides
            .retain(|record| record.id != candidate_id);
        let stale_message_prefix = format!("Override {candidate_id} ");
        override_list.issues.retain(|issue| {
            issue.code != "stale_override" || !issue.message.starts_with(&stale_message_prefix)
        });
        override_list.overrides.push(candidate);
    }
    let transactions = load_all_transactions_for_review_through(db, today)?;
    let split_market_authority = load_split_market_authority(db, &transactions)?;
    if split_market_authority != discovered_split_market_authority {
        return Err(
            "Security market identities changed during cache preparation; rebuild before confirming."
                .to_string(),
        );
    }
    let derivation_ledger =
        derivation_transactions(&transactions, &override_list.overrides, &query);
    let action_build = build_stock_actions(&derivation_ledger, &override_list.overrides);
    let scoped_actions = action_build
        .actions
        .iter()
        .filter(|action| action_is_in_query_scope(action, &query))
        .cloned()
        .collect::<Vec<_>>();
    let scoped_position_events = action_build
        .position_events
        .iter()
        .filter(|event| {
            query
                .account_id
                .as_ref()
                .is_none_or(|account| event.account_id == *account)
                && query
                    .market
                    .as_ref()
                    .is_none_or(|market| event.market == *market)
        })
        .cloned()
        .collect::<Vec<_>>();
    let scoped_corrected_transactions = action_build
        .corrected_transactions
        .iter()
        .filter(|corrected| transaction_is_in_query_scope(&corrected.transaction, &query))
        .cloned()
        .collect::<Vec<_>>();
    let corrected_transactions = &scoped_corrected_transactions;
    let prepared_campaigns = build_stock_campaigns(
        &action_build.position_events,
        &action_build.actions,
        &override_list.overrides,
        query.end_date,
    );
    let prepared_campaign_summaries = project_campaigns(prepared_campaigns.campaigns, &query);
    let mut reloaded_security_keys = corrected_transactions
        .iter()
        .map(|corrected| &corrected.transaction)
        .filter(|transaction| {
            transaction_date(transaction).is_some_and(|date| date <= query.end_date)
        })
        .filter(|transaction| !crate::services::quote_service::is_cash_symbol(&transaction.symbol))
        .map(|transaction| (transaction.symbol.clone(), transaction.market.clone()))
        .collect::<BTreeSet<_>>();
    for key in load_current_holding_keys(db, &query)? {
        reloaded_security_keys.insert(key);
    }
    if reloaded_security_keys != security_keys {
        return Err(
            "User-owned security dependencies changed during cache preparation; rebuild before confirming."
                .to_string(),
        );
    }
    let security_keys = reloaded_security_keys;
    let market_calendars_by_market = local_markets
        .iter()
        .map(|market| {
            load_market_sessions(db, market, price_start, evaluation_end)
                .map(|calendar| (market.clone(), calendar))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let market_sessions_by_market = market_calendars_by_market
        .iter()
        .map(|(market, calendar)| (market.clone(), calendar.sessions.clone()))
        .collect::<BTreeMap<_, _>>();
    let prices_by_security = security_keys
        .iter()
        .map(|(symbol, market)| {
            load_stock_price_series(db, symbol, market, price_start, price_end)
                .map(|points| ((symbol.clone(), market.clone()), points))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;
    let mut benchmark_series = Vec::new();
    let mut benchmark_points_by_market = BTreeMap::new();
    for (market, symbol) in &benchmark_specs {
        let points = load_benchmark_series(db, symbol, price_start, price_end)?;
        benchmark_series.push(BenchmarkSeriesInput {
            market: market.clone(),
            availability: cached_point_availability(&points, query.start_date, query.end_date),
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
    let local_benchmark_points_by_market = local_markets
        .iter()
        .filter_map(|market| default_benchmark_symbol(market).map(|symbol| (market, symbol)))
        .map(|(market, symbol)| {
            load_benchmark_series(db, symbol, price_start, price_end)
                .map(|points| (market.clone(), points))
        })
        .collect::<Result<BTreeMap<_, _>, _>>()?;

    let valuation_markets = security_keys
        .iter()
        .map(|(_, market)| market.clone())
        .chain(query.market.clone())
        .collect::<BTreeSet<_>>();
    let (expected_actual_dates, expected_baseline_date) = portfolio_valuation_session_authority(
        &query,
        price_start,
        &valuation_markets,
        &market_calendars_by_market,
    );
    let (baseline, actual_values, mut actual_availability, actual_nav_complete) =
        load_actual_values(db, &query, expected_baseline_date)?;
    let actual_origin_date = expected_baseline_date
        .or_else(|| expected_actual_dates.first().copied())
        .unwrap_or(query.start_date);
    let (external_flows, external_flows_complete) =
        external_flows_base_from_db(db, corrected_transactions, &query, actual_origin_date);
    if !external_flows_complete {
        actual_availability = MetricAvailability {
            status: MetricStatus::Unavailable,
            note: Some(
                "Actual TWR is unavailable because a non-base external flow lacks cached daily FX."
                    .to_string(),
            ),
        };
    }
    let shadow_external_flows =
        external_flow_events(corrected_transactions, actual_origin_date, query.end_date);
    let recorded_splits = load_recorded_splits(db, today)?;
    let opening_positions = opening_positions(
        db,
        &query,
        &scoped_position_events,
        &recorded_splits,
        &split_market_authority,
        actual_origin_date,
    )?;
    let (opening_cash, opening_cash_complete) =
        opening_cash(db, corrected_transactions, &query, actual_origin_date)?;
    let mut preparation_issues = Vec::new();
    if actual_availability
        .note
        .as_deref()
        .is_some_and(|note| note.contains("exact daily FX"))
    {
        preparation_issues.push(StockReviewIssue {
            code: "snapshot_fx_unavailable".to_string(),
            severity: StockReviewIssueSeverity::Error,
            message: "Filtered snapshot NAV, turnover, and fee drag are unavailable because at least one local market value lacks exact daily FX to the requested base currency.".to_string(),
            affected_symbol: None,
            affected_date: None,
        });
    }
    if (query.account_id.is_some() || query.market.is_some()) && !actual_values.is_empty() {
        preparation_issues.push(StockReviewIssue {
            code: "filtered_nav_cash_unavailable".to_string(),
            severity: StockReviewIssueSeverity::Error,
            message: "Filtered holding snapshots contain stock value but no authoritative account/market cash total; average NAV, turnover, and fee drag are unavailable.".to_string(),
            affected_symbol: None,
            affected_date: None,
        });
    }
    if !opening_cash_complete {
        preparation_issues.push(StockReviewIssue {
            code: "opening_cash_incomplete".to_string(),
            severity: StockReviewIssueSeverity::Error,
            message: "Opening cash cannot be reconstructed from a complete cash ledger or an authoritative current cash balance; shadow and fixed-weight benchmark outputs are unavailable.".to_string(),
            affected_symbol: None,
            affected_date: Some(actual_origin_date),
        });
    }
    for (market, calendar) in &market_calendars_by_market {
        preparation_issues.push(if calendar.availability.status == MetricStatus::Unavailable {
            StockReviewIssue {
                code: "market_calendar_unavailable".to_string(),
                severity: StockReviewIssueSeverity::Error,
                message: format!(
                    "No authoritative {market} exchange-session calendar is cached; exact forward and Campaign session metrics are unavailable."
                ),
                affected_symbol: None,
                affected_date: None,
            }
        } else {
            StockReviewIssue {
                code: "market_calendar_authority".to_string(),
                severity: StockReviewIssueSeverity::Info,
                message: format!(
                    "{market} sessions use the explicit exchange-session calendar cache; quote rows are prices only."
                ),
                affected_symbol: None,
                affected_date: None,
            }
        });
    }
    let valuation_dates = std::iter::once(actual_origin_date)
        .chain(expected_actual_dates.iter().copied())
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let fx_dates = valuation_dates
        .iter()
        .copied()
        .chain(std::iter::once(query.end_date))
        .chain(
            action_build
                .actions
                .iter()
                .filter_map(action_date)
                .filter(|date| *date <= query.end_date),
        )
        .chain(
            corrected_transactions
                .iter()
                .filter_map(|corrected| transaction_date(&corrected.transaction))
                .filter(|date| *date <= query.end_date),
        )
        .chain(
            prices_by_security
                .values()
                .flat_map(|points| points.iter().map(|point| point.date))
                .filter(|date| *date <= query.end_date),
        )
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let required_currencies = corrected_transactions
        .iter()
        .map(|corrected| corrected.transaction.currency.clone())
        .chain(
            opening_positions
                .iter()
                .map(|position| position.currency.clone()),
        )
        .chain(opening_cash.iter().map(|cash| cash.currency.clone()))
        .chain(
            security_keys
                .iter()
                .map(|(_, market)| market_currency(market).to_string()),
        )
        .collect::<BTreeSet<_>>();
    let mut fx_points = load_daily_fx_points(
        db,
        &fx_dates,
        &query.base_currency,
        required_currencies.iter().map(String::as_str),
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
            points
                .iter()
                .filter(move |point| {
                    point.date >= actual_origin_date && point.date <= query.end_date
                })
                .map(move |point| ShadowPricePoint {
                    date: point.date,
                    symbol: symbol.clone(),
                    market: market.clone(),
                    currency: currency.clone(),
                    close: point.close,
                    adjusted_close: point.adjusted_close,
                })
        })
        .collect::<Vec<_>>();
    let split_events = load_split_events(
        &recorded_splits,
        &opening_positions,
        &scoped_position_events,
        &split_market_authority,
        actual_origin_date,
        query.end_date,
    );
    preparation_issues.extend(ambiguous_split_issues(
        &recorded_splits,
        &opening_positions,
        &scoped_position_events,
        &split_market_authority,
    ));
    let complete_shadow_dividends = complete_shadow_dividend_events(
        &opening_positions,
        &prices_by_security,
        &market_sessions_by_market,
        actual_origin_date,
        query.end_date,
    );
    let adjusted_close_complete = shadow_total_return_field_complete(
        &opening_positions,
        &prices_by_security,
        &market_sessions_by_market,
        actual_origin_date,
        query.end_date,
        |point| point.adjusted_close.is_some(),
    );
    let (return_method, dividend_events) = if let Some(events) = complete_shadow_dividends {
        (ShadowReturnMethod::ExplicitDividends, events)
    } else if adjusted_close_complete {
        (ShadowReturnMethod::AdjustedClose, Vec::new())
    } else {
        preparation_issues.push(StockReviewIssue {
            code: "shadow_dividend_source_incomplete".to_string(),
            severity: StockReviewIssueSeverity::Warning,
            message: "A complete adjusted-close or per-session corporate-action dividend source is unavailable; shadow returns are price-only. Account PAY rows remain actual cash income and do not certify shadow dividends.".to_string(),
            affected_symbol: None,
            affected_date: None,
        });
        (ShadowReturnMethod::PriceOnly, Vec::new())
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
            let sessions = market_sessions_by_market.get(&action.market)?;
            let calendar = market_calendars_by_market.get(&action.market)?;
            let target_120 = nth_market_session_after(sessions, date, 120);
            let calendar_covers_window = calendar.complete_start.is_some_and(|start| start <= date)
                && (target_120.is_some()
                    || calendar
                        .complete_through
                        .is_some_and(|through| through >= today));
            let fx = exact_fx_on(
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
                market_session_dates: sessions.clone(),
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
                availability: if !calendar_covers_window
                    || calendar.availability.status == MetricStatus::Unavailable
                {
                    MetricAvailability {
                        status: MetricStatus::Unavailable,
                        note: Some(
                            "Authoritative exchange-session calendar is unavailable.".to_string(),
                        ),
                    }
                } else {
                    cached_point_availability(stock, date, price_end)
                },
            })
        })
        .collect::<Vec<_>>();
    let campaign_data = prepared_campaign_summaries
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
            let sessions = market_sessions_by_market.get(market)?;
            let calendar = market_calendars_by_market.get(market)?;
            let calendar_covers_campaign = calendar.covers(campaign_start, campaign_end);
            let currency = market_currency(market);
            let expected_session_dates = if calendar_covers_campaign {
                sessions
                    .iter()
                    .filter(|date| **date >= campaign_start && **date <= campaign_end)
                    .copied()
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            let mut campaign_issues = Vec::new();
            campaign_issues.push(StockReviewIssue {
                code: if !calendar_covers_campaign {
                    "campaign_calendar_unavailable"
                } else {
                    "campaign_calendar_authority"
                }
                .to_string(),
                severity: if !calendar_covers_campaign {
                    StockReviewIssueSeverity::Error
                } else {
                    StockReviewIssueSeverity::Info
                },
                message: if !calendar_covers_campaign {
                    "No authoritative exchange-session calendar covers this Campaign."
                        .to_string()
                } else {
                    format!("Campaign sessions use the explicit {market} exchange calendar.")
                },
                affected_symbol: Some(symbol.clone()),
                affected_date: None,
            });
            if expected_session_dates.is_empty()
                && !campaign_issues
                    .iter()
                    .any(|issue| issue.code == "campaign_calendar_unavailable")
            {
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
            let cash_flows = campaign_cash_flows(
                corrected_transactions,
                symbol,
                market,
                &campaign.account_ids,
                campaign_start,
                campaign_end,
                &query,
                &fx_points,
            );
            let position_events = campaign_position_events(
                &action_build.position_events,
                &split_events,
                campaign,
                query.account_id.as_deref(),
            );
            if let Some(flow) = cash_flows.iter().find(|flow| flow.amount_base.is_none()) {
                campaign_issues.push(StockReviewIssue {
                    code: "campaign_fx_unavailable".to_string(),
                    severity: StockReviewIssueSeverity::Error,
                    message: format!(
                        "Campaign P&L and excursion metrics are unavailable because {} on {} lacks exact daily FX.",
                        flow.currency, flow.date
                    ),
                    affected_symbol: Some(symbol.clone()),
                    affected_date: Some(flow.date),
                });
            }
            Some(CachedCampaignData {
                campaign_id: campaign.campaign_id.clone(),
                account_id: query.account_id.clone(),
                symbol: symbol.clone(),
                cash_flows,
                position_events,
                daily_prices: stock
                    .iter()
                    .filter(|point| point.date >= campaign_start && point.date <= campaign_end)
                    .map(|point| CampaignPricePoint {
                        date: point.date,
                        currency: currency.to_string(),
                        low: point.low,
                        high: point.high,
                        close: Some(point.close),
                        fx_to_base: exact_fx_on(
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
                    exact_fx_on(currency, &query.base_currency, &fx_points, point.date)
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
            exact_fx_on(
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
    let average_nav = if actual_values.is_empty() || !actual_nav_complete {
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
        corrected_transactions,
        &scoped_actions,
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
    let (risk_input, action_fx_complete) =
        load_risk_input(db, &query, &scoped_actions, average_nav, &fx_points)?;
    if !action_fx_complete {
        preparation_issues.push(StockReviewIssue {
            code: "action_fx_unavailable".to_string(),
            severity: StockReviewIssueSeverity::Error,
            message: "Turnover and fee drag are unavailable because at least one scoped stock action lacks exact action-date FX.".to_string(),
            affected_symbol: None,
            affected_date: None,
        });
    }
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

    let cached = CachedStockReviewInput {
        query: query.clone(),
        transactions,
        overrides: override_list.overrides,
        persisted_override_issues: override_list.issues,
        preparation_issues,
        result_quality_input: ResultQualityInput {
            actual_origin_date,
            actual_values,
            baseline,
            expected_actual_dates,
            expected_baseline_date,
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
    };
    if let Some(candidate) = candidate_revision.as_deref() {
        stock_review_persistence::verify_candidate_source_revision(db, candidate)?;
    }
    Ok(cached)
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

fn load_all_transactions_for_review(db: &Database) -> Result<Vec<Transaction>, String> {
    load_all_transactions_for_review_through(db, Utc::now().date_naive())
}

fn load_all_transactions_for_review_through(
    db: &Database,
    end: NaiveDate,
) -> Result<Vec<Transaction>, String> {
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

fn load_current_holding_currencies(
    db: &Database,
    query: &StockReviewQuery,
) -> Result<Vec<String>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT DISTINCT currency FROM holdings
             WHERE shares != 0
               AND (?1 IS NULL OR account_id = ?1)
               AND (?2 IS NULL OR market = ?2)
             ORDER BY currency",
        )
        .map_err(|error| error.to_string())?;
    let currencies = statement
        .query_map(params![query.account_id, query.market], |row| row.get(0))
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(currencies)
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

/// Derive the portfolio observation contract only from explicit exchange
/// calendars. Price or snapshot rows are observations, never calendar
/// authority. A multi-market portfolio is expected to have a valuation on
/// every session where at least one scoped market is open.
fn portfolio_valuation_session_authority(
    query: &StockReviewQuery,
    authority_start: NaiveDate,
    markets: &BTreeSet<String>,
    calendars: &BTreeMap<String, MarketCalendar>,
) -> (Vec<NaiveDate>, Option<NaiveDate>) {
    if markets.is_empty()
        || markets.iter().any(|market| {
            calendars
                .get(market)
                .is_none_or(|calendar| !calendar.covers(authority_start, query.end_date))
        })
    {
        return (Vec::new(), None);
    }

    let all_sessions = markets
        .iter()
        .filter_map(|market| calendars.get(market))
        .flat_map(|calendar| calendar.sessions.iter().copied())
        .collect::<BTreeSet<_>>();
    let expected_baseline_date = all_sessions.range(..query.start_date).next_back().copied();
    let expected_actual_dates = all_sessions
        .range(query.start_date..=query.end_date)
        .copied()
        .collect::<Vec<_>>();

    // Without a known prior session there is no authoritative pre-period
    // baseline cutoff. An empty requested-session set likewise cannot define
    // a terminal valuation.
    if expected_baseline_date.is_none() || expected_actual_dates.is_empty() {
        return (Vec::new(), None);
    }
    (expected_actual_dates, expected_baseline_date)
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
    expected_baseline_date: Option<NaiveDate>,
) -> Result<
    (
        Option<PortfolioValuePoint>,
        Vec<PortfolioValuePoint>,
        MetricAvailability,
        bool,
    ),
    String,
> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    if query.account_id.is_none() && query.market.is_none() {
        let baseline = expected_baseline_date.and_then(|expected_date| {
            conn.query_row(
                "SELECT date, total_value, exchange_rates FROM daily_portfolio_values
                 WHERE date = ?1",
                params![expected_date.format("%Y-%m-%d").to_string()],
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
            })
        });
        let mut statement = conn
            .prepare(
                "SELECT date, total_value, exchange_rates FROM daily_portfolio_values
                 WHERE date BETWEEN ?1 AND ?2 ORDER BY date ASC",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
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
            .collect::<Vec<_>>();
        let values = rows
            .iter()
            .filter_map(|(date, value, rates_json)| {
                NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                    .ok()
                    .zip(convert_snapshot_value(
                        *value,
                        rates_json,
                        &query.base_currency,
                    ))
                    .map(|(date, value_base)| PortfolioValuePoint { date, value_base })
            })
            .collect::<Vec<_>>();
        let nav_complete = baseline.is_some() && !values.is_empty() && values.len() == rows.len();
        let availability = if nav_complete {
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
        return Ok((baseline, values, availability, nav_complete));
    }
    // Filtered daily snapshots do not contain account cash. Preserve their
    // stock value path for context, but do not claim an authoritative TWR.
    let mut statement = conn
        .prepare(
            "SELECT snapshots.date, snapshots.market, snapshots.market_value,
                    portfolio.exchange_rates
             FROM daily_holding_snapshots AS snapshots
             LEFT JOIN daily_portfolio_values AS portfolio
               ON portfolio.date = snapshots.date
             WHERE snapshots.date BETWEEN ?1 AND ?2
               AND (?3 IS NULL OR snapshots.account_id = ?3)
               AND (?4 IS NULL OR snapshots.market = ?4)
             ORDER BY snapshots.date ASC, snapshots.account_id ASC,
                      snapshots.market ASC, snapshots.symbol ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![
                query.start_date.format("%Y-%m-%d").to_string(),
                query.end_date.format("%Y-%m-%d").to_string(),
                query.account_id,
                query.market,
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, Option<String>>(3)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let mut daily_values = BTreeMap::<NaiveDate, Option<f64>>::new();
    for (date, market, value, rates_json) in rows {
        let Some(date) = NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok() else {
            continue;
        };
        let currency = market_currency(&market);
        let converted = if currency == query.base_currency {
            value.is_finite().then_some(value)
        } else {
            rates_json
                .as_deref()
                .and_then(|json| serde_json::from_str::<crate::models::ExchangeRates>(json).ok())
                .map(|rates| {
                    crate::services::exchange_rate_service::convert_currency(
                        value,
                        currency,
                        &query.base_currency,
                        &rates,
                    )
                })
                .filter(|converted| converted.is_finite())
        };
        let entry = daily_values.entry(date).or_insert(Some(0.0));
        *entry = match (*entry, converted) {
            (Some(total), Some(converted)) => Some(total + converted),
            _ => None,
        };
    }
    let conversion_incomplete = daily_values.values().any(Option::is_none);
    let values = if conversion_incomplete {
        Vec::new()
    } else {
        daily_values
            .into_iter()
            .filter_map(|(date, value_base)| {
                value_base.map(|value_base| PortfolioValuePoint { date, value_base })
            })
            .collect()
    };
    Ok((
        None,
        values,
        MetricAvailability {
            status: MetricStatus::Unavailable,
            note: Some(if conversion_incomplete {
                "Filtered snapshots lack exact daily FX for at least one local market value, as well as an authoritative daily cash ledger; actual TWR and aggregate NAV-dependent metrics are unavailable.".to_string()
            } else {
                "Filtered snapshots lack an authoritative daily cash ledger; actual TWR is unavailable.".to_string()
            }),
        },
        false,
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
    position_events: &[crate::services::stock_action_builder::PositionEvent],
    recorded_splits: &[RecordedSplit],
    split_market_authority: &SplitMarketAuthority,
    origin: NaiveDate,
) -> Result<Vec<OpeningPosition>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT account_id, symbol, market, currency, shares FROM holdings
             WHERE shares > 0 AND symbol NOT LIKE '$CASH-%'
               AND (?1 IS NULL OR account_id = ?1) AND (?2 IS NULL OR market = ?2)",
        )
        .map_err(|error| error.to_string())?;
    let legacy_positions = statement
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
    let mut quantities = BTreeMap::<
        (String, String, String),
        (f64, &crate::services::stock_action_builder::PositionEvent),
    >::new();
    for event in position_events
        .iter()
        .filter(|event| event.trade_date <= origin)
    {
        let split_factor = recorded_splits
            .iter()
            .filter(|split| {
                stock_symbols_equal(&split.symbol, &event.symbol)
                    && split_market_authority
                        .get(&normalized_stock_symbol(&split.symbol).unwrap_or_default())
                        .is_some_and(|markets| {
                            markets.len() == 1
                                && markets.contains(
                                    &normalized_stock_market(&event.market).unwrap_or_default(),
                                )
                        })
                    && split.date > event.trade_date
                    && split.date <= origin
            })
            .map(|split| split.ratio)
            .product::<f64>();
        let entry = quantities
            .entry((
                event.account_id.clone(),
                normalized_stock_symbol(&event.symbol).unwrap_or_default(),
                normalized_stock_market(&event.market).unwrap_or_default(),
            ))
            .or_insert((0.0, event));
        entry.0 += event.shares_delta * split_factor;
        entry.1 = event;
    }
    let mut positions = quantities
        .into_values()
        .filter(|(quantity, _)| *quantity > 0.0)
        .map(|(quantity, event)| OpeningPosition {
            account_id: event.account_id.clone(),
            symbol: event.symbol.clone(),
            market: event.market.clone(),
            currency: market_currency(&event.market).to_string(),
            quantity,
        })
        .collect::<Vec<_>>();
    // Current holdings are an opening fallback only for legacy positions with
    // no source-ledger events at all. A symbol bought during the report must
    // not be projected backward merely because it exists in today's holdings.
    let ledger_keys = position_events
        .iter()
        .map(|event| {
            (
                event.account_id.clone(),
                normalized_stock_symbol(&event.symbol).unwrap_or_default(),
                normalized_stock_market(&event.market).unwrap_or_default(),
            )
        })
        .collect::<BTreeSet<_>>();
    positions.extend(legacy_positions.into_iter().filter_map(|mut position| {
        if ledger_keys.contains(&(
            position.account_id.clone(),
            normalized_stock_symbol(&position.symbol).unwrap_or_default(),
            normalized_stock_market(&position.market).unwrap_or_default(),
        )) {
            return None;
        }
        let post_origin_factor = recorded_splits
            .iter()
            .filter(|split| {
                stock_symbols_equal(&split.symbol, &position.symbol)
                    && split_market_authority
                        .get(&normalized_stock_symbol(&split.symbol).unwrap_or_default())
                        .is_some_and(|markets| {
                            markets.len() == 1
                                && markets.contains(
                                    &normalized_stock_market(&position.market).unwrap_or_default(),
                                )
                        })
                    && split.date > origin
            })
            .map(|split| split.ratio)
            .product::<f64>();
        if !post_origin_factor.is_finite() || post_origin_factor <= 0.0 {
            return None;
        }
        position.quantity /= post_origin_factor;
        Some(position)
    }));
    Ok(positions)
}

fn opening_cash(
    db: &Database,
    transactions: &[CorrectedTransaction],
    query: &StockReviewQuery,
    origin: NaiveDate,
) -> Result<(Vec<OpeningCashBalance>, bool), String> {
    let mut ledger_balances = BTreeMap::<(String, String), f64>::new();
    let mut ledger_anchor_keys = BTreeSet::<(String, String)>::new();
    let mut required_keys = BTreeSet::<(String, String)>::new();
    for transaction in transactions
        .iter()
        .filter(|corrected| corrected.has_cash_effect)
        .map(|corrected| &corrected.transaction)
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

    let mut authoritative = BTreeMap::new();
    for (account_id, currency, current_amount) in current_cash {
        let later_delta = transactions
            .iter()
            .filter(|corrected| corrected.has_cash_effect)
            .map(|corrected| &corrected.transaction)
            .filter(|transaction| {
                transaction.account_id == account_id
                    && transaction.currency == currency
                    && transaction_date(transaction).is_some_and(|date| date > origin)
            })
            .map(|transaction| {
                crate::commands::transactions::cash_delta(
                    &transaction.transaction_type,
                    &transaction.symbol,
                    transaction.total_amount,
                    transaction.commission,
                )
            })
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
    transactions: &[CorrectedTransaction],
    query: &StockReviewQuery,
    origin: NaiveDate,
) -> (Vec<ExternalFlowBase>, bool) {
    let conn = match db.conn.lock() {
        Ok(conn) => conn,
        Err(_) => return (vec![], false),
    };
    let mut grouped = BTreeMap::new();
    let mut complete = true;
    for transaction in transactions
        .iter()
        .filter(|corrected| corrected.has_cash_effect)
        .map(|corrected| &corrected.transaction)
        .filter(|transaction| {
            transaction_date(transaction)
                .is_some_and(|date| date > origin && date <= query.end_date)
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
        })
    {
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
    transactions: &[CorrectedTransaction],
    origin: NaiveDate,
    end: NaiveDate,
) -> Vec<crate::services::shadow_portfolio_engine::ExternalFlowEvent> {
    transactions
        .iter()
        .filter(|corrected| corrected.has_cash_effect)
        .map(|corrected| &corrected.transaction)
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

fn load_recorded_splits(db: &Database, end: NaiveDate) -> Result<Vec<RecordedSplit>, String> {
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
    Ok(rows
        .into_iter()
        .filter_map(|(symbol, date, ratio_from, ratio_to)| {
            let date = NaiveDate::parse_from_str(&date, "%Y-%m-%d").ok()?;
            let ratio = ratio_to / ratio_from;
            (ratio.is_finite() && ratio > 0.0).then_some(RecordedSplit {
                symbol,
                date,
                ratio,
            })
        })
        .collect())
}

fn load_split_market_authority(
    db: &Database,
    transactions: &[Transaction],
) -> Result<SplitMarketAuthority, String> {
    let mut authority = SplitMarketAuthority::new();
    let mut record = |symbol: &str, market: &str| {
        let Some(symbol) = normalized_stock_symbol(symbol) else {
            return;
        };
        let Some(market) = normalized_stock_market(market) else {
            return;
        };
        authority.entry(symbol).or_default().insert(market);
    };
    for transaction in transactions
        .iter()
        .filter(|transaction| !crate::services::quote_service::is_cash_symbol(&transaction.symbol))
    {
        record(&transaction.symbol, &transaction.market);
    }
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT symbol, market FROM holdings
             WHERE shares != 0 AND symbol NOT LIKE '$CASH-%'",
        )
        .map_err(|error| error.to_string())?;
    let holdings = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    for (symbol, market) in holdings {
        record(&symbol, &market);
    }
    Ok(authority)
}

fn load_split_events(
    recorded_splits: &[RecordedSplit],
    opening_positions: &[OpeningPosition],
    position_events: &[crate::services::stock_action_builder::PositionEvent],
    split_market_authority: &SplitMarketAuthority,
    origin: NaiveDate,
    end: NaiveDate,
) -> Vec<SplitEvent> {
    let mut quantities = opening_positions
        .iter()
        .map(|position| {
            (
                (
                    position.account_id.clone(),
                    normalized_stock_symbol(&position.symbol).unwrap_or_default(),
                    normalized_stock_market(&position.market).unwrap_or_default(),
                ),
                (
                    position.quantity,
                    position.symbol.clone(),
                    position.market.clone(),
                ),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let mut applied_transaction_ids = BTreeSet::new();
    let mut events = Vec::new();
    for split in recorded_splits
        .iter()
        .filter(|split| split.date > origin && split.date <= end)
    {
        let symbol_key = normalized_stock_symbol(&split.symbol).unwrap_or_default();
        let Some(authoritative_market) = split_market_authority
            .get(&symbol_key)
            .filter(|markets| markets.len() == 1)
            .and_then(|markets| markets.first())
            .cloned()
        else {
            continue;
        };
        for event in position_events
            .iter()
            .filter(|event| event.trade_date > origin && event.trade_date < split.date)
            .filter(|event| applied_transaction_ids.insert(event.transaction_id.clone()))
        {
            let entry = quantities
                .entry((
                    event.account_id.clone(),
                    normalized_stock_symbol(&event.symbol).unwrap_or_default(),
                    normalized_stock_market(&event.market).unwrap_or_default(),
                ))
                .or_insert((0.0, event.symbol.clone(), event.market.clone()));
            entry.0 += event.shares_delta;
        }
        for ((account_id, _, market_key), (quantity, symbol, market)) in &mut quantities {
            if *quantity > 0.0
                && stock_symbols_equal(symbol, &split.symbol)
                && *market_key == authoritative_market
            {
                events.push(SplitEvent {
                    date: split.date,
                    account_id: account_id.clone(),
                    symbol: symbol.clone(),
                    market: market.clone(),
                    ratio: split.ratio,
                });
                *quantity *= split.ratio;
            }
        }
    }
    events
}

fn ambiguous_split_issues(
    recorded_splits: &[RecordedSplit],
    opening_positions: &[OpeningPosition],
    position_events: &[crate::services::stock_action_builder::PositionEvent],
    split_market_authority: &SplitMarketAuthority,
) -> Vec<StockReviewIssue> {
    let relevant_symbols = opening_positions
        .iter()
        .filter_map(|position| normalized_stock_symbol(&position.symbol))
        .chain(
            position_events
                .iter()
                .filter_map(|event| normalized_stock_symbol(&event.symbol)),
        )
        .collect::<BTreeSet<_>>();
    recorded_splits
        .iter()
        .filter(|split| {
            normalized_stock_symbol(&split.symbol).is_some_and(|symbol| {
                relevant_symbols.contains(&symbol)
                    && split_market_authority
                        .get(&symbol)
                .is_some_and(|markets| markets.len() > 1)
            })
        })
        .map(|split| StockReviewIssue {
            code: "split_market_ambiguous".to_string(),
            severity: StockReviewIssueSeverity::Error,
            message: "A legacy split record has no market and the same code exists in multiple markets; the split is not applied until its market authority is resolved."
                .to_string(),
            affected_symbol: Some(split.symbol.clone()),
            affected_date: Some(split.date),
        })
        .collect()
}

fn load_dividend_events(
    transactions: &[CorrectedTransaction],
    origin: NaiveDate,
    end: NaiveDate,
) -> Vec<DividendEvent> {
    transactions
        .iter()
        .filter(|corrected| corrected.has_cash_effect)
        .map(|corrected| &corrected.transaction)
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

fn shadow_total_return_field_complete(
    opening_positions: &[OpeningPosition],
    prices: &BTreeMap<(String, String), Vec<DailyMarketPoint>>,
    sessions_by_market: &BTreeMap<String, Vec<NaiveDate>>,
    origin: NaiveDate,
    end: NaiveDate,
    field_present: impl Fn(&DailyMarketPoint) -> bool,
) -> bool {
    !opening_positions.is_empty()
        && opening_positions.iter().all(|position| {
            let expected = sessions_by_market
                .get(&position.market)
                .into_iter()
                .flat_map(|sessions| sessions.iter())
                .filter(|date| **date >= origin && **date <= end)
                .copied()
                .collect::<Vec<_>>();
            !expected.is_empty()
                && prices
                    .get(&(position.symbol.clone(), position.market.clone()))
                    .is_some_and(|points| {
                        expected.iter().all(|date| {
                            points
                                .iter()
                                .find(|point| point.date == *date)
                                .is_some_and(|point| field_present(point))
                        })
                    })
        })
}

fn complete_shadow_dividend_events(
    opening_positions: &[OpeningPosition],
    prices: &BTreeMap<(String, String), Vec<DailyMarketPoint>>,
    sessions_by_market: &BTreeMap<String, Vec<NaiveDate>>,
    origin: NaiveDate,
    end: NaiveDate,
) -> Option<Vec<DividendEvent>> {
    if !shadow_total_return_field_complete(
        opening_positions,
        prices,
        sessions_by_market,
        origin,
        end,
        |point| point.dividend.is_some(),
    ) {
        return None;
    }
    Some(
        opening_positions
            .iter()
            .flat_map(|position| {
                prices
                    .get(&(position.symbol.clone(), position.market.clone()))
                    .into_iter()
                    .flat_map(|points| points.iter())
                    .filter(|point| point.date > origin && point.date <= end)
                    .filter_map(|point| {
                        point
                            .dividend
                            .filter(|amount| amount.is_finite() && *amount > 0.0)
                            .map(|amount_per_share| DividendEvent {
                                date: point.date,
                                account_id: position.account_id.clone(),
                                symbol: position.symbol.clone(),
                                market: position.market.clone(),
                                currency: position.currency.clone(),
                                amount_per_share,
                            })
                    })
            })
            .collect(),
    )
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

fn exact_fx_on(
    currency: &str,
    base_currency: &str,
    points: &[crate::services::shadow_portfolio_engine::ShadowFxPoint],
    date: NaiveDate,
) -> Option<f64> {
    if currency == base_currency {
        return Some(1.0);
    }
    points.iter().find_map(|point| {
        (point.currency == currency && point.base_currency == base_currency && point.date == date)
            .then_some(point.rate)
    })
}

fn campaign_cash_flows(
    transactions: &[CorrectedTransaction],
    symbol: &str,
    market: &str,
    account_ids: &[String],
    start: NaiveDate,
    end: NaiveDate,
    query: &StockReviewQuery,
    fx_points: &[crate::services::shadow_portfolio_engine::ShadowFxPoint],
) -> Vec<CampaignCashFlow> {
    let mut flows = Vec::new();
    for corrected in transactions
        .iter()
        .filter(|corrected| !corrected.is_transfer)
    {
        let transaction = &corrected.transaction;
        if !(stock_symbols_equal(&transaction.symbol, symbol)
            && transaction.market == market
            && account_ids.contains(&transaction.account_id)
            && transaction_date(transaction).is_some_and(|date| date >= start && date <= end))
        {
            continue;
        }
        let Some(date) = transaction_date(transaction) else {
            continue;
        };
        let fx = exact_fx_on(&transaction.currency, &query.base_currency, fx_points, date);
        let action_id = corrected.action_id.clone();
        match transaction.transaction_type.as_str() {
            "BUY" => flows.push(CampaignTimelineItem {
                date,
                kind: CampaignCashFlowKind::Buy,
                amount_base: fx.map(|fx| transaction.total_amount * fx),
                amount_local: transaction.total_amount,
                currency: transaction.currency.clone(),
                shares: transaction.shares,
                account_id: transaction.account_id.clone(),
                action_id: action_id.clone(),
            }),
            "SELL" => flows.push(CampaignTimelineItem {
                date,
                kind: CampaignCashFlowKind::Sell,
                amount_base: fx.map(|fx| transaction.total_amount * fx),
                amount_local: transaction.total_amount,
                currency: transaction.currency.clone(),
                shares: transaction.shares,
                account_id: transaction.account_id.clone(),
                action_id: action_id.clone(),
            }),
            "PAY" => flows.push(CampaignTimelineItem {
                date,
                kind: CampaignCashFlowKind::Dividend,
                amount_base: fx.map(|fx| transaction.total_amount * fx),
                amount_local: transaction.total_amount,
                currency: transaction.currency.clone(),
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
                amount_base: fx.map(|fx| transaction.commission * fx),
                amount_local: transaction.commission,
                currency: transaction.currency.clone(),
                shares: 0.0,
                account_id: transaction.account_id.clone(),
                action_id,
            });
        }
    }
    flows.sort_by_key(|flow| flow.date);
    flows
}

fn campaign_position_events(
    source_events: &[crate::services::stock_action_builder::PositionEvent],
    split_events: &[SplitEvent],
    campaign: &StockCampaignSummary,
    selected_account: Option<&str>,
) -> Vec<CampaignPositionEvent> {
    #[derive(Clone)]
    enum RawPositionEvent<'a> {
        Source(&'a crate::services::stock_action_builder::PositionEvent),
        Split(&'a SplitEvent),
    }

    let visible_fragments = campaign
        .fragments
        .iter()
        .filter(|fragment| selected_account.is_none_or(|id| fragment.account_id == id))
        .collect::<Vec<_>>();
    let belongs_to_visible_fragment = |account_id: &str, date: NaiveDate| {
        visible_fragments.iter().any(|fragment| {
            fragment.account_id == account_id
                && fragment
                    .started_at
                    .get(..10)
                    .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
                    .is_some_and(|start| start <= date)
                && fragment
                    .ended_at
                    .as_deref()
                    .and_then(|value| value.get(..10))
                    .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
                    .is_none_or(|end| date <= end)
        })
    };
    let mut raw = source_events
        .iter()
        .filter(|event| {
            stock_securities_equal(
                &event.symbol,
                &event.market,
                &campaign.symbol,
                &campaign.market,
            ) && belongs_to_visible_fragment(&event.account_id, event.trade_date)
        })
        .map(RawPositionEvent::Source)
        .chain(
            split_events
                .iter()
                .filter(|split| {
                    stock_securities_equal(
                        &split.symbol,
                        &split.market,
                        &campaign.symbol,
                        &campaign.market,
                    ) && selected_account.is_none_or(|id| split.account_id == id)
                })
                .filter(|split| {
                    visible_fragments.iter().any(|fragment| {
                        fragment
                            .started_at
                            .get(..10)
                            .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
                            .is_some_and(|start| start <= split.date)
                            && fragment
                                .ended_at
                                .as_deref()
                                .and_then(|value| value.get(..10))
                                .and_then(|value| NaiveDate::parse_from_str(value, "%Y-%m-%d").ok())
                                .is_none_or(|end| split.date <= end)
                    })
                })
                .map(RawPositionEvent::Split),
        )
        .collect::<Vec<_>>();
    raw.sort_by(|left, right| {
        let key = |event: &RawPositionEvent<'_>| match event {
            // Splits are applied before stock flows on the effective date, the
            // same convention as the shadow and attribution engines.
            RawPositionEvent::Split(split) => (split.date, 0_u8, String::new()),
            RawPositionEvent::Source(event) => (event.trade_date, 1_u8, event.traded_at.clone()),
        };
        key(left).cmp(&key(right))
    });

    let mut quantities = BTreeMap::<String, f64>::new();
    let mut output = Vec::new();
    for raw_event in raw {
        match raw_event {
            RawPositionEvent::Source(event) => {
                let quantity = quantities.entry(event.account_id.clone()).or_default();
                *quantity += event.shares_delta;
                output.push(CampaignPositionEvent {
                    date: event.trade_date,
                    sequence: output.len(),
                    account_id: event.account_id.clone(),
                    kind: if event.transaction_type == "OPEN" {
                        CampaignPositionEventKind::Opening
                    } else if event.is_transfer {
                        CampaignPositionEventKind::Transfer
                    } else {
                        CampaignPositionEventKind::Trade
                    },
                    quantity_delta: event.shares_delta,
                    cost_basis_known: event.transaction_type != "OPEN"
                        && (!event.is_transfer || selected_account.is_none()),
                });
            }
            RawPositionEvent::Split(split) => {
                let quantity = quantities.entry(split.account_id.clone()).or_default();
                if *quantity <= 0.0 {
                    continue;
                }
                let delta = *quantity * (split.ratio - 1.0);
                *quantity += delta;
                output.push(CampaignPositionEvent {
                    date: split.date,
                    sequence: output.len(),
                    account_id: split.account_id.clone(),
                    kind: CampaignPositionEventKind::Split,
                    quantity_delta: delta,
                    cost_basis_known: true,
                });
            }
        }
    }
    output
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
            .zip(exact_fx_on(
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
    transactions: &[CorrectedTransaction],
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
        for corrected in transactions.iter().filter(|corrected| {
            transaction_date(&corrected.transaction)
                .is_some_and(|trade_date| trade_date > origin && trade_date <= *date)
        }) {
            let transaction = &corrected.transaction;
            let balance = balances
                .entry((transaction.account_id.clone(), transaction.currency.clone()))
                .or_default();
            if !corrected.has_cash_effect {
                continue;
            }
            let delta = crate::commands::transactions::cash_delta(
                &transaction.transaction_type,
                &transaction.symbol,
                transaction.total_amount,
                transaction.commission,
            );
            *balance += delta;
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
) -> Result<(RiskStructureInput, bool), String> {
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
        let Some(fx) = exact_fx_on(
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
            market,
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
    let mut action_fx_complete = true;
    for action in actions.iter().filter(|action| {
        action_date(action).is_some_and(|date| date >= query.start_date && date <= query.end_date)
            && !action.fact_labels.iter().any(|label| label == "transfer")
    }) {
        let date = action_date(action).unwrap();
        let currency = action
            .currency
            .as_deref()
            .unwrap_or(market_currency(&action.market));
        let fx = exact_fx_on(currency, &query.base_currency, fx_points, date);
        if let Some(fx) = fx {
            total_fees += action.fees.unwrap_or(0.0) * fx;
            if let Some(notional) = action.gross_amount {
                changes.push(StockChangeBase::trade(notional * fx));
            }
        } else {
            action_fx_complete = false;
        }
    }
    Ok((
        RiskStructureInput {
            snapshots,
            stock_changes: changes,
            total_stock_trading_fees_base: action_fx_complete.then_some(total_fees),
            average_portfolio_nav_base: action_fx_complete.then_some(average_nav).flatten(),
        },
        action_fx_complete,
    ))
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
    annotations.retain(|annotation| annotation_visible_as_of(annotation, query.end_date));
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn.prepare(
        "SELECT qh.id, qh.account_id, qh.symbol, qh.notes, qh.decision_quality, qs.snapshot_date
         FROM quarterly_holding_snapshots qh JOIN quarterly_snapshots qs ON qs.id = qh.quarterly_snapshot_id
         WHERE (?1 IS NULL OR qh.account_id = ?1) AND (?2 IS NULL OR qh.market = ?2)
           AND qs.snapshot_date <= ?3
         ORDER BY qs.snapshot_date ASC, qh.id ASC"
    ).map_err(|error| error.to_string())?;
    let historical = statement
        .query_map(
            params![
                query.account_id,
                query.market,
                query.end_date.format("%Y-%m-%d").to_string()
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, String>(5)?,
                ))
            },
        )
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
                .filter(|date| exact_fx_on(currency, base_currency, fx_points, **date).is_some())
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

    fn stock_event(
        id: &str,
        account: &str,
        symbol: &str,
        market: &str,
        transaction_type: &str,
        shares: f64,
        before: f64,
        after: f64,
        traded_at: &str,
    ) -> crate::services::stock_action_builder::PositionEvent {
        crate::services::stock_action_builder::PositionEvent {
            transaction_id: id.to_string(),
            account_id: account.to_string(),
            symbol: symbol.to_string(),
            market: market.to_string(),
            transaction_type: transaction_type.to_string(),
            traded_at: traded_at.to_string(),
            trade_date: day(&traded_at[..10]),
            shares_delta: shares,
            shares_before: before,
            shares_after: after,
            is_date_precision: false,
            is_transfer: false,
        }
    }

    #[test]
    fn opening_and_legacy_fallback_identity_include_market() {
        // Collapsing (account, code, market) to (account, code) either merges
        // these openings or lets a US ledger row suppress the CN fallback.
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_holding(&db, "cn-holding", "acct", "000001", "CN", "CNY", 20.0);
        let query = live_query("2024-01-10", "2024-01-11", None);
        let events = vec![stock_event(
            "us-open",
            "acct",
            "000001",
            "US",
            "OPEN",
            10.0,
            0.0,
            10.0,
            "2024-01-01T09:30:00Z",
        )];

        let positions = opening_positions(
            &db,
            &query,
            &events,
            &[],
            &BTreeMap::new(),
            day("2024-01-09"),
        )
        .unwrap();
        assert_eq!(positions.len(), 2);
        assert!(positions
            .iter()
            .any(|position| position.market == "US" && position.quantity == 10.0));
        assert!(positions
            .iter()
            .any(|position| position.market == "CN" && position.quantity == 20.0));

        let price = |close: f64| DailyMarketPoint {
            date: day("2024-01-09"),
            open: Some(close),
            high: Some(close),
            low: Some(close),
            close,
            volume: Some(1.0),
            adjusted_close: None,
            dividend: None,
        };
        let prices = BTreeMap::from([
            (("000001".to_string(), "US".to_string()), vec![price(10.0)]),
            (("000001".to_string(), "CN".to_string()), vec![price(5.0)]),
        ]);
        let fx = vec![ShadowFxPoint {
            date: day("2024-01-09"),
            currency: "CNY".to_string(),
            base_currency: "USD".to_string(),
            rate: 0.2,
        }];
        let opening_values =
            opening_market_values(&positions, &prices, &fx, "USD", day("2024-01-09"));
        assert!(opening_values
            .iter()
            .any(|value| value.market == "US" && value.value_base == 100.0));
        assert!(opening_values
            .iter()
            .any(|value| value.market == "CN" && value.value_base == 20.0));

        let fixed = calculate_result_quality(&ResultQualityInput {
            actual_origin_date: day("2024-01-09"),
            actual_values: vec![PortfolioValuePoint {
                date: day("2024-01-10"),
                value_base: 120.0,
            }],
            baseline: Some(PortfolioValuePoint {
                date: day("2024-01-09"),
                value_base: 120.0,
            }),
            expected_actual_dates: vec![day("2024-01-10")],
            expected_baseline_date: Some(day("2024-01-09")),
            external_flows_base: vec![],
            actual_availability: available(),
            opening_market_values_base: opening_values,
            opening_cash_value_base: 0.0,
            benchmark_series: vec![
                BenchmarkSeriesInput {
                    market: "US".to_string(),
                    availability: available(),
                    points: vec![
                        BenchmarkPoint {
                            date: day("2024-01-09"),
                            value: 100.0,
                        },
                        BenchmarkPoint {
                            date: day("2024-01-10"),
                            value: 100.0,
                        },
                    ],
                },
                BenchmarkSeriesInput {
                    market: "CN".to_string(),
                    availability: available(),
                    points: vec![
                        BenchmarkPoint {
                            date: day("2024-01-09"),
                            value: 100.0,
                        },
                        BenchmarkPoint {
                            date: day("2024-01-10"),
                            value: 100.0,
                        },
                    ],
                },
            ],
            benchmark_selection: BenchmarkSelection::AutomaticMixed,
            shadow_curve: vec![],
        });
        assert!((fixed.fixed_weights["US"] - 5.0 / 6.0).abs() < 1e-12);
        assert!((fixed.fixed_weights["CN"] - 1.0 / 6.0).abs() < 1e-12);

        let shadow = build_shadow_series(&ShadowPortfolioInput {
            base_currency: "USD".to_string(),
            return_method: ShadowReturnMethod::PriceOnly,
            opening_positions: positions,
            opening_cash: vec![],
            valuation_dates: vec![day("2024-01-09"), day("2024-01-10")],
            price_points: vec![
                ShadowPricePoint {
                    date: day("2024-01-09"),
                    symbol: "000001".to_string(),
                    market: "US".to_string(),
                    currency: "USD".to_string(),
                    close: 10.0,
                    adjusted_close: None,
                },
                ShadowPricePoint {
                    date: day("2024-01-10"),
                    symbol: "000001".to_string(),
                    market: "US".to_string(),
                    currency: "USD".to_string(),
                    close: 20.0,
                    adjusted_close: None,
                },
                ShadowPricePoint {
                    date: day("2024-01-09"),
                    symbol: "000001".to_string(),
                    market: "CN".to_string(),
                    currency: "CNY".to_string(),
                    close: 5.0,
                    adjusted_close: None,
                },
                ShadowPricePoint {
                    date: day("2024-01-10"),
                    symbol: "000001".to_string(),
                    market: "CN".to_string(),
                    currency: "CNY".to_string(),
                    close: 5.0,
                    adjusted_close: None,
                },
            ],
            fx_points: vec![
                fx[0].clone(),
                ShadowFxPoint {
                    date: day("2024-01-10"),
                    ..fx[0].clone()
                },
            ],
            external_flows: vec![],
            cash_income_events: vec![],
            dividend_events: vec![],
            split_events: vec![],
        });
        assert_eq!(shadow.ending_value, Some(220.0));
    }

    #[test]
    fn marketless_split_is_skipped_when_equal_code_has_multiple_market_authorities() {
        // The legacy stock_splits row has no market. Applying it to both
        // listings is fabricated authority; ambiguity must degrade safely.
        let openings = vec![
            OpeningPosition {
                account_id: "acct".to_string(),
                symbol: "000001".to_string(),
                market: "CN".to_string(),
                currency: "CNY".to_string(),
                quantity: 10.0,
            },
            OpeningPosition {
                account_id: "acct".to_string(),
                symbol: "000001".to_string(),
                market: "HK".to_string(),
                currency: "HKD".to_string(),
                quantity: 10.0,
            },
        ];
        let split = RecordedSplit {
            symbol: "000001".to_string(),
            date: day("2024-01-10"),
            ratio: 2.0,
        };
        let ambiguous_authority = BTreeMap::from([(
            "000001".to_string(),
            BTreeSet::from(["CN".to_string(), "HK".to_string()]),
        )]);

        let ambiguous = load_split_events(
            std::slice::from_ref(&split),
            &openings[..1],
            &[],
            &ambiguous_authority,
            day("2024-01-09"),
            day("2024-01-11"),
        );
        assert!(ambiguous.is_empty());
        let issues = ambiguous_split_issues(
            std::slice::from_ref(&split),
            &openings[..1],
            &[],
            &ambiguous_authority,
        );
        assert_eq!(issues.len(), 1);
        assert_eq!(issues[0].code, "split_market_ambiguous");
        assert_eq!(issues[0].affected_symbol.as_deref(), Some("000001"));

        let authoritative = load_split_events(
            &[split],
            &openings[..1],
            &[],
            &BTreeMap::from([("000001".to_string(), BTreeSet::from(["CN".to_string()]))]),
            day("2024-01-09"),
            day("2024-01-11"),
        );
        assert_eq!(authoritative.len(), 1);
        assert_eq!(authoritative[0].market, "CN");
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
        for (market, symbol) in [("US", "^GSPC"), ("CN", "000300.SS"), ("HK", "^HSI")] {
            seed_benchmark_cache(db, symbol, start, None, 100.0);
            install_market_sessions(
                db,
                market,
                &calendar_dates(
                    start,
                    (Utc::now().date_naive() - start).num_days().max(0) as usize + 1,
                ),
            );
        }
    }

    fn calendar_dates(start: NaiveDate, count: usize) -> Vec<NaiveDate> {
        (0..count)
            .map(|offset| start + Duration::days(offset as i64))
            .collect()
    }

    fn install_market_sessions(db: &Database, market: &str, dates: &[NaiveDate]) {
        let Some(first) = dates.iter().min().copied() else {
            return;
        };
        let last = dates.iter().max().copied().unwrap();
        let open_dates = dates.iter().copied().collect::<BTreeSet<_>>();
        let conn = db.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS stock_market_sessions (
                market TEXT NOT NULL,
                date TEXT NOT NULL,
                is_session INTEGER NOT NULL DEFAULT 1,
                source TEXT NOT NULL,
                updated_at TEXT NOT NULL,
                PRIMARY KEY (market, date)
            );
            CREATE TABLE IF NOT EXISTS stock_market_calendar_coverage (
                market TEXT PRIMARY KEY,
                source TEXT NOT NULL,
                complete_start TEXT NOT NULL,
                complete_through TEXT NOT NULL,
                revision TEXT NOT NULL,
                encodes_closed_dates INTEGER NOT NULL,
                updated_at TEXT NOT NULL
            );",
        )
        .unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO stock_market_calendar_coverage
                (market, source, complete_start, complete_through, revision, encodes_closed_dates, updated_at)
             VALUES (?1, 'fixture_exchange_calendar', ?2, ?3, 'fixture-v1', 1, '2024-01-01T00:00:00Z')",
            params![market, first.format("%Y-%m-%d").to_string(), last.format("%Y-%m-%d").to_string()],
        ).unwrap();
        let mut date = first;
        while date <= last {
            conn.execute(
                "INSERT OR REPLACE INTO stock_market_sessions (market, date, is_session, source, updated_at)
                 VALUES (?1, ?2, ?3, 'fixture_exchange_calendar', '2024-01-01T00:00:00Z')",
                params![market, date.format("%Y-%m-%d").to_string(), i64::from(open_dates.contains(&date))],
            )
            .unwrap();
            date += Duration::days(1);
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
                expected_actual_dates: vec![day("2024-01-01"), day("2024-01-02")],
                expected_baseline_date: Some(day("2023-12-31")),
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
                            market: "US".to_string(),
                            value_base: 100.0,
                        }],
                        cash_value_base: Some(900.0),
                        reliable: true,
                    },
                    RiskSnapshotInput {
                        date: day("2024-01-02"),
                        stock_values_base: vec![StockValueBase {
                            symbol: "AAPL".to_string(),
                            market: "US".to_string(),
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
                    amount_base: Some(100.0),
                    amount_local: 100.0,
                    currency: "USD".to_string(),
                    shares: 1.0,
                    account_id: "acct".to_string(),
                    action_id: Some(action_id.to_string()),
                }],
                position_events: vec![CampaignPositionEvent {
                    date: day("2024-01-01"),
                    sequence: 0,
                    account_id: "acct".to_string(),
                    kind: CampaignPositionEventKind::Trade,
                    quantity_delta: 1.0,
                    cost_basis_known: true,
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
                amount_base: Some(100.0),
                amount_local: 100.0,
                currency: "USD".to_string(),
                shares: 1.0,
                account_id: "acct".to_string(),
                action_id: Some(action_id.to_string()),
            });
        input.campaign_data[0]
            .position_events
            .push(CampaignPositionEvent {
                date: day("2024-01-01"),
                sequence: 1,
                account_id: "acct".to_string(),
                kind: CampaignPositionEventKind::Trade,
                quantity_delta: -1.0,
                cost_basis_known: true,
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

    #[test]
    fn account_filtered_transfer_derives_with_both_legs_then_projects_only_local_fragment() {
        // Filtering before transfer derivation drops the referenced opposite
        // leg, while failing to project afterwards leaks the other account.
        for (selected, expect_in, expect_out) in
            [("source", false, true), ("destination", true, false)]
        {
            let mut input = transfer_fixture();
            input.query.account_id = Some(selected.to_string());
            let report = build_stock_review_report_from_cached_data(&input).unwrap();

            assert!(report.actions.is_empty());
            assert_eq!(report.campaigns.len(), 1);
            let campaign = &report.campaigns[0];
            assert_eq!(campaign.account_ids, vec![selected.to_string()]);
            assert_eq!(campaign.fragments.len(), 1);
            assert_eq!(campaign.fragments[0].account_id, selected);
            assert_eq!(
                campaign.fragments[0].transfer_in.is_some(),
                expect_in,
                "selected={selected}, issues={:?}",
                report
                    .data_quality
                    .issues
                    .iter()
                    .map(|issue| issue.code.as_str())
                    .collect::<Vec<_>>()
            );
            assert_eq!(
                campaign.fragments[0].transfer_out.is_some(),
                expect_out,
                "selected={selected}"
            );
            assert!(report
                .data_quality
                .issues
                .iter()
                .all(|issue| issue.code != "invalid_transfer_override"));
        }

        let mut invalid = transfer_fixture();
        invalid.query.account_id = Some("source".to_string());
        invalid.overrides[0].transaction_ids_json = r#"["transfer-out","missing"]"#.to_string();
        let report = build_stock_review_report_from_cached_data(&invalid).unwrap();
        assert!(report.actions.is_empty());
        assert_eq!(report.campaigns.len(), 1);
        assert_eq!(report.campaigns[0].account_ids, vec!["source".to_string()]);
        assert!(report.campaigns[0].fragments[0].transfer_out.is_none());
        assert!(report
            .data_quality
            .issues
            .iter()
            .any(|issue| issue.code == "invalid_transfer_override"));
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
        let loaded = load_all_transactions_for_review(&db).unwrap();
        let corrected = build_stock_actions(&loaded, &[]).corrected_transactions;
        let (cash, complete) = opening_cash(&db, &corrected, &query, day("2024-01-09")).unwrap();
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
        assert_eq!(report.summary.result_quality.excess_return, None);
        assert_eq!(report.summary.result_quality.active_return, None);
        assert_eq!(report.summary.rebalance_value_add.value_add, None);
        assert_eq!(report.summary.rebalance_value_add.actual_return, None);
        assert_eq!(report.summary.rebalance_value_add.shadow_return, None);
        assert_eq!(
            report
                .summary
                .rebalance_value_add
                .ending_value_difference_base,
            None
        );
        assert!(report
            .summary
            .rebalance_value_add
            .availability
            .note
            .as_deref()
            .is_some_and(|note| note.contains("opening cash")));
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
            live_query("2024-01-09", "2024-01-11", Some("US")),
        )
        .await
        .unwrap();
        assert_eq!(
            load_recorded_splits(&db, day("2024-01-11")).unwrap().len(),
            1
        );
        assert_eq!(prepared.shadow_input.opening_positions[0].quantity, 10.0);
        assert_eq!(prepared.shadow_input.split_events.len(), 1);
        let campaign = prepared.campaign_data.first().unwrap();
        assert_eq!(campaign.position_events.len(), 2);
        assert_eq!(
            campaign.position_events[0].kind,
            CampaignPositionEventKind::Opening
        );
        assert_eq!(campaign.position_events[0].quantity_delta, 10.0);
        assert_eq!(
            campaign.position_events[1].kind,
            CampaignPositionEventKind::Split
        );
        assert_eq!(campaign.position_events[1].quantity_delta, 10.0);
        let detail =
            build_stock_campaign_detail_from_cached_data(&prepared, &campaign.campaign_id).unwrap();
        assert_eq!(detail.pnl.remaining_shares, 20.0);
        assert_eq!(detail.pnl.total_pnl_base, None);
        assert_eq!(detail.campaign_return, None);
        let shadow = build_shadow_series(&prepared.shadow_input);
        assert_eq!(shadow.ending_value, Some(1_000.0));
        assert_eq!(
            shadow.twr_return_series.last().unwrap().cumulative_return,
            0.0
        );
    }

    #[tokio::test]
    async fn live_current_holding_with_in_period_buy_is_not_an_opening_position() {
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
            "2024-01-10T09:30:00Z",
        );
        insert_holding(&db, "stock", "acct", "AAPL", "US", "USD", 1.0);
        for date in ["2024-01-09", "2024-01-10", "2024-01-11"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
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

        let prepared =
            prepare_cached_stock_review_input(&db, live_query("2024-01-10", "2024-01-11", None))
                .await
                .unwrap();
        assert!(prepared.shadow_input.opening_positions.is_empty());
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
    async fn filtered_mixed_currency_snapshots_convert_each_row_before_nav_aggregation() {
        // Summing 100 USD and 700 CNY as 800 base units would corrupt average
        // NAV, turnover, and fee drag. Exact-date 7 CNY/USD makes the literal
        // base NAV 200 USD.
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_holding(&db, "us", "acct", "AAPL", "US", "USD", 1.0);
        insert_holding(&db, "cn", "acct", "600000", "CN", "CNY", 7.0);
        insert_holding(&db, "cash", "acct", "$CASH-USD", "US", "USD", 0.0);
        for date in ["2024-01-09", "2024-01-10", "2024-01-11"] {
            insert_portfolio_value(&db, date, 200.0, 7.0);
            insert_holding_snapshot(&db, date, "acct", "AAPL", "US", 1.0, 100.0, 100.0);
            insert_holding_snapshot(&db, date, "acct", "600000", "CN", 7.0, 100.0, 700.0);
        }
        let cache_start = day("2023-12-01");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[(day("2024-01-09"), 100.0), (day("2024-01-11"), 100.0)],
        );
        seed_stock_cache_bounds(
            &db,
            "600000",
            "CN",
            cache_start,
            &[(day("2024-01-09"), 100.0), (day("2024-01-11"), 100.0)],
        );
        seed_default_benchmarks(&db, cache_start);
        let mut query = live_query("2024-01-10", "2024-01-11", None);
        query.account_id = Some("acct".to_string());

        let prepared = prepare_cached_stock_review_input(&db, query).await.unwrap();
        assert_eq!(prepared.result_quality_input.actual_values.len(), 2);
        assert!(prepared
            .result_quality_input
            .actual_values
            .iter()
            .all(|point| (point.value_base - 200.0).abs() < 1e-12));
        assert_eq!(prepared.attribution_input.average_portfolio_nav, None);
        assert!(prepared
            .preparation_issues
            .iter()
            .any(|issue| issue.code == "filtered_nav_cash_unavailable"));
        assert!(prepared.shadow_input.fx_points.iter().any(|point| {
            point.currency == "CNY"
                && point.date == day("2024-01-10")
                && (point.rate - 1.0 / 7.0).abs() < 1e-12
        }));
    }

    #[tokio::test]
    async fn filtered_snapshot_nav_is_suppressed_when_any_exact_daily_fx_is_missing() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "CN");
        insert_holding(&db, "cn", "acct", "600000", "CN", "CNY", 1.0);
        insert_holding_snapshot(&db, "2024-01-10", "acct", "600000", "CN", 1.0, 700.0, 700.0);
        let cache_start = day("2023-12-01");
        seed_stock_cache_bounds(
            &db,
            "600000",
            "CN",
            cache_start,
            &[(day("2024-01-10"), 700.0)],
        );
        seed_default_benchmarks(&db, cache_start);
        let mut query = live_query("2024-01-10", "2024-01-10", None);
        query.account_id = Some("acct".to_string());

        let prepared = prepare_cached_stock_review_input(&db, query).await.unwrap();
        assert!(prepared.result_quality_input.actual_values.is_empty());
        assert_eq!(prepared.attribution_input.average_portfolio_nav, None);
        assert!(prepared
            .preparation_issues
            .iter()
            .any(|issue| issue.code == "snapshot_fx_unavailable"));
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
        assert!(prepared.shadow_input.dividend_events.is_empty());
        assert_eq!(
            prepared.shadow_input.return_method,
            ShadowReturnMethod::PriceOnly
        );
        assert!(prepared.attribution_input.dividends.is_empty());
        assert!(!prepared.attribution_input.valuations.is_empty());
        assert_eq!(prepared.attribution_input.batches.len(), 1);
        assert_eq!(prepared.attribution_input.cash_returns.len(), 1);
        assert_eq!(prepared.attribution_input.cash_returns[0].return_rate, 0.0);
        assert!(prepared.attribution_input.fx_rates.iter().any(|fx| {
            fx.date == day("2024-01-11") && fx.currency == "CNY" && (fx.rate - 0.15).abs() < 1e-12
        }));
        let actual_cash = prepared
            .attribution_input
            .valuations
            .iter()
            .map(|valuation| valuation.cash_balances[0].actual_amount)
            .collect::<Vec<_>>();
        assert_eq!(actual_cash, vec![900.0, 910.0]);
        let report = build_stock_review_report_from_cached_data(&prepared).unwrap();
        assert_eq!(
            report.attribution.availability.status,
            MetricStatus::Unavailable
        );
        assert!(prepared
            .preparation_issues
            .iter()
            .any(|issue| issue.code == "shadow_dividend_source_incomplete"));
        assert!(report.summary.result_quality.shadow_return.unwrap() > 0.0);
    }

    #[tokio::test]
    async fn account_pay_row_does_not_certify_complete_shadow_dividend_history() {
        // The actual account sold before PAY, while the shadow still owns the
        // stock. One account cash row cannot certify the shadow's per-share
        // corporate-action history for every holding and date.
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
            "open",
            "acct",
            "AAPL",
            "US",
            "OPEN",
            10.0,
            100.0,
            1_000.0,
            0.0,
            "USD",
            "2023-12-01T09:00:00Z",
        );
        insert_live_transaction(
            &db,
            "sell",
            "acct",
            "AAPL",
            "US",
            "SELL",
            10.0,
            100.0,
            1_000.0,
            0.0,
            "USD",
            "2024-01-10T09:30:00Z",
        );
        insert_live_transaction(
            &db,
            "pay",
            "acct",
            "AAPL",
            "US",
            "PAY",
            10.0,
            1.0,
            10.0,
            0.0,
            "USD",
            "2024-01-11T09:30:00Z",
        );
        for date in ["2024-01-09", "2024-01-10", "2024-01-11"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
        }
        let cache_start = day("2023-12-01");
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

        let prepared =
            prepare_cached_stock_review_input(&db, live_query("2024-01-10", "2024-01-11", None))
                .await
                .unwrap();
        assert_eq!(
            prepared.shadow_input.return_method,
            ShadowReturnMethod::PriceOnly
        );
        assert!(prepared.shadow_input.dividend_events.is_empty());
        assert!(prepared
            .preparation_issues
            .iter()
            .any(|issue| issue.code == "shadow_dividend_source_incomplete"));
    }

    #[tokio::test]
    async fn pre_origin_split_adjusts_opening_quantity_once_and_is_not_replayed_again() {
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
            "2023-12-01T08:00:00Z",
        );
        insert_live_transaction(
            &db,
            "open",
            "acct",
            "AAPL",
            "US",
            "OPEN",
            10.0,
            100.0,
            1_000.0,
            0.0,
            "USD",
            "2023-12-01T09:00:00Z",
        );
        db.conn.lock().unwrap().execute(
            "INSERT INTO stock_splits (stock_code, split_date, ratio_from, ratio_to, created_at)
             VALUES ('AAPL', '2024-01-05', 1, 2, '2024-01-01')",
            [],
        ).unwrap();
        for date in ["2024-01-09", "2024-01-10"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
        }
        let cache_start = day("2023-12-01");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[(day("2024-01-09"), 50.0), (day("2024-01-10"), 50.0)],
        );
        seed_default_benchmarks(&db, cache_start);

        let prepared = prepare_cached_stock_review_input(
            &db,
            live_query("2024-01-10", "2024-01-10", Some("US")),
        )
        .await
        .unwrap();
        assert_eq!(prepared.shadow_input.opening_positions[0].quantity, 20.0);
        assert!(prepared
            .shadow_input
            .split_events
            .iter()
            .all(|event| event.date > day("2024-01-09")));
        assert_eq!(
            build_shadow_series(&prepared.shadow_input).ending_value,
            Some(1_000.0)
        );
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
        install_market_sessions(&db, "US", &calendar_dates(day("2024-01-01"), 6));

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
    async fn campaign_grouped_flows_reference_real_actions_and_annotations_do_not_cross_accounts() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "a", "US");
        insert_account(&db, "b", "US");
        for account in ["a", "b"] {
            insert_live_transaction(
                &db,
                &format!("cash-{account}"),
                account,
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
        }
        insert_live_transaction(
            &db,
            "a-fill-1",
            "a",
            "AAPL",
            "US",
            "BUY",
            0.6,
            100.0,
            60.0,
            0.0,
            "USD",
            "2024-01-02T09:30:00Z",
        );
        insert_live_transaction(
            &db,
            "a-fill-2",
            "a",
            "AAPL",
            "US",
            "BUY",
            0.4,
            100.0,
            40.0,
            0.0,
            "USD",
            "2024-01-02T10:00:00Z",
        );
        insert_live_transaction(
            &db,
            "b-buy",
            "b",
            "AAPL",
            "US",
            "BUY",
            1.0,
            100.0,
            100.0,
            0.0,
            "USD",
            "2024-01-02T11:00:00Z",
        );
        save_user_stock_review_annotation(
            &db,
            StockReviewAnnotationInput {
                id: "b-stock-note".to_string(),
                scope_type: "stock".to_string(),
                scope_key: "AAPL".to_string(),
                account_id: Some("b".to_string()),
                symbol: Some("AAPL".to_string()),
                annotation_type: "note".to_string(),
                value_json: r#"{"note":"b only"}"#.to_string(),
                source: "user".to_string(),
            },
        )
        .unwrap();
        for date in ["2024-01-01", "2024-01-02", "2024-01-03"] {
            insert_portfolio_value(&db, date, 2_000.0, 7.0);
        }
        let cache_start = day("2023-12-01");
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
        let artifacts = build_stock_review_artifacts(&prepared).unwrap();
        let campaign_a = artifacts
            .campaign_details
            .iter()
            .find(|detail| detail.summary.account_ids == vec!["a".to_string()])
            .unwrap();
        let campaign_b = artifacts
            .campaign_details
            .iter()
            .find(|detail| detail.summary.account_ids == vec!["b".to_string()])
            .unwrap();
        assert_eq!(campaign_a.actions.len(), 1);
        let action_ids = campaign_a
            .actions
            .iter()
            .map(|action| action.action_id.as_str())
            .collect::<BTreeSet<_>>();
        assert!(campaign_a.timeline.iter().all(|flow| flow
            .action_id
            .as_deref()
            .is_none_or(|id| action_ids.contains(id))));
        assert!(campaign_a.annotations.is_empty());
        assert_eq!(campaign_b.annotations.len(), 1);
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
        install_market_sessions(&db, "US", &calendar_dates(day("2024-01-01"), 121));
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
    async fn historical_report_action_matures_beyond_report_end_without_leaking_terminal_value() {
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
        insert_portfolio_value(&db, "2024-01-03", 1_002.0, 7.0);
        // Calendar authority must include the action date as well as all
        // post-action observation sessions.
        let sessions = calendar_dates(day("2024-01-01"), 122);
        install_market_sessions(&db, "US", &sessions);
        let mut stock_points = vec![(day("2024-01-01"), 100.0)];
        stock_points.extend(
            sessions
                .iter()
                .filter(|date| **date > day("2024-01-01"))
                .enumerate()
                .map(|(index, date)| (*date, 101.0 + index as f64)),
        );
        seed_stock_cache_bounds(&db, "AAPL", "US", day("2023-12-20"), &stock_points);
        seed_benchmark_cache(&db, "^GSPC", day("2023-12-20"), None, 100.0);

        let prepared = prepare_cached_stock_review_input(
            &db,
            live_query("2024-01-01", "2024-01-03", Some("US")),
        )
        .await
        .unwrap();
        assert_eq!(
            prepared.forward_actions[0].market_session_dates,
            sessions[..=120]
        );
        let artifacts = build_stock_review_artifacts(&prepared).unwrap();
        assert_eq!(
            artifacts.report.summary.forward_effect.day_60.status.status,
            MetricStatus::Available
        );
        assert_eq!(
            artifacts.campaign_details[0]
                .pnl
                .remaining_market_value_base,
            Some(102.0)
        );
    }

    #[tokio::test]
    async fn authoritative_session_with_missing_benchmark_quote_is_not_shifted() {
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
        let sessions = calendar_dates(day("2024-01-01"), 122);
        let target_60 = sessions[60];
        install_market_sessions(&db, "US", &sessions);
        for date in ["2023-12-31", "2024-01-01", "2024-05-01"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
        }
        let mut stock_points = vec![(day("2024-01-01"), 100.0)];
        stock_points.extend(sessions.iter().map(|date| (*date, 110.0)));
        seed_stock_cache_bounds(&db, "AAPL", "US", day("2023-12-20"), &stock_points);
        seed_benchmark_cache(&db, "^GSPC", day("2023-12-20"), Some(target_60), 100.0);

        let report =
            get_stock_review_report(&db, live_query("2024-01-01", "2024-05-01", Some("US")))
                .await
                .unwrap();
        assert_eq!(
            report.summary.forward_effect.day_60.status.status,
            MetricStatus::Unavailable
        );
        assert_eq!(
            report
                .summary
                .forward_effect
                .day_60
                .amount_weighted_excess_return,
            None
        );
    }

    #[tokio::test]
    async fn missing_authoritative_calendar_suppresses_exact_session_metrics() {
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
        for date in ["2023-12-31", "2024-01-01", "2024-05-01"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
        }
        let points = calendar_dates(day("2024-01-01"), 121)
            .into_iter()
            .map(|date| (date, 100.0))
            .collect::<Vec<_>>();
        seed_stock_cache_bounds(&db, "AAPL", "US", day("2023-12-20"), &points);
        seed_benchmark_cache(&db, "^GSPC", day("2023-12-20"), None, 100.0);

        let report =
            get_stock_review_report(&db, live_query("2024-01-01", "2024-05-01", Some("US")))
                .await
                .unwrap();
        assert_eq!(
            report.summary.forward_effect.day_60.status.status,
            MetricStatus::Unavailable
        );
        assert!(report
            .data_quality
            .issues
            .iter()
            .any(|issue| issue.code == "market_calendar_unavailable"));
    }

    #[tokio::test]
    async fn forward_quality_ignores_unrelated_security_and_currency_gaps() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "trade", "US");
        insert_account(&db, "legacy-cn", "CN");
        insert_live_transaction(
            &db,
            "cash",
            "trade",
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
            "trade",
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
        insert_holding(&db, "cn", "legacy-cn", "600000", "CN", "CNY", 10.0);
        for date in ["2023-12-31", "2024-01-01", "2024-05-01"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
        }
        let sessions = calendar_dates(day("2024-01-01"), 122);
        install_market_sessions(&db, "US", &sessions);
        let mut aapl = vec![(day("2024-01-01"), 100.0)];
        aapl.extend(sessions.iter().map(|date| (*date, 110.0)));
        seed_stock_cache_bounds(&db, "AAPL", "US", day("2023-12-20"), &aapl);
        seed_benchmark_cache(&db, "^GSPC", day("2023-12-20"), None, 100.0);
        seed_benchmark_cache(&db, "000300.SS", day("2023-12-20"), None, 100.0);

        let report = get_stock_review_report(&db, live_query("2024-01-01", "2024-05-01", None))
            .await
            .unwrap();
        assert_eq!(
            report.summary.forward_effect.day_60.status.status,
            MetricStatus::Available
        );
        assert_eq!(
            report.data_quality.forward_effect_availability.status,
            MetricStatus::Available
        );
        assert_ne!(
            report.data_quality.shadow_value_add_availability.status,
            MetricStatus::Available
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

        for (selected, fact) in [("source", "out"), ("destination", "in")] {
            let mut query = live_query("2024-01-02", "2024-01-03", None);
            query.account_id = Some(selected.to_string());
            let filtered = prepare_cached_stock_review_input(&db, query).await.unwrap();
            assert!(filtered
                .attribution_input
                .valuations
                .iter()
                .flat_map(|valuation| valuation.positions.iter())
                .all(|position| position.account_id == selected));
            assert!(filtered
                .campaign_data
                .iter()
                .flat_map(|campaign| campaign.cash_flows.iter())
                .all(|flow| flow.account_id == selected));
            let report = build_stock_review_report_from_cached_data(&filtered).unwrap();
            assert!(report.actions.is_empty());
            assert_eq!(report.campaigns.len(), 1);
            assert_eq!(report.campaigns[0].account_ids, vec![selected.to_string()]);
            assert!(report.campaigns[0].fragments.iter().all(|fragment| {
                fragment.account_id == selected
                    && if fact == "in" {
                        fragment.transfer_in.is_some()
                    } else {
                        fragment.transfer_out.is_some()
                    }
            }));
        }
    }

    #[tokio::test]
    async fn live_corrected_ledger_drives_cash_attribution_and_campaign_flows_once() {
        // Replaying raw rows in any consumer counts both duplicates, counts a
        // confirmed non-trade, or turns a book transfer into account cash.
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "source", "US");
        insert_account(&db, "destination", "US");
        insert_live_transaction(
            &db,
            "deposit",
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
        for id in ["canonical", "duplicate"] {
            insert_live_transaction(
                &db,
                id,
                "source",
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
        }
        insert_live_transaction(
            &db,
            "not-a-trade",
            "source",
            "MSFT",
            "US",
            "BUY",
            1.0,
            50.0,
            50.0,
            0.0,
            "USD",
            "2024-01-02T09:30:00Z",
        );
        insert_live_transaction(
            &db,
            "transfer-out",
            "source",
            "AAPL",
            "US",
            "SELL",
            1.0,
            100.0,
            100.0,
            0.0,
            "USD",
            "2024-01-10T09:30:00Z",
        );
        insert_live_transaction(
            &db,
            "transfer-in",
            "destination",
            "AAPL",
            "US",
            "BUY",
            1.0,
            100.0,
            100.0,
            0.0,
            "USD",
            "2024-01-10T10:30:00Z",
        );
        stock_review_persistence::save_override(
            &db,
            StockReviewOverrideInput {
                id: "duplicate-pair".to_string(),
                override_type: "duplicate".to_string(),
                transaction_ids_json: r#"["canonical","duplicate"]"#.to_string(),
                value_json: "{}".to_string(),
            },
        )
        .unwrap();
        stock_review_persistence::save_override(
            &db,
            StockReviewOverrideInput {
                id: "non-trade".to_string(),
                override_type: "non_trade".to_string(),
                transaction_ids_json: r#"["not-a-trade"]"#.to_string(),
                value_json: "{}".to_string(),
            },
        )
        .unwrap();
        stock_review_persistence::save_override(
            &db,
            StockReviewOverrideInput {
                id: "book-transfer".to_string(),
                override_type: "transfer".to_string(),
                transaction_ids_json: r#"["transfer-out","transfer-in"]"#.to_string(),
                value_json: "{}".to_string(),
            },
        )
        .unwrap();
        for date in ["2024-01-09", "2024-01-10", "2024-01-11"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
            insert_holding_snapshot(&db, date, "destination", "AAPL", "US", 1.0, 100.0, 100.0);
        }
        let cache_start = day("2023-12-01");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[(day("2024-01-09"), 100.0), (day("2024-01-11"), 100.0)],
        );
        seed_default_benchmarks(&db, cache_start);

        let prepared =
            prepare_cached_stock_review_input(&db, live_query("2024-01-10", "2024-01-11", None))
                .await
                .unwrap();
        assert_eq!(prepared.shadow_input.opening_cash[0].amount, 900.0);
        assert_eq!(prepared.shadow_input.opening_positions.len(), 1);
        assert_eq!(prepared.shadow_input.opening_positions[0].quantity, 1.0);
        let ending_cash = &prepared
            .attribution_input
            .valuations
            .last()
            .unwrap()
            .cash_balances;
        assert_eq!(
            ending_cash
                .iter()
                .find(|cash| cash.account_id == "source")
                .unwrap()
                .actual_amount,
            900.0
        );
        assert_eq!(
            ending_cash
                .iter()
                .find(|cash| cash.account_id == "destination")
                .unwrap()
                .actual_amount,
            0.0
        );
        let artifacts = build_stock_review_artifacts(&prepared).unwrap();
        assert_eq!(artifacts.report.actions.len(), 0);
        assert_eq!(artifacts.report.campaigns.len(), 1);
        let timeline = &artifacts.campaign_details[0].timeline;
        assert_eq!(
            timeline
                .iter()
                .filter(|item| item.kind == CampaignCashFlowKind::Buy)
                .count(),
            1
        );
        assert!(timeline.iter().all(|item| item.date != day("2024-01-10")));
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
        let rebuilt = get_stock_review_report(&db, live_query("2024-01-02", "2024-01-03", None))
            .await
            .unwrap();
        assert_eq!(report.actions, rebuilt.actions);
        assert_eq!(report.campaigns, rebuilt.campaigns);
        assert_eq!(report.summary, rebuilt.summary);
    }

    #[tokio::test]
    async fn candidate_async_cache_preparation_rejects_in_scope_user_mutation() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_holding(&db, "holding", "acct", "AAPL", "US", "USD", 1.0);
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
        let cache_start = day("2023-12-20");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[(day("2024-01-01"), 100.0), (day("2024-01-03"), 100.0)],
        );
        seed_default_benchmarks(&db, cache_start);

        let query = live_query("2024-01-02", "2024-01-03", Some("US"));
        let mut prepared_candidate = stock_review_persistence::prepare_override_candidate(
            &db,
            StockReviewOverrideInput {
                id: "race".to_string(),
                override_type: "non_trade".to_string(),
                transaction_ids_json: r#"["buy"]"#.to_string(),
                value_json: "{}".to_string(),
            },
        )
        .unwrap();
        stock_review_persistence::scope_candidate_to_query(&db, &mut prepared_candidate, &query)
            .unwrap();
        let canonical = prepared_candidate.input.clone();
        let candidate_record = StockReviewOverride {
            id: canonical.id,
            override_type: canonical.override_type,
            transaction_ids_json: canonical.transaction_ids_json,
            value_json: canonical.value_json,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let result = prepare_cached_stock_review_input_with_candidate_and_cache_hook(
            &db,
            query,
            Some(candidate_record),
            Some(&mut prepared_candidate),
            |db| {
                db.conn
                    .lock()
                    .map_err(|error| error.to_string())?
                    .execute("UPDATE holdings SET shares = 2 WHERE id = 'holding'", [])
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
        )
        .await;
        let error = result.unwrap_err();
        assert!(
            error.contains("changed during cache preparation"),
            "unexpected preparation error: {error}"
        );
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM stock_review_overrides", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn candidate_reloads_future_evaluation_calendar_after_cache_fill() {
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
        for id in ["buy-a", "buy-b"] {
            insert_live_transaction(
                &db,
                id,
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
        }
        for date in ["2023-12-31", "2024-01-01", "2024-01-03"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
        }
        let sessions = calendar_dates(day("2024-01-01"), 130);
        let missing_future_session = sessions[60];
        install_market_sessions(&db, "US", &sessions);
        let stock_points = sessions
            .iter()
            .map(|date| (*date, 110.0))
            .collect::<Vec<_>>();
        seed_stock_cache_bounds(&db, "AAPL", "US", day("2023-12-20"), &stock_points);
        seed_benchmark_cache(&db, "^GSPC", day("2023-12-20"), None, 100.0);

        let query = live_query("2024-01-01", "2024-01-03", Some("US"));
        let mut prepared_candidate = stock_review_persistence::prepare_override_candidate(
            &db,
            StockReviewOverrideInput {
                id: "calendar-race".to_string(),
                override_type: "duplicate".to_string(),
                transaction_ids_json: r#"["buy-a","buy-b"]"#.to_string(),
                value_json: "{}".to_string(),
            },
        )
        .unwrap();
        stock_review_persistence::scope_candidate_to_query(&db, &mut prepared_candidate, &query)
            .unwrap();
        let canonical = prepared_candidate.input.clone();
        let candidate_record = StockReviewOverride {
            id: canonical.id,
            override_type: canonical.override_type,
            transaction_ids_json: canonical.transaction_ids_json,
            value_json: canonical.value_json,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let prepared = prepare_cached_stock_review_input_with_candidate_and_cache_hook(
            &db,
            query,
            Some(candidate_record),
            Some(&mut prepared_candidate),
            move |db| {
                let conn = db.conn.lock().map_err(|error| error.to_string())?;
                conn.execute(
                    "DELETE FROM stock_market_sessions WHERE market = 'US' AND date = ?1",
                    params![missing_future_session.format("%Y-%m-%d").to_string()],
                )
                .map_err(|error| error.to_string())?;
                conn.execute(
                    "UPDATE stock_market_calendar_coverage SET revision = 'fixture-v2' WHERE market = 'US'",
                    [],
                )
                .map_err(|error| error.to_string())?;
                Ok(())
            },
        )
        .await
        .unwrap();

        assert!(prepared.forward_actions[0].market_session_dates.is_empty());
        assert_eq!(
            prepared.forward_actions[0].availability.status,
            MetricStatus::Unavailable
        );
    }

    #[tokio::test]
    async fn candidate_rejects_post_report_transaction_and_split_mutation() {
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
            "2024-01-02T09:30:00Z",
        );
        for date in ["2024-01-01", "2024-01-02", "2024-01-03"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
        }
        let cache_start = day("2023-12-20");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[(day("2024-01-01"), 100.0), (day("2024-01-03"), 100.0)],
        );
        seed_default_benchmarks(&db, cache_start);

        let query = live_query("2024-01-02", "2024-01-03", Some("US"));
        let mut prepared_candidate = stock_review_persistence::prepare_override_candidate(
            &db,
            StockReviewOverrideInput {
                id: "future-user-race".to_string(),
                override_type: "non_trade".to_string(),
                transaction_ids_json: r#"["buy"]"#.to_string(),
                value_json: "{}".to_string(),
            },
        )
        .unwrap();
        stock_review_persistence::scope_candidate_to_query(&db, &mut prepared_candidate, &query)
            .unwrap();
        let canonical = prepared_candidate.input.clone();
        let candidate_record = StockReviewOverride {
            id: canonical.id,
            override_type: canonical.override_type,
            transaction_ids_json: canonical.transaction_ids_json,
            value_json: canonical.value_json,
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        let result = prepare_cached_stock_review_input_with_candidate_and_cache_hook(
            &db,
            query,
            Some(candidate_record),
            Some(&mut prepared_candidate),
            |db| {
                insert_live_transaction(
                    db,
                    "late-sell",
                    "acct",
                    "AAPL",
                    "US",
                    "SELL",
                    1.0,
                    110.0,
                    110.0,
                    0.0,
                    "USD",
                    "2024-02-01T09:30:00Z",
                );
                db.conn
                    .lock()
                    .map_err(|error| error.to_string())?
                    .execute(
                        "INSERT INTO stock_splits (stock_code, split_date, ratio_from, ratio_to, created_at)
                         VALUES ('AAPL', '2024-02-02', 1, 2, '2024-02-02')",
                        [],
                    )
                    .map_err(|error| error.to_string())?;
                Ok(())
            },
        )
        .await;

        let error = result.unwrap_err();
        assert!(
            error.contains("changed during cache preparation"),
            "unexpected preparation error: {error}"
        );
    }

    #[tokio::test]
    async fn override_preview_rejects_a_reference_outside_exact_query_scope() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "a", "US");
        insert_account(&db, "b", "US");
        insert_live_transaction(
            &db,
            "b-buy",
            "b",
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
        for date in ["2024-01-01", "2024-01-03"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
        }
        seed_default_benchmarks(&db, day("2023-12-20"));
        let mut query = live_query("2024-01-02", "2024-01-03", None);
        query.account_id = Some("a".to_string());
        let result = confirm_stock_review_override(
            &db,
            query,
            StockReviewOverrideInput {
                id: "out-of-scope".to_string(),
                override_type: "non_trade".to_string(),
                transaction_ids_json: r#"["b-buy"]"#.to_string(),
                value_json: "{}".to_string(),
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM stock_review_overrides", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn in_scope_candidate_that_breaks_position_replay_fails_after_in_memory_insertion() {
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
        insert_live_transaction(
            &db,
            "sell",
            "acct",
            "AAPL",
            "US",
            "SELL",
            1.0,
            100.0,
            100.0,
            0.0,
            "USD",
            "2024-01-03T09:30:00Z",
        );
        for date in ["2024-01-01", "2024-01-02", "2024-01-03"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
        }
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            day("2023-12-20"),
            &[
                (day("2024-01-01"), 100.0),
                (day("2024-01-02"), 100.0),
                (day("2024-01-03"), 100.0),
            ],
        );
        seed_default_benchmarks(&db, day("2023-12-20"));
        let result = confirm_stock_review_override(
            &db,
            live_query("2024-01-02", "2024-01-03", None),
            StockReviewOverrideInput {
                id: "break-replay".to_string(),
                override_type: "non_trade".to_string(),
                transaction_ids_json: r#"["buy"]"#.to_string(),
                value_json: "{}".to_string(),
            },
        )
        .await;
        assert!(result.is_err());
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM stock_review_overrides", [], |row| row
                    .get::<_, i64>(0))
                .unwrap(),
            0
        );
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
        let corrected = CorrectedTransaction {
            transaction,
            is_transfer: false,
            has_cash_effect: true,
            action_id: None,
        };
        let (flows, complete) =
            external_flows_base_from_db(&db, &[corrected], &query, day("2024-01-01"));
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
        let (baseline, values, availability, nav_complete) =
            load_actual_values(&db, &query, Some(day("2023-12-31"))).unwrap();
        assert!(nav_complete);
        assert_eq!(availability.status, MetricStatus::Available);
        assert_eq!(baseline.unwrap().value_base, 7_000.0);
        assert_eq!(values[0].value_base, 7_700.0);
    }

    #[test]
    fn actual_snapshot_baseline_uses_the_exact_authoritative_cutoff() {
        let db = Database::new(":memory:").unwrap();
        for (date, value) in [
            ("2024-01-05", 100.0),
            ("2024-01-07", 999.0),
            ("2024-01-08", 110.0),
        ] {
            insert_portfolio_value(&db, date, value, 7.0);
        }
        let mut query = complete_cached_fixture(false).query;
        query.start_date = day("2024-01-08");
        query.end_date = day("2024-01-08");
        query.market = None;

        let (baseline, values, availability, nav_complete) =
            load_actual_values(&db, &query, Some(day("2024-01-05"))).unwrap();
        assert!(nav_complete);
        assert_eq!(availability.status, MetricStatus::Available);
        assert_eq!(baseline.unwrap().date, day("2024-01-05"));
        assert_eq!(values.last().unwrap().date, day("2024-01-08"));
    }

    #[tokio::test]
    async fn filtered_nav_never_averages_the_surviving_fx_subset() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "CN");
        insert_holding(&db, "cn", "acct", "600000", "CN", "CNY", 1.0);
        insert_holding_snapshot(&db, "2024-01-10", "acct", "600000", "CN", 1.0, 700.0, 700.0);
        insert_holding_snapshot(&db, "2024-01-11", "acct", "600000", "CN", 1.0, 700.0, 700.0);
        insert_portfolio_value(&db, "2024-01-10", 100.0, 7.0);
        let cache_start = day("2023-12-01");
        seed_stock_cache_bounds(
            &db,
            "600000",
            "CN",
            cache_start,
            &[(day("2024-01-10"), 700.0), (day("2024-01-11"), 700.0)],
        );
        seed_default_benchmarks(&db, cache_start);
        let mut query = live_query("2024-01-10", "2024-01-11", None);
        query.account_id = Some("acct".to_string());

        let prepared = prepare_cached_stock_review_input(&db, query).await.unwrap();
        assert!(prepared.result_quality_input.actual_values.is_empty());
        assert_eq!(prepared.attribution_input.average_portfolio_nav, None);
        assert!(prepared
            .preparation_issues
            .iter()
            .any(|issue| issue.code == "snapshot_fx_unavailable"));
    }

    #[tokio::test]
    async fn filtered_stock_snapshots_without_authoritative_cash_disable_nav_ratios() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_holding(&db, "stock", "acct", "AAPL", "US", "USD", 1.0);
        for date in ["2024-01-10", "2024-01-11"] {
            insert_holding_snapshot(&db, date, "acct", "AAPL", "US", 1.0, 100.0, 100.0);
            insert_portfolio_value(&db, date, 100.0, 7.0);
        }
        let cache_start = day("2023-12-01");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[(day("2024-01-10"), 100.0), (day("2024-01-11"), 100.0)],
        );
        seed_default_benchmarks(&db, cache_start);
        let mut query = live_query("2024-01-10", "2024-01-11", None);
        query.account_id = Some("acct".to_string());

        let prepared = prepare_cached_stock_review_input(&db, query).await.unwrap();
        assert_eq!(prepared.attribution_input.average_portfolio_nav, None);
        let report = build_stock_review_report_from_cached_data(&prepared).unwrap();
        assert_eq!(report.summary.risk_structure.one_way_turnover, None);
        assert_eq!(report.summary.risk_structure.fee_drag, None);
        assert!(report
            .data_quality
            .issues
            .iter()
            .any(|issue| issue.code == "filtered_nav_cash_unavailable"));
    }

    #[tokio::test]
    async fn missing_action_date_fx_disables_turnover_and_fee_drag() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "CN");
        insert_live_transaction(
            &db,
            "cash",
            "acct",
            "$CASH-CNY",
            "CN",
            "BUY",
            1000.0,
            1.0,
            1000.0,
            0.0,
            "CNY",
            "2024-01-01T08:00:00Z",
        );
        insert_live_transaction(
            &db,
            "buy",
            "acct",
            "600000",
            "CN",
            "BUY",
            1.0,
            700.0,
            700.0,
            7.0,
            "CNY",
            "2024-01-10T09:30:00Z",
        );
        insert_portfolio_value(&db, "2024-01-09", 142.857142857, 7.0);
        insert_portfolio_value(&db, "2024-01-11", 142.857142857, 7.0);
        let cache_start = day("2023-12-01");
        seed_stock_cache_bounds(
            &db,
            "600000",
            "CN",
            cache_start,
            &[(day("2024-01-10"), 700.0), (day("2024-01-11"), 700.0)],
        );
        seed_default_benchmarks(&db, cache_start);

        let prepared =
            prepare_cached_stock_review_input(&db, live_query("2024-01-10", "2024-01-11", None))
                .await
                .unwrap();
        let report = build_stock_review_report_from_cached_data(&prepared).unwrap();
        assert_eq!(report.summary.risk_structure.one_way_turnover, None);
        assert_eq!(report.summary.risk_structure.fee_drag, None);
        assert!(report
            .data_quality
            .issues
            .iter()
            .any(|issue| issue.code == "action_fx_unavailable"));
    }

    #[tokio::test]
    async fn campaign_retains_non_base_flows_when_exact_fx_is_missing() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "CN");
        insert_live_transaction(
            &db,
            "cash",
            "acct",
            "$CASH-CNY",
            "CN",
            "BUY",
            1000.0,
            1.0,
            1000.0,
            0.0,
            "CNY",
            "2024-01-01T08:00:00Z",
        );
        insert_live_transaction(
            &db,
            "buy",
            "acct",
            "600000",
            "CN",
            "BUY",
            1.0,
            700.0,
            700.0,
            7.0,
            "CNY",
            "2024-01-10T09:30:00Z",
        );
        insert_live_transaction(
            &db,
            "pay",
            "acct",
            "600000",
            "CN",
            "PAY",
            1.0,
            10.0,
            10.0,
            0.0,
            "CNY",
            "2024-01-11T09:30:00Z",
        );
        insert_portfolio_value(&db, "2024-01-09", 142.857142857, 7.0);
        insert_portfolio_value(&db, "2024-01-12", 144.285714286, 7.0);
        let cache_start = day("2023-12-01");
        seed_stock_cache_bounds(
            &db,
            "600000",
            "CN",
            cache_start,
            &[
                (day("2024-01-10"), 700.0),
                (day("2024-01-11"), 700.0),
                (day("2024-01-12"), 700.0),
            ],
        );
        seed_default_benchmarks(&db, cache_start);

        let prepared =
            prepare_cached_stock_review_input(&db, live_query("2024-01-10", "2024-01-12", None))
                .await
                .unwrap();
        assert_eq!(prepared.campaign_data[0].cash_flows.len(), 3);
        let mut artifacts = build_stock_review_artifacts(&prepared).unwrap();
        let detail = artifacts.campaign_details.remove(0);
        assert_eq!(detail.pnl_availability.status, MetricStatus::Unavailable);
        assert!(detail
            .issues
            .iter()
            .any(|issue| issue.code == "campaign_fx_unavailable"));
    }

    #[tokio::test]
    async fn legacy_current_holding_is_reversed_for_post_origin_split_then_replayed_once() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_holding(&db, "legacy", "acct", "AAPL", "US", "USD", 20.0);
        db.conn.lock().unwrap().execute(
            "INSERT INTO stock_splits (stock_code, split_date, ratio_from, ratio_to, created_at) VALUES ('AAPL', '2024-01-10', 1, 2, '2024-01-10')",
            [],
        ).unwrap();
        for (date, value) in [
            ("2024-01-09", 1000.0),
            ("2024-01-10", 1000.0),
            ("2024-01-11", 1000.0),
        ] {
            insert_portfolio_value(&db, date, value, 7.0);
        }
        let cache_start = day("2023-12-01");
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
        seed_default_benchmarks(&db, cache_start);

        let prepared =
            prepare_cached_stock_review_input(&db, live_query("2024-01-10", "2024-01-11", None))
                .await
                .unwrap();
        assert_eq!(prepared.shadow_input.opening_positions[0].quantity, 10.0);
        assert_eq!(prepared.shadow_input.split_events.len(), 1);
        let shadow = build_shadow_series(&prepared.shadow_input);
        assert_eq!(shadow.ending_value, Some(1000.0));
    }

    #[tokio::test]
    async fn nonempty_session_rows_without_coverage_metadata_are_not_authority() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_live_transaction(
            &db,
            "cash",
            "acct",
            "$CASH-USD",
            "US",
            "BUY",
            1000.0,
            1.0,
            1000.0,
            0.0,
            "USD",
            "2024-01-01T08:00:00Z",
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
        insert_portfolio_value(&db, "2024-01-01", 1000.0, 7.0);
        insert_portfolio_value(&db, "2024-01-03", 1000.0, 7.0);
        let cache_start = day("2023-12-01");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[(day("2024-01-02"), 100.0), (day("2024-01-03"), 100.0)],
        );
        seed_benchmark_cache(&db, "^GSPC", cache_start, None, 100.0);
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO stock_market_sessions (market, date, is_session, source, updated_at)
             VALUES ('US', '2024-01-02', 1, 'unproven', '2024-01-02')",
                [],
            )
            .unwrap();

        let prepared = prepare_cached_stock_review_input(
            &db,
            live_query("2024-01-02", "2024-01-03", Some("US")),
        )
        .await
        .unwrap();
        assert!(prepared.forward_actions[0].market_session_dates.is_empty());
        assert_eq!(
            prepared.forward_actions[0].availability.status,
            MetricStatus::Unavailable
        );
        assert!(prepared
            .result_quality_input
            .expected_actual_dates
            .is_empty());
        let result = calculate_result_quality(&prepared.result_quality_input);
        assert_eq!(result.metric.availability.status, MetricStatus::Unavailable);
        assert_eq!(result.metric.portfolio_return, None);
        assert_eq!(result.max_drawdown.max_drawdown, None);
        assert!(prepared
            .preparation_issues
            .iter()
            .any(|issue| issue.code == "market_calendar_unavailable"));
    }

    #[tokio::test]
    async fn declared_calendar_with_missing_interior_day_is_invalid() {
        let db = Database::new(":memory:").unwrap();
        install_market_sessions(&db, "US", &calendar_dates(day("2024-01-01"), 5));
        db.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM stock_market_sessions WHERE market = 'US' AND date = '2024-01-03'",
                [],
            )
            .unwrap();
        let calendar =
            load_market_sessions(&db, "US", day("2024-01-01"), day("2024-01-05")).unwrap();
        assert_eq!(calendar.availability.status, MetricStatus::Unavailable);
        assert!(calendar.sessions.is_empty());
    }

    #[tokio::test]
    async fn campaign_terminal_is_unavailable_when_calendar_coverage_stops_before_cutoff() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_live_transaction(
            &db,
            "cash",
            "acct",
            "$CASH-USD",
            "US",
            "BUY",
            1000.0,
            1.0,
            1000.0,
            0.0,
            "USD",
            "2024-01-01T08:00:00Z",
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
            "2024-01-09T09:30:00Z",
        );
        insert_portfolio_value(&db, "2024-01-08", 1000.0, 7.0);
        insert_portfolio_value(&db, "2024-01-11", 1000.0, 7.0);
        let cache_start = day("2023-12-01");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[
                (day("2024-01-09"), 100.0),
                (day("2024-01-10"), 110.0),
                (day("2024-01-11"), 120.0),
            ],
        );
        seed_benchmark_cache(&db, "^GSPC", cache_start, None, 100.0);
        install_market_sessions(&db, "US", &calendar_dates(day("2024-01-01"), 10));

        let prepared = prepare_cached_stock_review_input(
            &db,
            live_query("2024-01-09", "2024-01-11", Some("US")),
        )
        .await
        .unwrap();
        let mut artifacts = build_stock_review_artifacts(&prepared).unwrap();
        let detail = artifacts.campaign_details.remove(0);
        assert_eq!(detail.pnl_availability.status, MetricStatus::Unavailable);
        assert!(detail
            .issues
            .iter()
            .any(|issue| issue.code == "campaign_calendar_unavailable"));
    }

    #[tokio::test]
    async fn same_id_candidate_replaces_stale_preview_state_and_matches_saved_report() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct", "US");
        insert_live_transaction(
            &db,
            "cash",
            "acct",
            "$CASH-USD",
            "US",
            "BUY",
            1000.0,
            1.0,
            1000.0,
            0.0,
            "USD",
            "2024-01-01T08:00:00Z",
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
            "2024-01-10T09:30:00Z",
        );
        for date in ["2024-01-09", "2024-01-10", "2024-01-11"] {
            insert_portfolio_value(&db, date, 1000.0, 7.0);
        }
        let cache_start = day("2023-12-01");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[(day("2024-01-10"), 100.0), (day("2024-01-11"), 100.0)],
        );
        seed_default_benchmarks(&db, cache_start);
        let input = StockReviewOverrideInput {
            id: "same-id".to_string(),
            override_type: "non_trade".to_string(),
            transaction_ids_json: r#"["buy"]"#.to_string(),
            value_json: "{}".to_string(),
        };
        stock_review_persistence::save_override(&db, input.clone()).unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE transactions SET price = 101 WHERE id = 'buy'", [])
            .unwrap();
        assert_eq!(
            stock_review_persistence::list_overrides(&db)
                .unwrap()
                .stale_overrides
                .len(),
            1
        );

        let query = live_query("2024-01-10", "2024-01-11", None);
        let candidate = confirm_stock_review_override(&db, query.clone(), input)
            .await
            .unwrap();
        assert!(!candidate
            .data_quality
            .issues
            .iter()
            .any(|issue| issue.code == "stale_override"));
        let saved = get_stock_review_report(&db, query).await.unwrap();
        assert_eq!(candidate.summary, saved.summary);
        assert_eq!(candidate.actions, saved.actions);
        assert_eq!(candidate.campaigns, saved.campaigns);
        assert_eq!(candidate.data_quality.issues, saved.data_quality.issues);
    }

    #[tokio::test]
    async fn report_scopes_stale_override_rows_and_issues_to_the_query() {
        let db = Database::new(":memory:").unwrap();
        insert_account(&db, "acct-a", "US");
        insert_account(&db, "acct-b", "US");
        insert_live_transaction(
            &db,
            "cash-a",
            "acct-a",
            "$CASH-USD",
            "US",
            "BUY",
            1_000.0,
            1.0,
            1_000.0,
            0.0,
            "USD",
            "2024-01-01T08:00:00Z",
        );
        for (id, account, symbol, price) in [
            ("buy-a", "acct-a", "AAPL", 100.0),
            ("buy-b", "acct-b", "MSFT", 200.0),
        ] {
            insert_live_transaction(
                &db,
                id,
                account,
                symbol,
                "US",
                "BUY",
                1.0,
                price,
                price,
                0.0,
                "USD",
                "2024-01-10T09:30:00Z",
            );
            stock_review_persistence::save_override(
                &db,
                StockReviewOverrideInput {
                    id: format!("stale-{account}"),
                    override_type: "non_trade".to_string(),
                    transaction_ids_json: serde_json::json!([id]).to_string(),
                    value_json: "{}".to_string(),
                },
            )
            .unwrap();
        }
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE transactions
                 SET price = price + 1, total_amount = total_amount + 1
                 WHERE id IN ('buy-a', 'buy-b')",
                [],
            )
            .unwrap();
        for date in ["2024-01-09", "2024-01-10", "2024-01-11"] {
            insert_portfolio_value(&db, date, 1_000.0, 7.0);
        }
        let cache_start = day("2023-12-01");
        seed_stock_cache_bounds(
            &db,
            "AAPL",
            "US",
            cache_start,
            &[(day("2024-01-10"), 101.0), (day("2024-01-11"), 101.0)],
        );
        seed_default_benchmarks(&db, cache_start);
        let mut query = live_query("2024-01-10", "2024-01-11", Some("US"));
        query.account_id = Some("acct-a".to_string());

        let prepared = prepare_cached_stock_review_input(&db, query).await.unwrap();
        let stale = prepared
            .persisted_override_issues
            .iter()
            .filter(|issue| issue.code == "stale_override")
            .collect::<Vec<_>>();
        assert_eq!(stale.len(), 1);
        assert!(stale[0].message.contains("stale-acct-a"));
        assert!(!stale[0].message.contains("stale-acct-b"));
    }

    #[test]
    fn stock_annotations_require_explicit_effective_time_when_multiple_cycles_match() {
        let campaign = |id: &str, start: &str, end: &str| StockCampaignSummary {
            campaign_id: id.to_string(),
            account_ids: vec!["acct".to_string()],
            action_ids: vec![],
            fragments: vec![],
            campaign_status: StockCampaignStatus::Completed,
            availability: available(),
            symbol: "AAPL".to_string(),
            market: "US".to_string(),
            started_at: format!("{start}T09:30:00Z"),
            ended_at: Some(format!("{end}T16:00:00Z")),
            contribution: None,
        };
        let campaigns = vec![
            campaign("first", "2024-01-01", "2024-01-10"),
            campaign("second", "2024-02-01", "2024-02-10"),
        ];
        let annotation = |id: &str, value_json: &str| StockReviewAnnotation {
            id: id.to_string(),
            scope_type: "stock".to_string(),
            scope_key: "AAPL".to_string(),
            account_id: Some("acct".to_string()),
            symbol: Some("AAPL".to_string()),
            annotation_type: "thesis".to_string(),
            value_json: value_json.to_string(),
            source: "user".to_string(),
            created_at: "2026-08-28T00:00:00Z".to_string(),
            updated_at: "2026-08-28T00:00:00Z".to_string(),
        };
        let undated = annotation("undated", r#"{"note":"report-level"}"#);
        assert!(!annotation_applies_to_campaign(
            &undated,
            &campaigns[0],
            &campaigns,
            day("2026-08-28"),
        ));
        assert!(!annotation_applies_to_campaign(
            &undated,
            &campaigns[1],
            &campaigns,
            day("2026-08-28"),
        ));

        let dated = annotation("dated", r#"{"effective_date":"2024-02-05"}"#);
        assert!(!annotation_applies_to_campaign(
            &dated,
            &campaigns[0],
            &campaigns,
            day("2026-08-28"),
        ));
        assert!(annotation_applies_to_campaign(
            &dated,
            &campaigns[1],
            &campaigns,
            day("2026-08-28"),
        ));
    }

    #[test]
    fn historical_display_excludes_future_quarterly_notes() {
        let db = Database::new(":memory:").unwrap();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO quarterly_snapshots (id, quarter, snapshot_date, created_at)
             VALUES ('past', '2024Q1', '2024-01-15', '2024-01-15')",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO quarterly_snapshots (id, quarter, snapshot_date, created_at)
             VALUES ('future', '2024Q2', '2024-02-15', '2024-02-15')",
            [],
        )
        .unwrap();
        for (id, snapshot, note) in [
            ("past-note", "past", "known then"),
            ("future-note", "future", "not yet known"),
        ] {
            conn.execute(
                "INSERT INTO quarterly_holding_snapshots
                    (id, quarterly_snapshot_id, account_id, symbol, name, market, notes)
                 VALUES (?1, ?2, 'acct', 'AAPL', 'AAPL', 'US', ?3)",
                params![id, snapshot, note],
            )
            .unwrap();
        }
        drop(conn);

        let mut query = live_query("2024-01-01", "2024-01-31", Some("US"));
        query.account_id = Some("acct".to_string());
        let annotations = load_display_context(&db, &query).unwrap();
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].id, "quarterly:past-note");
    }

    #[test]
    fn active_historical_campaign_rejects_annotation_starting_after_report_as_of() {
        let campaign = StockCampaignSummary {
            campaign_id: "active".to_string(),
            account_ids: vec!["acct".to_string()],
            action_ids: vec![],
            fragments: vec![],
            campaign_status: StockCampaignStatus::Active,
            availability: available(),
            symbol: "AAPL".to_string(),
            market: "US".to_string(),
            started_at: "2024-01-01T09:30:00Z".to_string(),
            ended_at: None,
            contribution: None,
        };
        let annotation = StockReviewAnnotation {
            id: "future-thesis".to_string(),
            scope_type: "stock".to_string(),
            scope_key: "AAPL".to_string(),
            account_id: Some("acct".to_string()),
            symbol: Some("AAPL".to_string()),
            annotation_type: "thesis".to_string(),
            value_json: r#"{"effective_start":"2024-02-01"}"#.to_string(),
            source: "user".to_string(),
            created_at: "2024-01-15T00:00:00Z".to_string(),
            updated_at: "2024-01-15T00:00:00Z".to_string(),
        };

        assert!(!annotation_applies_to_campaign(
            &annotation,
            &campaign,
            std::slice::from_ref(&campaign),
            day("2024-01-31"),
        ));
    }

    #[test]
    fn malformed_legacy_annotation_date_is_not_treated_as_undated_display_context() {
        let annotation = StockReviewAnnotation {
            id: "legacy-malformed".to_string(),
            scope_type: "stock".to_string(),
            scope_key: "AAPL".to_string(),
            account_id: Some("acct".to_string()),
            symbol: Some("AAPL".to_string()),
            annotation_type: "thesis".to_string(),
            value_json: r#"{"effective_date":"2024-02-30","note":"invalid legacy row"}"#
                .to_string(),
            source: "user".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        };

        assert!(!annotation_visible_as_of(&annotation, day("2024-03-01")));
    }

    #[test]
    fn missing_actual_terminal_does_not_suppress_an_independent_shadow_result() {
        let mut input = complete_cached_fixture(false);
        input.result_quality_input.actual_values.pop();

        let report = build_stock_review_report_from_cached_data(&input).unwrap();
        assert_eq!(report.summary.result_quality.portfolio_return, None);
        assert_eq!(report.summary.max_drawdown.max_drawdown, None);
        assert_eq!(report.summary.result_quality.shadow_return, Some(0.0));
        assert_eq!(
            report.data_quality.actual_result_availability.status,
            MetricStatus::Unavailable
        );
        assert_eq!(
            report.data_quality.shadow_value_add_availability.status,
            MetricStatus::Unavailable
        );
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
