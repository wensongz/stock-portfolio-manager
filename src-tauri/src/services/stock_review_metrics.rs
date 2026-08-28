#![allow(dead_code)]

use crate::models::performance::ReturnDataPoint;
use crate::models::stock_review::{
    CampaignCashFlowKind, CampaignPnl, CampaignTimelineItem, ConcentrationSnapshot,
    ForwardEffectWindow, MaxDrawdownMetric, MetricAvailability, MetricStatus,
    RebalanceValueAddMetric, ResultQualityMetric, RiskStructureDetail, StockActionReview,
    StockCampaignDetail, StockCampaignStatus, StockCampaignSummary, StockReviewAnnotation,
};
use crate::services::performance_service::build_twr_return_series;
use crate::services::stock_review_market_data::{nth_market_session_after, MarketReturnMode};
use crate::services::stock_review_quality::merge_metric_statuses;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const EPSILON: f64 = 1e-9;

fn availability(status: MetricStatus, note: Option<String>) -> MetricAvailability {
    MetricAvailability { status, note }
}

fn unavailable(note: impl Into<String>) -> MetricAvailability {
    availability(MetricStatus::Unavailable, Some(note.into()))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PortfolioValuePoint {
    pub date: NaiveDate,
    pub value_base: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExternalFlowBase {
    pub date: NaiveDate,
    pub amount_base: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketValue {
    pub market: String,
    pub value_base: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkPoint {
    pub date: NaiveDate,
    pub value: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct BenchmarkSeriesInput {
    pub market: String,
    pub availability: MetricAvailability,
    pub points: Vec<BenchmarkPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BenchmarkSelection {
    AutomaticMixed,
    SingleMarket(String),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResultQualityInput {
    pub actual_values: Vec<PortfolioValuePoint>,
    pub baseline: Option<PortfolioValuePoint>,
    pub external_flows_base: Vec<ExternalFlowBase>,
    pub actual_availability: MetricAvailability,
    pub opening_market_values_base: Vec<MarketValue>,
    pub opening_cash_value_base: f64,
    pub benchmark_series: Vec<BenchmarkSeriesInput>,
    pub benchmark_selection: BenchmarkSelection,
    pub shadow_curve: Vec<CurveReturnPoint>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CurveReturnPoint {
    pub date: NaiveDate,
    /// Cumulative return expressed as a fraction, so 0.05 is 5%.
    pub cumulative_return: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NormalizedReviewCurvePoint {
    pub date: NaiveDate,
    pub portfolio_index: Option<f64>,
    pub shadow_index: Option<f64>,
    pub benchmark_index: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResultQualityOutput {
    pub metric: ResultQualityMetric,
    pub max_drawdown: MaxDrawdownMetric,
    pub actual_twr_series: Vec<ReturnDataPoint>,
    pub normalized_curve: Vec<NormalizedReviewCurvePoint>,
    pub fixed_weights: BTreeMap<String, f64>,
}

pub fn calculate_result_quality(input: &ResultQualityInput) -> ResultQualityOutput {
    let actual_twr = validated_actual_twr(
        &input.actual_values,
        input.baseline.as_ref(),
        &input.external_flows_base,
        &input.actual_availability,
    );
    let actual_twr_series = actual_twr.clone().unwrap_or_default();
    let actual_return = actual_twr_series
        .last()
        .map(|point| point.cumulative_return / 100.0);

    let fixed_weights = opening_weights(input);
    let benchmark_by_date = benchmark_returns_by_date(input, &fixed_weights);
    let benchmark_return = input
        .actual_values
        .last()
        .and_then(|point| benchmark_by_date.get(&point.date).copied().flatten());
    let benchmark_status = benchmark_status(input, &fixed_weights);
    let actual_usable = actual_return.is_some();
    let benchmark_usable =
        benchmark_return.is_some() && benchmark_status != MetricStatus::Unavailable;
    let status = if !actual_usable {
        MetricStatus::Unavailable
    } else if !benchmark_usable {
        MetricStatus::Degraded
    } else {
        merge_metric_statuses(&[input.actual_availability.status.clone(), benchmark_status])
    };
    let portfolio_return = actual_return;
    let benchmark_return = benchmark_usable.then_some(benchmark_return).flatten();
    let note = if !actual_usable {
        Some(
            "Actual TWR is unavailable because the value or cash-flow history is invalid."
                .to_string(),
        )
    } else if !benchmark_usable {
        Some(
            "Actual TWR is available; the selected fixed-weight benchmark is unavailable."
                .to_string(),
        )
    } else {
        None
    };
    let metric = ResultQualityMetric {
        availability: availability(status, note),
        portfolio_return,
        shadow_return: None,
        benchmark_return,
        excess_return: portfolio_return
            .zip(benchmark_return)
            .map(|(actual, benchmark)| actual - benchmark),
        active_return: portfolio_return
            .zip(benchmark_return)
            .map(|(actual, benchmark)| actual - benchmark),
    };
    let normalized_curve = normalized_review_curve(input, &actual_twr_series, &benchmark_by_date);
    let max_drawdown = actual_twr
        .as_ref()
        .map(|series| {
            drawdown_from_twr(
                series,
                input.baseline.as_ref(),
                input.actual_availability.clone(),
            )
        })
        .unwrap_or_else(|_| unavailable_drawdown());

    ResultQualityOutput {
        metric,
        max_drawdown,
        actual_twr_series,
        normalized_curve,
        fixed_weights,
    }
}

fn opening_weights(input: &ResultQualityInput) -> BTreeMap<String, f64> {
    if !input.opening_cash_value_base.is_finite()
        || input.opening_cash_value_base < 0.0
        || input.opening_market_values_base.iter().any(|value| {
            !value.value_base.is_finite()
                || value.value_base < 0.0
                || value.market.trim().is_empty()
        })
    {
        return BTreeMap::new();
    }
    let mut opening_values = BTreeMap::new();
    for value in &input.opening_market_values_base {
        if value.value_base > 0.0 {
            *opening_values
                .entry(value.market.trim().to_ascii_uppercase())
                .or_insert(0.0) += value.value_base;
        }
    }
    let stock_total = opening_values.values().sum::<f64>();
    let total = stock_total + input.opening_cash_value_base;
    if !total.is_finite() || total <= 0.0 {
        return BTreeMap::new();
    }
    let mut weights = opening_values
        .into_iter()
        .map(|(market, value)| (market, value / total))
        .collect::<BTreeMap<_, _>>();
    if input.opening_cash_value_base > 0.0 {
        weights.insert("cash".to_string(), input.opening_cash_value_base / total);
    }
    let weight_sum = weights.values().sum::<f64>();
    if !weight_sum.is_finite() || (weight_sum - 1.0).abs() > EPSILON {
        return BTreeMap::new();
    }
    weights
}

fn benchmark_status(input: &ResultQualityInput, weights: &BTreeMap<String, f64>) -> MetricStatus {
    match &input.benchmark_selection {
        BenchmarkSelection::SingleMarket(market) => input
            .benchmark_series
            .iter()
            .find(|series| series.market == *market)
            .map(|series| series.availability.status.clone())
            .unwrap_or(MetricStatus::Unavailable),
        BenchmarkSelection::AutomaticMixed => {
            if weights.is_empty() {
                return MetricStatus::Unavailable;
            }
            let markets = weights
                .keys()
                .filter(|market| market.as_str() != "cash")
                .collect::<Vec<_>>();
            if markets.is_empty() {
                return MetricStatus::Available;
            }
            let statuses = markets
                .iter()
                .filter_map(|market| {
                    input
                        .benchmark_series
                        .iter()
                        .find(|series| series.market.trim().eq_ignore_ascii_case(market))
                        .map(|series| series.availability.status.clone())
                })
                .collect::<Vec<_>>();
            if statuses.len() != markets.len() {
                MetricStatus::Unavailable
            } else {
                merge_metric_statuses(&statuses)
            }
        }
    }
}

fn benchmark_returns_by_date(
    input: &ResultQualityInput,
    weights: &BTreeMap<String, f64>,
) -> BTreeMap<NaiveDate, Option<f64>> {
    let dates = input
        .actual_values
        .iter()
        .map(|point| point.date)
        .chain(input.baseline.iter().map(|point| point.date))
        .collect::<BTreeSet<_>>();
    dates
        .into_iter()
        .map(|date| {
            let result = match &input.benchmark_selection {
                BenchmarkSelection::SingleMarket(market) => input
                    .benchmark_series
                    .iter()
                    .find(|series| &series.market == market)
                    .and_then(|series| series_return_on(series, date)),
                BenchmarkSelection::AutomaticMixed => {
                    let mut total_return = 0.0;
                    let mut complete = !weights.is_empty();
                    for (market, weight) in weights
                        .iter()
                        .filter(|(market, _)| market.as_str() != "cash")
                    {
                        let value = input
                            .benchmark_series
                            .iter()
                            .find(|series| series.market.trim().eq_ignore_ascii_case(market))
                            .and_then(|series| series_return_on(series, date));
                        match value {
                            Some(value) => total_return += weight * value,
                            None => complete = false,
                        }
                    }
                    complete.then_some(total_return)
                }
            };
            (date, result)
        })
        .collect()
}

fn series_return_on(series: &BenchmarkSeriesInput, date: NaiveDate) -> Option<f64> {
    if series.availability.status == MetricStatus::Unavailable {
        return None;
    }
    let start = series.points.first()?;
    let current = series.points.iter().find(|point| point.date == date)?;
    (start.value.is_finite() && start.value > 0.0 && current.value.is_finite())
        .then_some(current.value / start.value - 1.0)
}

fn validated_actual_twr(
    values: &[PortfolioValuePoint],
    baseline: Option<&PortfolioValuePoint>,
    external_flows: &[ExternalFlowBase],
    input_availability: &MetricAvailability,
) -> Result<Vec<ReturnDataPoint>, String> {
    if input_availability.status == MetricStatus::Unavailable || values.is_empty() {
        return Err("Actual value history is unavailable.".to_string());
    }
    if values
        .iter()
        .any(|point| !point.value_base.is_finite() || point.value_base <= 0.0)
        || values.windows(2).any(|pair| pair[0].date >= pair[1].date)
    {
        return Err("Actual values must be positive, finite, and strictly ordered.".to_string());
    }
    if baseline.is_some_and(|point| {
        !point.value_base.is_finite() || point.value_base <= 0.0 || point.date >= values[0].date
    }) {
        return Err("The baseline must be positive, finite, and predate observations.".to_string());
    }
    if external_flows
        .iter()
        .any(|flow| !flow.amount_base.is_finite())
        || external_flows
            .windows(2)
            .any(|pair| pair[0].date > pair[1].date)
    {
        return Err("External flows must be finite and date-ordered.".to_string());
    }
    let origin_date = baseline.map(|point| point.date).unwrap_or(values[0].date);
    let end_date = values.last().unwrap().date;
    if external_flows
        .iter()
        .any(|flow| flow.date <= origin_date || flow.date > end_date)
    {
        return Err("External flows must fall inside the measured period.".to_string());
    }
    let daily_values = values
        .iter()
        .map(|point| (point.date, point.value_base, 0.0))
        .collect::<Vec<_>>();
    let flows = external_flows
        .iter()
        .map(|flow| (flow.date, flow.amount_base))
        .collect::<Vec<_>>();
    let series = build_twr_return_series(
        &daily_values,
        baseline.map(|point| (point.date, point.value_base)),
        &flows,
    );
    let generated_valid = series.len() == values.len()
        && series.iter().zip(values).all(|(point, source)| {
            point.date == source.date.format("%Y-%m-%d").to_string()
                && point.cumulative_return.is_finite()
                && point.daily_return.is_finite()
                && point.portfolio_value.is_finite()
                && point.daily_pnl.is_finite()
                && 1.0 + point.cumulative_return / 100.0 > 0.0
        });
    generated_valid
        .then_some(series)
        .ok_or_else(|| "The generated TWR curve is not finite and usable.".to_string())
}

fn normalized_review_curve(
    input: &ResultQualityInput,
    actual_twr_series: &[ReturnDataPoint],
    benchmark_by_date: &BTreeMap<NaiveDate, Option<f64>>,
) -> Vec<NormalizedReviewCurvePoint> {
    if actual_twr_series.is_empty() {
        return Vec::new();
    }
    let origin = input
        .baseline
        .as_ref()
        .map(|point| point.date)
        .unwrap_or(input.actual_values[0].date);
    let shadow_origin = input
        .shadow_curve
        .iter()
        .find(|point| point.date == origin)
        .filter(|point| point.cumulative_return.is_finite())
        .map(|point| 1.0 + point.cumulative_return)
        .filter(|value| value.is_finite() && *value > 0.0);
    let benchmark_origin = benchmark_by_date
        .get(&origin)
        .copied()
        .flatten()
        .map(|value| 1.0 + value)
        .filter(|value| value.is_finite() && *value > 0.0);
    let dates = std::iter::once(origin)
        .chain(input.actual_values.iter().map(|point| point.date))
        .collect::<BTreeSet<_>>();
    dates
        .into_iter()
        .map(|date| {
            let portfolio_index = if date == origin {
                Some(100.0)
            } else {
                actual_twr_series
                    .iter()
                    .find(|point| point.date == date.format("%Y-%m-%d").to_string())
                    .map(|point| 100.0 + point.cumulative_return)
            };
            let shadow_index = if date == origin {
                shadow_origin.map(|_| 100.0)
            } else {
                input
                    .shadow_curve
                    .iter()
                    .find(|point| point.date == date)
                    .filter(|point| point.cumulative_return.is_finite())
                    .map(|point| 1.0 + point.cumulative_return)
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .zip(shadow_origin)
                    .map(|(value, base)| 100.0 * value / base)
            };
            let benchmark_index = if date == origin {
                benchmark_origin.map(|_| 100.0)
            } else {
                benchmark_by_date
                    .get(&date)
                    .copied()
                    .flatten()
                    .map(|value| 1.0 + value)
                    .filter(|value| value.is_finite() && *value > 0.0)
                    .zip(benchmark_origin)
                    .map(|(value, base)| 100.0 * value / base)
            };
            NormalizedReviewCurvePoint {
                date,
                portfolio_index,
                shadow_index,
                benchmark_index,
            }
        })
        .collect()
}

pub fn calculate_max_drawdown_metric(
    values: &[PortfolioValuePoint],
    baseline: Option<PortfolioValuePoint>,
    external_flows: &[ExternalFlowBase],
    input_availability: MetricAvailability,
) -> MaxDrawdownMetric {
    let Ok(return_series) = validated_actual_twr(
        values,
        baseline.as_ref(),
        external_flows,
        &input_availability,
    ) else {
        return unavailable_drawdown();
    };
    drawdown_from_twr(&return_series, baseline.as_ref(), input_availability)
}

fn unavailable_drawdown() -> MaxDrawdownMetric {
    MaxDrawdownMetric {
        availability: unavailable("A complete cash-flow-adjusted return curve is required."),
        max_drawdown: None,
        peak_date: None,
        trough_date: None,
        duration_days: None,
        recovery_date: None,
        recovery_duration_days: None,
    }
}

fn drawdown_from_twr(
    return_series: &[ReturnDataPoint],
    baseline: Option<&PortfolioValuePoint>,
    input_availability: MetricAvailability,
) -> MaxDrawdownMetric {
    let mut parsed = return_series
        .iter()
        .filter_map(|point| {
            NaiveDate::parse_from_str(&point.date, "%Y-%m-%d")
                .ok()
                .map(|date| (date, 1.0 + point.cumulative_return / 100.0))
        })
        .collect::<Vec<_>>();
    if let Some(baseline) = baseline {
        parsed.insert(0, (baseline.date, 1.0));
    }
    if parsed.is_empty() {
        return unavailable_drawdown();
    }
    let mut running_peak = parsed[0];
    let mut worst = 0.0;
    let mut worst_peak = parsed[0].0;
    let mut trough = parsed[0].0;
    for &(date, nav) in &parsed {
        if nav > running_peak.1 {
            running_peak = (date, nav);
        }
        let drawdown = if running_peak.1 > 0.0 {
            nav / running_peak.1 - 1.0
        } else {
            0.0
        };
        if drawdown < worst {
            worst = drawdown;
            worst_peak = running_peak.0;
            trough = date;
        }
    }
    let peak_nav = parsed
        .iter()
        .find(|(date, _)| *date == worst_peak)
        .map(|(_, nav)| *nav)
        .unwrap_or(parsed[0].1);
    let recovery_date = (worst < -EPSILON)
        .then(|| {
            parsed
                .iter()
                .find(|(date, nav)| *date > trough && *nav + EPSILON >= peak_nav)
                .map(|(date, _)| *date)
        })
        .flatten();
    let end_date = parsed.last().map(|(date, _)| *date).unwrap_or(trough);
    let duration_days = if worst < -EPSILON {
        Some((recovery_date.unwrap_or(end_date) - worst_peak).num_days())
    } else {
        Some(0)
    };
    MaxDrawdownMetric {
        availability: input_availability,
        max_drawdown: Some(worst),
        peak_date: Some(worst_peak),
        trough_date: Some(trough),
        duration_days,
        recovery_date,
        recovery_duration_days: recovery_date.map(|date| (date - trough).num_days()),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RebalanceValueAddInput {
    pub actual_recorded_twr: Option<f64>,
    pub actual_comparable_total_return: Option<f64>,
    pub actual_comparable_price_return: Option<f64>,
    pub shadow_return: Option<f64>,
    pub shadow_return_mode: MarketReturnMode,
    pub actual_comparable_ending_value_base: Option<f64>,
    pub shadow_comparable_ending_value_base: Option<f64>,
    pub comparison_mode: MarketReturnMode,
    pub availability: MetricAvailability,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RebalanceValueAddOutput {
    pub metric: RebalanceValueAddMetric,
    pub actual_recorded_twr: Option<f64>,
    pub comparison_mode: MarketReturnMode,
    pub comparison_label: String,
}

pub fn calculate_rebalance_value_add(input: &RebalanceValueAddInput) -> RebalanceValueAddOutput {
    let actual_comparable = match input.comparison_mode {
        MarketReturnMode::TotalReturn => input.actual_comparable_total_return,
        MarketReturnMode::PriceOnly => input.actual_comparable_price_return,
    };
    let modes_match = input.comparison_mode == input.shadow_return_mode;
    let complete = modes_match
        && [
            actual_comparable,
            input.shadow_return,
            input.actual_comparable_ending_value_base,
            input.shadow_comparable_ending_value_base,
        ]
        .iter()
        .all(|value| value.is_some_and(f64::is_finite));
    let status = if !complete || input.availability.status == MetricStatus::Unavailable {
        MetricStatus::Unavailable
    } else if input.comparison_mode == MarketReturnMode::PriceOnly {
        MetricStatus::Degraded
    } else {
        input.availability.status.clone()
    };
    let precise = status != MetricStatus::Unavailable;
    let actual_return = precise.then_some(actual_comparable).flatten();
    let shadow_return = precise.then_some(input.shadow_return).flatten();
    let ending_difference = precise
        .then(|| {
            input
                .actual_comparable_ending_value_base
                .zip(input.shadow_comparable_ending_value_base)
                .map(|(actual, shadow)| actual - shadow)
        })
        .flatten();
    RebalanceValueAddOutput {
        metric: RebalanceValueAddMetric {
            availability: availability(
                status,
                if !modes_match {
                    Some(
                        "Actual and shadow comparison modes differ; value-add is unavailable."
                            .to_string(),
                    )
                } else if input.comparison_mode == MarketReturnMode::PriceOnly {
                    Some(
                        "Actual and shadow comparable curves both exclude dividends; recorded actual TWR is unchanged."
                            .to_string(),
                    )
                } else {
                    None
                },
            ),
            value_add: actual_return
                .zip(shadow_return)
                .map(|(actual, shadow)| actual - shadow),
            actual_return,
            shadow_return,
            ending_value_difference_base: ending_difference,
        },
        actual_recorded_twr: input.actual_recorded_twr,
        comparison_mode: input.comparison_mode.clone(),
        comparison_label: match input.comparison_mode {
            MarketReturnMode::TotalReturn => "rebalance_value_add",
            MarketReturnMode::PriceOnly => "rebalance_price_value_add",
        }
        .to_string(),
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LocalPricePoint {
    pub date: NaiveDate,
    pub close: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForwardActionInput {
    pub action_id: String,
    pub action_type: String,
    pub market: String,
    pub action_date: NaiveDate,
    pub action_notional_local: f64,
    pub action_day_fx_to_base: Option<f64>,
    #[serde(default)]
    pub market_session_dates: Vec<NaiveDate>,
    pub stock_prices_local: Vec<LocalPricePoint>,
    pub benchmark_prices_local: Vec<LocalPricePoint>,
    pub availability: MetricAvailability,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionEffectWindow {
    pub trading_days: u16,
    pub status: MetricAvailability,
    pub stock_return: Option<f64>,
    pub benchmark_return: Option<f64>,
    pub effect: Option<f64>,
    pub notional_base: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ActionForwardEffect {
    pub action_id: String,
    pub windows: Vec<ActionEffectWindow>,
    pub fact_labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ForwardEffectOutput {
    pub windows: Vec<ForwardEffectWindow>,
    pub actions: Vec<ActionForwardEffect>,
}

pub fn calculate_forward_effect(
    actions: &[ForwardActionInput],
    windows: &[u16],
) -> ForwardEffectOutput {
    let mut action_results = actions
        .iter()
        .map(|action| ActionForwardEffect {
            action_id: action.action_id.clone(),
            windows: windows
                .iter()
                .map(|window| calculate_action_window(action, *window))
                .collect(),
            fact_labels: Vec::new(),
        })
        .collect::<Vec<_>>();
    for (action, result) in actions.iter().zip(action_results.iter_mut()) {
        result.fact_labels = forward_fact_labels(action, &result.windows);
    }
    let aggregate_windows = windows
        .iter()
        .map(|window| {
            let observations = action_results
                .iter()
                .filter_map(|action| {
                    action
                        .windows
                        .iter()
                        .find(|value| value.trading_days == *window)
                })
                .collect::<Vec<_>>();
            let matured = observations
                .iter()
                .filter(|value| value.effect.is_some() && value.notional_base.is_some())
                .collect::<Vec<_>>();
            let pending_actions = observations
                .iter()
                .filter(|value| value.status.status == MetricStatus::Pending)
                .count();
            let invalid_actions = observations
                .iter()
                .filter(|value| value.status.status == MetricStatus::Unavailable)
                .count();
            let degraded_actions = observations
                .iter()
                .filter(|value| value.status.status == MetricStatus::Degraded)
                .count();
            let total_notional = matured
                .iter()
                .filter_map(|value| value.notional_base)
                .sum::<f64>();
            let weighted = (total_notional > 0.0).then(|| {
                matured
                    .iter()
                    .map(|value| value.effect.unwrap() * value.notional_base.unwrap())
                    .sum::<f64>()
                    / total_notional
            });
            let positive = (total_notional > 0.0).then(|| {
                matured
                    .iter()
                    .filter(|value| value.effect.unwrap() > 0.0)
                    .map(|value| value.notional_base.unwrap())
                    .sum::<f64>()
                    / total_notional
            });
            let status = if !matured.is_empty() {
                if invalid_actions > 0 || degraded_actions > 0 {
                    MetricStatus::Degraded
                } else {
                    MetricStatus::Available
                }
            } else if pending_actions > 0 && invalid_actions == 0 {
                MetricStatus::Pending
            } else {
                MetricStatus::Unavailable
            };
            ForwardEffectWindow {
                trading_days: *window,
                status: availability(status, None),
                matured_actions: matured.len(),
                pending_actions,
                amount_weighted_excess_return: weighted,
                positive_notional_ratio: positive,
            }
        })
        .collect();
    ForwardEffectOutput {
        windows: aggregate_windows,
        actions: action_results,
    }
}

fn calculate_action_window(action: &ForwardActionInput, window: u16) -> ActionEffectWindow {
    if action.availability.status == MetricStatus::Unavailable {
        return empty_action_window(
            window,
            MetricStatus::Unavailable,
            action.availability.note.clone(),
        );
    }
    let buy_direction = matches!(action.action_type.as_str(), "open" | "add");
    let sell_direction = matches!(action.action_type.as_str(), "reduce" | "close");
    if !buy_direction && !sell_direction {
        return empty_action_window(
            window,
            MetricStatus::Unavailable,
            Some("Action type is not evaluable as a stock trade.".to_string()),
        );
    }
    let fx = action
        .action_day_fx_to_base
        .filter(|value| value.is_finite() && *value > 0.0);
    let valid_notional =
        action.action_notional_local.is_finite() && action.action_notional_local.abs() > 0.0;
    let notional_base = fx.map(|value| action.action_notional_local.abs() * value);
    let start_point = action
        .stock_prices_local
        .iter()
        .find(|point| point.date == action.action_date);
    let benchmark_start = action
        .benchmark_prices_local
        .iter()
        .find(|point| point.date == action.action_date);
    let valid_start = start_point.is_some_and(|point| point.close.is_finite() && point.close > 0.0)
        && benchmark_start.is_some_and(|point| point.close.is_finite() && point.close > 0.0);
    let valid_calendar = !action.market_session_dates.is_empty()
        && action
            .market_session_dates
            .windows(2)
            .all(|pair| pair[0] < pair[1]);
    if !valid_notional
        || !notional_base.is_some_and(|value| value.is_finite() && value > 0.0)
        || !valid_start
        || !valid_calendar
    {
        return empty_action_window(
            window,
            MetricStatus::Unavailable,
            Some(
                "Action type, action-day prices, notional, FX, and market calendar must be valid."
                    .to_string(),
            ),
        );
    }
    let Some(target_date) = nth_market_session_after(
        &action.market_session_dates,
        action.action_date,
        window as usize,
    ) else {
        return empty_action_window(
            window,
            MetricStatus::Pending,
            Some("The market-session observation window has not matured.".to_string()),
        );
    };
    let Some(end_point) = action
        .stock_prices_local
        .iter()
        .find(|point| point.date == target_date)
    else {
        return empty_action_window(
            window,
            MetricStatus::Unavailable,
            Some("The exact target-session stock price is unavailable.".to_string()),
        );
    };
    let benchmark_end = action
        .benchmark_prices_local
        .iter()
        .find(|point| point.date == target_date);
    let valid_end = end_point.close.is_finite()
        && end_point.close > 0.0
        && benchmark_end.is_some_and(|point| point.close.is_finite() && point.close > 0.0);
    if !valid_end {
        return empty_action_window(
            window,
            MetricStatus::Unavailable,
            Some("An exact target-session stock or benchmark price is unavailable.".to_string()),
        );
    }
    let start_point = start_point.unwrap();
    let benchmark_start = benchmark_start.unwrap();
    let benchmark_end = benchmark_end.unwrap();
    let stock_return = end_point.close / start_point.close - 1.0;
    let benchmark_return = benchmark_end.close / benchmark_start.close - 1.0;
    let effect = if buy_direction {
        stock_return - benchmark_return
    } else {
        benchmark_return - stock_return
    };
    ActionEffectWindow {
        trading_days: window,
        status: availability(
            action.availability.status.clone(),
            action.availability.note.clone(),
        ),
        stock_return: Some(stock_return),
        benchmark_return: Some(benchmark_return),
        effect: Some(effect),
        notional_base,
    }
}

fn empty_action_window(
    trading_days: u16,
    status: MetricStatus,
    note: Option<String>,
) -> ActionEffectWindow {
    ActionEffectWindow {
        trading_days,
        status: availability(status, note),
        stock_return: None,
        benchmark_return: None,
        effect: None,
        notional_base: None,
    }
}

fn forward_fact_labels(action: &ForwardActionInput, windows: &[ActionEffectWindow]) -> Vec<String> {
    let mut labels = Vec::new();
    let sell = matches!(action.action_type.as_str(), "reduce" | "close");
    for window in windows {
        match (
            window.trading_days,
            window.status.status.clone(),
            window.effect,
        ) {
            (_, MetricStatus::Pending, _) => labels.push("observing".to_string()),
            (_, MetricStatus::Unavailable, _) => labels.push("data_insufficient".to_string()),
            (60, _, Some(effect)) if sell && effect > 0.0 => {
                labels.push("effective_avoidance".to_string())
            }
            (60, _, Some(effect)) if sell && effect < 0.0 => {
                labels.push("ex_post_opportunity_loss".to_string())
            }
            (60, _, Some(effect)) if effect > 0.0 => {
                labels.push("short_term_effective".to_string())
            }
            (60, _, Some(effect)) if effect < 0.0 => labels.push("short_term_adverse".to_string()),
            (120, _, Some(effect)) if effect > 0.0 => {
                labels.push("long_term_effective".to_string())
            }
            _ => {}
        }
    }
    labels.sort();
    labels.dedup();
    labels
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockValueBase {
    pub symbol: String,
    pub value_base: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskSnapshotInput {
    pub date: NaiveDate,
    pub stock_values_base: Vec<StockValueBase>,
    pub cash_value_base: Option<f64>,
    pub reliable: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StockChangeKind {
    Trade,
    Transfer,
    Split,
    NonTrade,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockChangeBase {
    pub notional_base: f64,
    pub kind: StockChangeKind,
}

impl StockChangeBase {
    pub fn trade(notional_base: f64) -> Self {
        Self {
            notional_base,
            kind: StockChangeKind::Trade,
        }
    }
    pub fn transfer(notional_base: f64) -> Self {
        Self {
            notional_base,
            kind: StockChangeKind::Transfer,
        }
    }
    pub fn split(notional_base: f64) -> Self {
        Self {
            notional_base,
            kind: StockChangeKind::Split,
        }
    }
    pub fn non_trade(notional_base: f64) -> Self {
        Self {
            notional_base,
            kind: StockChangeKind::NonTrade,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct RiskStructureInput {
    pub snapshots: Vec<RiskSnapshotInput>,
    pub stock_changes: Vec<StockChangeBase>,
    pub total_stock_trading_fees_base: Option<f64>,
    pub average_portfolio_nav_base: Option<f64>,
}

pub fn calculate_risk_structure(input: &RiskStructureInput) -> RiskStructureDetail {
    let opening = input
        .snapshots
        .first()
        .map(concentration)
        .unwrap_or_else(empty_concentration);
    let ending = input
        .snapshots
        .last()
        .map(concentration)
        .unwrap_or_else(empty_concentration);
    let all_reliable = !input.snapshots.is_empty() && input.snapshots.iter().all(valid_snapshot);
    let peak = if all_reliable {
        let points = input
            .snapshots
            .iter()
            .map(concentration)
            .collect::<Vec<_>>();
        ConcentrationSnapshot {
            date: None,
            max_stock_weight: points
                .iter()
                .filter_map(|point| point.max_stock_weight)
                .reduce(f64::max),
            cr5: points.iter().filter_map(|point| point.cr5).reduce(f64::max),
            hhi: points.iter().filter_map(|point| point.hhi).reduce(f64::max),
            cash_ratio: points
                .iter()
                .filter_map(|point| point.cash_ratio)
                .reduce(f64::max),
        }
    } else {
        empty_concentration()
    };
    let average_nav = input
        .average_portfolio_nav_base
        .filter(|value| value.is_finite() && *value > 0.0);
    let trade_inputs_valid = input
        .stock_changes
        .iter()
        .all(|change| change.notional_base.is_finite());
    let trade_notional = trade_inputs_valid.then(|| {
        input
            .stock_changes
            .iter()
            .filter(|change| change.kind == StockChangeKind::Trade)
            .map(|change| change.notional_base.abs())
            .sum::<f64>()
    });
    let one_way_turnover = trade_notional
        .zip(average_nav)
        .map(|(notional, nav)| notional / (2.0 * nav));
    let fee_drag = input
        .total_stock_trading_fees_base
        .filter(|value| value.is_finite() && *value >= 0.0)
        .zip(average_nav)
        .map(|(fees, nav)| fees / nav);
    let mut data_hints = Vec::new();
    if input.total_stock_trading_fees_base == Some(0.0) {
        data_hints.push("fees_may_be_incompletely_imported".to_string());
    }
    if !all_reliable {
        data_hints.push("holding_snapshots_incomplete".to_string());
    }
    let mut fact_labels = Vec::new();
    if opening
        .max_stock_weight
        .zip(ending.max_stock_weight)
        .is_some_and(|(start, end)| (end - start).abs() > 0.05)
    {
        fact_labels.push("concentration_changed_materially".to_string());
    }
    let concentration_status = if all_reliable {
        MetricStatus::Available
    } else {
        MetricStatus::Unavailable
    };
    let turnover_status = if one_way_turnover.is_some() {
        MetricStatus::Available
    } else {
        MetricStatus::Unavailable
    };
    let fee_status = if fee_drag.is_some() {
        MetricStatus::Available
    } else {
        MetricStatus::Unavailable
    };
    let status = if concentration_status == MetricStatus::Available
        && turnover_status == MetricStatus::Available
        && fee_status == MetricStatus::Available
    {
        MetricStatus::Available
    } else if concentration_status == MetricStatus::Available
        || turnover_status == MetricStatus::Available
        || fee_status == MetricStatus::Available
    {
        MetricStatus::Degraded
    } else {
        MetricStatus::Unavailable
    };
    RiskStructureDetail {
        availability: availability(status, None),
        concentration_availability: availability(concentration_status, None),
        turnover_availability: availability(turnover_status, None),
        fee_availability: availability(fee_status, None),
        opening,
        ending: ending.clone(),
        peak,
        one_way_turnover,
        fee_drag,
        data_hints,
        fact_labels,
        market_weights: vec![],
        category_weights: vec![],
        top_position_weights: vec![],
        concentration: ending.max_stock_weight,
        diversification_score: ending.hhi.map(|hhi| 1.0 - hhi),
    }
}

fn valid_snapshot(snapshot: &RiskSnapshotInput) -> bool {
    snapshot.reliable
        && snapshot.cash_value_base.is_some_and(f64::is_finite)
        && !snapshot.stock_values_base.is_empty()
        && snapshot
            .stock_values_base
            .iter()
            .all(|value| value.value_base.is_finite() && value.value_base >= 0.0)
        && snapshot
            .stock_values_base
            .iter()
            .map(|value| value.value_base)
            .sum::<f64>()
            > 0.0
}

fn concentration(snapshot: &RiskSnapshotInput) -> ConcentrationSnapshot {
    if !valid_snapshot(snapshot) {
        return ConcentrationSnapshot {
            date: Some(snapshot.date),
            max_stock_weight: None,
            cr5: None,
            hhi: None,
            cash_ratio: None,
        };
    }
    let mut values_by_symbol = BTreeMap::new();
    for value in &snapshot.stock_values_base {
        *values_by_symbol
            .entry(value.symbol.trim().to_ascii_uppercase())
            .or_insert(0.0) += value.value_base;
    }
    let stock_total = values_by_symbol.values().sum::<f64>();
    let mut weights = values_by_symbol
        .values()
        .map(|value| value / stock_total)
        .collect::<Vec<_>>();
    weights.sort_by(|left, right| right.partial_cmp(left).unwrap_or(std::cmp::Ordering::Equal));
    let cash = snapshot.cash_value_base.unwrap();
    ConcentrationSnapshot {
        date: Some(snapshot.date),
        max_stock_weight: weights.first().copied(),
        cr5: Some(weights.iter().take(5).sum()),
        hhi: Some(weights.iter().map(|weight| weight * weight).sum()),
        cash_ratio: (stock_total + cash > 0.0).then_some(cash / (stock_total + cash)),
    }
}

fn empty_concentration() -> ConcentrationSnapshot {
    ConcentrationSnapshot {
        date: None,
        max_stock_weight: None,
        cr5: None,
        hhi: None,
        cash_ratio: None,
    }
}

pub type CampaignCashFlow = CampaignTimelineItem;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CampaignPricePoint {
    pub date: NaiveDate,
    pub currency: String,
    pub low: Option<f64>,
    pub high: Option<f64>,
    pub close: Option<f64>,
    pub fx_to_base: Option<f64>,
}

impl CampaignPricePoint {
    pub fn complete(date: NaiveDate, low: f64, high: f64, close: f64) -> Self {
        Self {
            date,
            currency: "BASE".to_string(),
            low: Some(low),
            high: Some(high),
            close: Some(close),
            fx_to_base: Some(1.0),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CampaignDetailInput {
    pub summary: StockCampaignSummary,
    pub cash_flows: Vec<CampaignCashFlow>,
    pub daily_prices: Vec<CampaignPricePoint>,
    pub benchmark_prices: Vec<LocalPricePoint>,
    pub current_price_local: Option<f64>,
    pub current_fx_to_base: Option<f64>,
    pub actions: Vec<StockActionReview>,
    pub forward_actions: Vec<ForwardActionInput>,
    pub annotations: Vec<StockReviewAnnotation>,
}

pub fn calculate_campaign_detail(input: &CampaignDetailInput) -> StockCampaignDetail {
    let mut timeline = input.cash_flows.clone();
    timeline.sort_by_key(|flow| flow.date);
    let buy_outlays = timeline
        .iter()
        .filter(|flow| flow.kind == CampaignCashFlowKind::Buy)
        .map(|flow| flow.amount_base)
        .sum::<f64>();
    let sell_proceeds = timeline
        .iter()
        .filter(|flow| flow.kind == CampaignCashFlowKind::Sell)
        .map(|flow| flow.amount_base)
        .sum::<f64>();
    let dividends = timeline
        .iter()
        .filter(|flow| flow.kind == CampaignCashFlowKind::Dividend)
        .map(|flow| flow.amount_base)
        .sum::<f64>();
    let fees = timeline
        .iter()
        .filter(|flow| flow.kind == CampaignCashFlowKind::Fee)
        .map(|flow| flow.amount_base)
        .sum::<f64>();
    let bought_shares = timeline
        .iter()
        .filter(|flow| flow.kind == CampaignCashFlowKind::Buy)
        .map(|flow| flow.shares)
        .sum::<f64>();
    let sold_shares = timeline
        .iter()
        .filter(|flow| flow.kind == CampaignCashFlowKind::Sell)
        .map(|flow| flow.shares)
        .sum::<f64>();
    let remaining_shares = bought_shares - sold_shares;
    let mut net_invested = 0.0;
    let mut max_invested: f64 = 0.0;
    for flow in &timeline {
        match flow.kind {
            CampaignCashFlowKind::Buy | CampaignCashFlowKind::Fee => {
                net_invested += flow.amount_base
            }
            CampaignCashFlowKind::Sell | CampaignCashFlowKind::Dividend => {
                net_invested -= flow.amount_base
            }
        }
        max_invested = max_invested.max(net_invested);
    }
    let max_invested = (max_invested > 0.0 && max_invested.is_finite()).then_some(max_invested);
    let remaining_market_value = if input.summary.campaign_status == StockCampaignStatus::Active {
        input
            .current_price_local
            .filter(|price| price.is_finite() && *price >= 0.0)
            .zip(
                input
                    .current_fx_to_base
                    .filter(|fx| fx.is_finite() && *fx > 0.0),
            )
            .map(|(price, fx)| remaining_shares * price * fx)
    } else {
        Some(0.0)
    };
    let total_pnl = remaining_market_value
        .map(|remaining| sell_proceeds + dividends + remaining - buy_outlays - fees);
    let pnl = CampaignPnl {
        buy_outlays_base: buy_outlays,
        sell_proceeds_base: sell_proceeds,
        dividends_base: dividends,
        trading_fees_base: fees,
        remaining_shares,
        remaining_market_value_base: remaining_market_value,
        total_pnl_base: total_pnl,
        max_invested_capital_base: max_invested,
        label: if input.summary.campaign_status == StockCampaignStatus::Completed {
            "completed_net_pnl"
        } else {
            "active_total_pnl_including_remaining_value"
        }
        .to_string(),
    };
    let excursions = campaign_excursions(&timeline, &input.daily_prices);
    let mae_base = excursions.mae_base;
    let mfe_base = excursions.mfe_base;
    let mae_percent = mae_base
        .zip(max_invested)
        .map(|(amount, capital)| amount / capital);
    let mfe_percent = mfe_base
        .zip(max_invested)
        .map(|(amount, capital)| amount / capital);
    let campaign_return = total_pnl
        .zip(max_invested)
        .map(|(amount, capital)| amount / capital);
    let benchmark_return = input
        .benchmark_prices
        .first()
        .zip(input.benchmark_prices.last())
        .filter(|(start, end)| {
            start.close.is_finite() && start.close > 0.0 && end.close.is_finite()
        })
        .map(|(start, end)| end.close / start.close - 1.0);
    let pnl_status =
        if total_pnl.is_none() || input.summary.availability.status == MetricStatus::Unavailable {
            MetricStatus::Unavailable
        } else {
            input.summary.availability.status.clone()
        };
    let excursion_status = if max_invested.is_none()
        || (input.daily_prices.is_empty() && mae_base.is_none() && mfe_base.is_none())
    {
        MetricStatus::Unavailable
    } else if !excursions.path_valid
        || !excursions.intraday_complete
        || !excursions.close_complete
        || mae_base.is_none()
        || mfe_base.is_none()
    {
        MetricStatus::Degraded
    } else {
        MetricStatus::Available
    };
    let holding_period_drawdown = if excursions.path_valid && excursions.close_complete {
        campaign_drawdown(&excursions.close_path, max_invested)
    } else {
        None
    };
    let drawdown_status = if max_invested.is_none() || input.daily_prices.is_empty() {
        MetricStatus::Unavailable
    } else if holding_period_drawdown.is_some() {
        MetricStatus::Available
    } else {
        MetricStatus::Degraded
    };
    let benchmark_status = if benchmark_return.is_some() {
        MetricStatus::Available
    } else {
        MetricStatus::Unavailable
    };
    let forward = calculate_forward_effect(&input.forward_actions, &[20, 60, 120]);
    let mut actions = input.actions.clone();
    for action in &mut actions {
        let Some(effect) = forward
            .actions
            .iter()
            .find(|effect| effect.action_id == action.action_id)
        else {
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
                positive_notional_ratio: window.effect.map(|value| f64::from(value > 0.0)),
            })
            .collect();
        action.fact_labels.extend(effect.fact_labels.clone());
        action.fact_labels.sort();
        action.fact_labels.dedup();
    }
    let empty_window = |days| ForwardEffectWindow {
        trading_days: days,
        status: availability(
            MetricStatus::Unavailable,
            Some("No evaluable actions are available.".to_string()),
        ),
        matured_actions: 0,
        pending_actions: 0,
        amount_weighted_excess_return: None,
        positive_notional_ratio: None,
    };
    let window = |days| {
        forward
            .windows
            .iter()
            .find(|value| value.trading_days == days)
            .cloned()
            .unwrap_or_else(|| empty_window(days))
    };
    let mut fact_labels = forward
        .actions
        .iter()
        .flat_map(|action| action.fact_labels.clone())
        .collect::<Vec<_>>();
    fact_labels.push(
        match input.summary.campaign_status {
            StockCampaignStatus::Active => "campaign_active",
            StockCampaignStatus::Completed => "campaign_completed",
        }
        .to_string(),
    );
    let availability_status = if pnl_status == MetricStatus::Unavailable {
        MetricStatus::Unavailable
    } else if pnl_status != MetricStatus::Available
        || excursion_status != MetricStatus::Available
        || drawdown_status != MetricStatus::Available
        || benchmark_status != MetricStatus::Available
    {
        MetricStatus::Degraded
    } else {
        MetricStatus::Available
    };
    if excursion_status != MetricStatus::Available
        || benchmark_status != MetricStatus::Available
        || pnl_status != MetricStatus::Available
    {
        fact_labels.push("data_insufficient".to_string());
    }
    fact_labels.sort();
    fact_labels.dedup();
    StockCampaignDetail {
        availability: availability(availability_status, None),
        pnl_availability: availability(
            pnl_status,
            total_pnl
                .is_none()
                .then(|| "Active Campaign current value is unavailable.".to_string()),
        ),
        excursion_availability: availability(
            excursion_status.clone(),
            (!excursions.path_valid
                || !excursions.intraday_complete
                || !excursions.close_complete)
                .then(|| {
                    "Daily OHLC/FX coverage is invalid or incomplete; partial paths are not treated as complete."
                        .to_string()
                }),
        ),
        drawdown_availability: availability(
            drawdown_status,
            holding_period_drawdown
                .is_none()
                .then(|| "A complete valid daily close path is required for drawdown.".to_string()),
        ),
        benchmark_availability: availability(
            benchmark_status,
            benchmark_return
                .is_none()
                .then(|| "Holding-period benchmark return is unavailable.".to_string()),
        ),
        summary: input.summary.clone(),
        actions,
        forward_effect_20d: window(20),
        forward_effect_60d: window(60),
        forward_effect_120d: window(120),
        pnl,
        campaign_return,
        benchmark_return,
        excess_return: campaign_return
            .zip(benchmark_return)
            .map(|(campaign, benchmark)| campaign - benchmark),
        mae_base,
        mfe_base,
        mae_percent,
        mfe_percent,
        holding_period_drawdown,
        timeline,
        fact_labels,
        completed_sample_count: usize::from(
            input.summary.campaign_status == StockCampaignStatus::Completed,
        ),
        active_sample_count: usize::from(
            input.summary.campaign_status == StockCampaignStatus::Active,
        ),
        annotations: input.annotations.clone(),
    }
}

struct CampaignExcursionEvaluation {
    mae_base: Option<f64>,
    mfe_base: Option<f64>,
    close_path: Vec<f64>,
    intraday_complete: bool,
    close_complete: bool,
    path_valid: bool,
}

fn campaign_excursions(
    flows: &[CampaignCashFlow],
    prices: &[CampaignPricePoint],
) -> CampaignExcursionEvaluation {
    let mut cash = 0.0;
    let mut shares = 0.0;
    let mut applied = 0usize;
    let mut adverse: Option<f64> = None;
    let mut favorable: Option<f64> = None;
    let mut close_path = Vec::new();
    let mut intraday_complete = !prices.is_empty();
    let mut close_complete = !prices.is_empty();
    let mut path_valid = true;
    let mut ordered_prices = prices.to_vec();
    ordered_prices.sort_by_key(|point| point.date);
    for price in &ordered_prices {
        while applied < flows.len() && flows[applied].date <= price.date {
            let flow = &flows[applied];
            match flow.kind {
                CampaignCashFlowKind::Buy => {
                    cash -= flow.amount_base;
                    shares += flow.shares;
                }
                CampaignCashFlowKind::Sell => {
                    cash += flow.amount_base;
                    shares -= flow.shares;
                }
                CampaignCashFlowKind::Dividend => cash += flow.amount_base,
                CampaignCashFlowKind::Fee => cash -= flow.amount_base,
            }
            applied += 1;
        }
        let Some(fx) = price.fx_to_base else {
            intraday_complete = false;
            close_complete = false;
            continue;
        };
        if !fx.is_finite() || fx <= 0.0 {
            path_valid = false;
            break;
        }
        let low = match price.low {
            Some(value) if value.is_finite() && value >= 0.0 => Some(value),
            Some(_) => {
                path_valid = false;
                break;
            }
            None => None,
        };
        let high = match price.high {
            Some(value) if value.is_finite() && value >= 0.0 => Some(value),
            Some(_) => {
                path_valid = false;
                break;
            }
            None => None,
        };
        if low.zip(high).is_some_and(|(low, high)| low > high) {
            path_valid = false;
            break;
        }
        match low {
            Some(low) => {
                let amount = cash + shares * low * fx;
                adverse = Some(adverse.map_or(amount, |value| value.min(amount)))
            }
            None => intraday_complete = false,
        }
        match high {
            Some(high) => {
                let amount = cash + shares * high * fx;
                favorable = Some(favorable.map_or(amount, |value| value.max(amount)))
            }
            None => intraday_complete = false,
        }
        match price.close {
            Some(close) if close.is_finite() && close >= 0.0 => {
                close_path.push(cash + shares * close * fx)
            }
            Some(_) => {
                path_valid = false;
                break;
            }
            None => close_complete = false,
        }
    }
    if !path_valid {
        adverse = None;
        favorable = None;
        close_path.clear();
        intraday_complete = false;
        close_complete = false;
    }
    CampaignExcursionEvaluation {
        mae_base: adverse,
        mfe_base: favorable,
        close_path,
        intraday_complete,
        close_complete,
        path_valid,
    }
}

fn campaign_drawdown(pnl_path: &[f64], capital: Option<f64>) -> Option<f64> {
    let capital = capital?;
    if pnl_path.is_empty() {
        return None;
    }
    let mut peak = capital;
    let mut worst = 0.0;
    for pnl in pnl_path {
        let value = capital + pnl;
        if value > peak {
            peak = value;
        }
        if peak > 0.0 {
            worst = f64::min(worst, value / peak - 1.0);
        }
    }
    Some(worst)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CampaignAggregate {
    pub completed_sample_count: usize,
    pub active_sample_count: usize,
    pub average_completed_net_pnl_base: Option<f64>,
    pub completed_ranking: Vec<String>,
}

pub fn calculate_campaign_aggregates(details: &[StockCampaignDetail]) -> CampaignAggregate {
    let mut completed = details
        .iter()
        .filter(|detail| detail.summary.campaign_status == StockCampaignStatus::Completed)
        .filter_map(|detail| {
            detail
                .pnl
                .total_pnl_base
                .map(|pnl| (detail.summary.campaign_id.clone(), pnl))
        })
        .collect::<Vec<_>>();
    completed.sort_by(|left, right| {
        right
            .1
            .partial_cmp(&left.1)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.0.cmp(&right.0))
    });
    let completed_sample_count = details
        .iter()
        .filter(|detail| detail.summary.campaign_status == StockCampaignStatus::Completed)
        .count();
    let active_sample_count = details
        .iter()
        .filter(|detail| detail.summary.campaign_status == StockCampaignStatus::Active)
        .count();
    let average_completed_net_pnl_base = (!completed.is_empty())
        .then(|| completed.iter().map(|(_, pnl)| *pnl).sum::<f64>() / completed.len() as f64);
    CampaignAggregate {
        completed_sample_count,
        active_sample_count,
        average_completed_net_pnl_base,
        completed_ranking: completed.into_iter().map(|(id, _)| id).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::stock_review::{
        AccountCampaignFragment, MetricAvailability, MetricStatus, StockActionReview,
        StockCampaignStatus, StockCampaignSummary,
    };
    use crate::services::stock_review_market_data::MarketReturnMode;
    use chrono::{Duration, NaiveDate};

    fn date(day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 1, day).unwrap()
    }

    fn available() -> MetricAvailability {
        MetricAvailability {
            status: MetricStatus::Available,
            note: None,
        }
    }

    fn benchmark(market: &str, start: f64, end: f64) -> BenchmarkSeriesInput {
        BenchmarkSeriesInput {
            market: market.to_string(),
            availability: available(),
            points: vec![
                BenchmarkPoint {
                    date: date(1),
                    value: start,
                },
                BenchmarkPoint {
                    date: date(2),
                    value: end,
                },
            ],
        }
    }

    fn result_input(selection: BenchmarkSelection) -> ResultQualityInput {
        ResultQualityInput {
            actual_values: vec![
                PortfolioValuePoint {
                    date: date(1),
                    value_base: 100.0,
                },
                PortfolioValuePoint {
                    date: date(2),
                    value_base: 200.0,
                },
            ],
            baseline: None,
            external_flows_base: vec![],
            actual_availability: available(),
            opening_market_values_base: vec![
                MarketValue {
                    market: "US".to_string(),
                    value_base: 60.0,
                },
                MarketValue {
                    market: "CN".to_string(),
                    value_base: 30.0,
                },
            ],
            opening_cash_value_base: 10.0,
            benchmark_series: vec![benchmark("US", 100.0, 110.0), benchmark("CN", 100.0, 90.0)],
            benchmark_selection: selection,
            shadow_curve: vec![
                CurveReturnPoint {
                    date: date(1),
                    cumulative_return: 0.0,
                },
                CurveReturnPoint {
                    date: date(2),
                    cumulative_return: 0.05,
                },
            ],
        }
    }

    #[test]
    fn uses_fixed_start_weights_for_mixed_benchmark() {
        // Reweighting the benchmark from the actual portfolio's 100% period
        // gain would break the literal 60/30/10 opening-weight result.
        let mixed = calculate_result_quality(&result_input(BenchmarkSelection::AutomaticMixed));
        assert_eq!(mixed.metric.availability.status, MetricStatus::Available);
        assert!((mixed.metric.benchmark_return.unwrap() - 0.03).abs() < 1e-12);
        assert!((mixed.metric.portfolio_return.unwrap() - 1.0).abs() < 1e-12);
        assert!((mixed.metric.excess_return.unwrap() - 0.97).abs() < 1e-12);
        assert_eq!(mixed.fixed_weights["US"], 0.6);
        assert_eq!(mixed.fixed_weights["CN"], 0.3);
        assert_eq!(mixed.fixed_weights["cash"], 0.1);

        let single = calculate_result_quality(&result_input(BenchmarkSelection::SingleMarket(
            "US".to_string(),
        )));
        assert!((single.metric.benchmark_return.unwrap() - 0.10).abs() < 1e-12);
    }

    #[test]
    fn actual_result_reuses_cash_flow_adjusted_twr_and_normalizes_curves() {
        // Treating a contribution as return would report 60%, not the
        // authority function's 10%; normalized indices must start at 100.
        let mut input = result_input(BenchmarkSelection::SingleMarket("US".to_string()));
        input.actual_values[1].value_base = 160.0;
        input.external_flows_base.push(ExternalFlowBase {
            date: date(2),
            amount_base: 50.0,
        });
        let result = calculate_result_quality(&input);
        assert!((result.metric.portfolio_return.unwrap() - 0.10).abs() < 1e-12);
        assert!((result.metric.excess_return.unwrap()).abs() < 1e-12);
        assert_eq!(result.normalized_curve[0].portfolio_index, Some(100.0));
        assert_eq!(result.normalized_curve[0].shadow_index, Some(100.0));
        assert_eq!(result.normalized_curve[0].benchmark_index, Some(100.0));
        assert!((result.normalized_curve[1].portfolio_index.unwrap() - 110.0).abs() < 1e-12);
        assert!((result.normalized_curve[1].shadow_index.unwrap() - 105.0).abs() < 1e-12);
    }

    #[test]
    fn invalid_actual_inputs_clear_twr_and_every_drawdown_field() {
        let mut invalid_baseline = result_input(BenchmarkSelection::SingleMarket("US".to_string()));
        invalid_baseline.baseline = Some(PortfolioValuePoint {
            date: date(1),
            value_base: 0.0,
        });
        invalid_baseline.actual_values[0].date = date(2);
        invalid_baseline.actual_values[1].date = date(3);
        let mut invalid_value = result_input(BenchmarkSelection::SingleMarket("US".to_string()));
        invalid_value.actual_values[1].value_base = f64::NAN;
        for input in [invalid_baseline, invalid_value] {
            let result = calculate_result_quality(&input);
            assert_eq!(result.metric.portfolio_return, None);
            assert!(result.actual_twr_series.is_empty());
            assert_eq!(result.max_drawdown.max_drawdown, None);
            assert_eq!(result.max_drawdown.peak_date, None);
            assert_eq!(result.max_drawdown.trough_date, None);
            assert_eq!(result.max_drawdown.duration_days, None);
            assert_eq!(result.max_drawdown.recovery_date, None);
            assert_eq!(result.max_drawdown.recovery_duration_days, None);
        }

        let mut invalid_flow = result_input(BenchmarkSelection::SingleMarket("US".to_string()));
        invalid_flow.external_flows_base.push(ExternalFlowBase {
            date: date(2),
            amount_base: f64::NAN,
        });
        let result = calculate_result_quality(&invalid_flow);
        assert_eq!(result.metric.portfolio_return, None);
        assert_eq!(result.max_drawdown.max_drawdown, None);
    }

    #[test]
    fn missing_benchmark_preserves_valid_actual_twr() {
        let mut input = result_input(BenchmarkSelection::SingleMarket("US".to_string()));
        input.benchmark_series.clear();
        let result = calculate_result_quality(&input);
        assert_eq!(result.metric.availability.status, MetricStatus::Degraded);
        assert_eq!(result.metric.portfolio_return, Some(1.0));
        assert_eq!(result.metric.benchmark_return, None);
        assert_eq!(result.metric.excess_return, None);
        assert_eq!(
            result.max_drawdown.availability.status,
            MetricStatus::Available
        );
        assert!(result
            .metric
            .availability
            .note
            .as_deref()
            .is_some_and(|note| note.contains("benchmark")));
    }

    #[test]
    fn benchmark_openings_ignore_zero_components_support_cash_only_and_reject_malformed() {
        let mut zero_component = result_input(BenchmarkSelection::AutomaticMixed);
        zero_component.opening_market_values_base[1].value_base = 0.0;
        zero_component
            .benchmark_series
            .retain(|series| series.market == "US");
        let result = calculate_result_quality(&zero_component);
        assert_eq!(result.metric.availability.status, MetricStatus::Available);
        assert!(!result.fixed_weights.contains_key("CN"));
        assert!((result.fixed_weights["US"] - 6.0 / 7.0).abs() < 1e-12);
        assert!((result.metric.benchmark_return.unwrap() - 3.0 / 35.0).abs() < 1e-12);

        let mut cash_only = result_input(BenchmarkSelection::AutomaticMixed);
        cash_only.opening_market_values_base.clear();
        cash_only.opening_cash_value_base = 100.0;
        cash_only.benchmark_series.clear();
        let result = calculate_result_quality(&cash_only);
        assert_eq!(result.metric.availability.status, MetricStatus::Available);
        assert_eq!(result.fixed_weights.get("cash"), Some(&1.0));
        assert_eq!(result.metric.benchmark_return, Some(0.0));

        let mut malformed = result_input(BenchmarkSelection::AutomaticMixed);
        malformed.opening_market_values_base[0].value_base = -1.0;
        let result = calculate_result_quality(&malformed);
        assert!(result.fixed_weights.is_empty());
        assert_eq!(result.metric.portfolio_return, Some(1.0));
        assert_eq!(result.metric.benchmark_return, None);
        assert_eq!(result.metric.availability.status, MetricStatus::Degraded);
    }

    #[test]
    fn normalized_curves_share_an_explicit_baseline_origin_without_changing_twr() {
        let mut input = result_input(BenchmarkSelection::SingleMarket("US".to_string()));
        input.baseline = Some(PortfolioValuePoint {
            date: date(1),
            value_base: 100.0,
        });
        input.actual_values = vec![
            PortfolioValuePoint {
                date: date(2),
                value_base: 110.0,
            },
            PortfolioValuePoint {
                date: date(3),
                value_base: 121.0,
            },
        ];
        input.benchmark_series[0].points.push(BenchmarkPoint {
            date: date(3),
            value: 121.0,
        });
        input.shadow_curve = vec![
            CurveReturnPoint {
                date: date(1),
                cumulative_return: 0.20,
            },
            CurveReturnPoint {
                date: date(2),
                cumulative_return: 0.32,
            },
            CurveReturnPoint {
                date: date(3),
                cumulative_return: 0.44,
            },
        ];
        let result = calculate_result_quality(&input);
        assert!((result.metric.portfolio_return.unwrap() - 0.21).abs() < 1e-12);
        assert_eq!(result.normalized_curve[0].date, date(1));
        assert_eq!(result.normalized_curve[0].portfolio_index, Some(100.0));
        assert_eq!(result.normalized_curve[0].shadow_index, Some(100.0));
        assert_eq!(result.normalized_curve[0].benchmark_index, Some(100.0));
        assert!((result.normalized_curve[1].shadow_index.unwrap() - 110.0).abs() < 1e-12);
    }

    #[test]
    fn drawdown_reports_peak_trough_duration_and_recovery() {
        let values = [100.0, 120.0, 90.0, 110.0, 121.0]
            .into_iter()
            .enumerate()
            .map(|(index, value)| PortfolioValuePoint {
                date: date(index as u32 + 1),
                value_base: value,
            })
            .collect::<Vec<_>>();
        let recovered = calculate_max_drawdown_metric(&values, None, &[], available());
        assert!((recovered.max_drawdown.unwrap() + 0.25).abs() < 1e-12);
        assert_eq!(recovered.peak_date, Some(date(2)));
        assert_eq!(recovered.trough_date, Some(date(3)));
        assert_eq!(recovered.duration_days, Some(3));
        assert_eq!(recovered.recovery_date, Some(date(5)));
        assert_eq!(recovered.recovery_duration_days, Some(2));

        let unrecovered = calculate_max_drawdown_metric(&values[..4], None, &[], available());
        assert_eq!(unrecovered.recovery_date, None);
        assert_eq!(unrecovered.recovery_duration_days, None);
        assert_eq!(unrecovered.duration_days, Some(2));
    }

    #[test]
    fn drawdown_includes_loss_from_the_pre_window_baseline() {
        let baseline = PortfolioValuePoint {
            date: date(1),
            value_base: 100.0,
        };
        let values = vec![
            PortfolioValuePoint {
                date: date(2),
                value_base: 80.0,
            },
            PortfolioValuePoint {
                date: date(3),
                value_base: 90.0,
            },
        ];
        let result = calculate_max_drawdown_metric(&values, Some(baseline), &[], available());
        assert!((result.max_drawdown.unwrap() + 0.20).abs() < 1e-12);
        assert_eq!(result.peak_date, Some(date(1)));
        assert_eq!(result.trough_date, Some(date(2)));
        assert_eq!(result.recovery_date, None);
        assert_eq!(result.duration_days, Some(2));
    }

    #[test]
    fn monotonic_curve_has_no_recovery_episode_or_duration() {
        let values = [100.0, 110.0, 120.0]
            .into_iter()
            .enumerate()
            .map(|(index, value)| PortfolioValuePoint {
                date: date(index as u32 + 1),
                value_base: value,
            })
            .collect::<Vec<_>>();
        let result = calculate_max_drawdown_metric(&values, None, &[], available());
        assert_eq!(result.max_drawdown, Some(0.0));
        assert_eq!(result.duration_days, Some(0));
        assert_eq!(result.recovery_date, None);
        assert_eq!(result.recovery_duration_days, None);
    }

    #[test]
    fn price_only_value_add_compares_compatible_curves_without_replacing_actual_twr() {
        let result = calculate_rebalance_value_add(&RebalanceValueAddInput {
            actual_recorded_twr: Some(0.12),
            actual_comparable_total_return: Some(0.12),
            actual_comparable_price_return: Some(0.08),
            shadow_return: Some(0.05),
            shadow_return_mode: MarketReturnMode::PriceOnly,
            actual_comparable_ending_value_base: Some(1_080.0),
            shadow_comparable_ending_value_base: Some(1_050.0),
            comparison_mode: MarketReturnMode::PriceOnly,
            availability: available(),
        });
        assert_eq!(result.metric.availability.status, MetricStatus::Degraded);
        assert_eq!(result.actual_recorded_twr, Some(0.12));
        assert_eq!(result.metric.actual_return, Some(0.08));
        assert!((result.metric.value_add.unwrap() - 0.03).abs() < 1e-12);
        assert_eq!(result.metric.ending_value_difference_base, Some(30.0));
        assert_eq!(result.comparison_label, "rebalance_price_value_add");
    }

    #[test]
    fn rebalance_value_add_rejects_mismatched_shadow_return_mode() {
        let result = calculate_rebalance_value_add(&RebalanceValueAddInput {
            actual_recorded_twr: Some(0.12),
            actual_comparable_total_return: Some(0.12),
            actual_comparable_price_return: Some(0.08),
            shadow_return: Some(0.05),
            shadow_return_mode: MarketReturnMode::TotalReturn,
            actual_comparable_ending_value_base: Some(1_080.0),
            shadow_comparable_ending_value_base: Some(1_050.0),
            comparison_mode: MarketReturnMode::PriceOnly,
            availability: available(),
        });
        assert_eq!(result.metric.availability.status, MetricStatus::Unavailable);
        assert_eq!(result.metric.value_add, None);
        assert_eq!(result.metric.actual_return, None);
        assert_eq!(result.metric.shadow_return, None);
        assert_eq!(result.metric.ending_value_difference_base, None);
    }

    fn sessions(count: usize, step_days: i64, start: f64, end: f64) -> Vec<LocalPricePoint> {
        (0..count)
            .map(|index| LocalPricePoint {
                date: date(1) + Duration::days(index as i64 * step_days),
                close: start + (end - start) * index as f64 / (count - 1) as f64,
            })
            .collect()
    }

    fn forward_action(
        id: &str,
        action_type: &str,
        market: &str,
        stock_end: f64,
        benchmark_end: f64,
        notional: f64,
        fx: f64,
        session_count: usize,
        step_days: i64,
    ) -> ForwardActionInput {
        ForwardActionInput {
            action_id: id.to_string(),
            action_type: action_type.to_string(),
            market: market.to_string(),
            action_date: date(1),
            action_notional_local: notional,
            action_day_fx_to_base: Some(fx),
            market_session_dates: sessions(session_count, step_days, 100.0, stock_end)
                .into_iter()
                .map(|point| point.date)
                .collect(),
            stock_prices_local: sessions(session_count, step_days, 100.0, stock_end),
            benchmark_prices_local: sessions(session_count, step_days, 100.0, benchmark_end),
            availability: available(),
        }
    }

    #[test]
    fn forward_effect_uses_direction_local_returns_action_day_notional_and_market_sessions() {
        let actions = vec![
            forward_action("buy", "open", "US", 120.0, 110.0, 100.0, 2.0, 121, 1),
            forward_action("sell", "close", "CN", 90.0, 100.0, 300.0, 1.0, 121, 2),
            forward_action("pending", "add", "HK", 110.0, 100.0, 500.0, 3.0, 30, 3),
        ];
        let result = calculate_forward_effect(&actions, &[60, 120]);
        let day_60 = result
            .windows
            .iter()
            .find(|window| window.trading_days == 60)
            .unwrap();
        assert_eq!(day_60.status.status, MetricStatus::Available);
        assert_eq!(day_60.matured_actions, 2);
        assert_eq!(day_60.pending_actions, 1);
        assert!((day_60.amount_weighted_excess_return.unwrap() - 0.05).abs() < 1e-12);
        assert_eq!(day_60.positive_notional_ratio, Some(1.0));
        let day_120 = result
            .windows
            .iter()
            .find(|window| window.trading_days == 120)
            .unwrap();
        assert_eq!(day_120.matured_actions, 2);
        assert_eq!(day_120.pending_actions, 1);
        assert!((day_120.amount_weighted_excess_return.unwrap() - 0.10).abs() < 1e-12);
        assert!(result.actions[0]
            .fact_labels
            .contains(&"short_term_effective".to_string()));
        assert!(result.actions[0]
            .fact_labels
            .contains(&"long_term_effective".to_string()));
        assert!(result.actions[1]
            .fact_labels
            .contains(&"effective_avoidance".to_string()));
        assert!(result.actions[2]
            .fact_labels
            .contains(&"observing".to_string()));
    }

    #[test]
    fn sell_after_rise_is_labeled_opportunity_loss_not_a_judgment() {
        let result = calculate_forward_effect(
            &[forward_action(
                "sell", "reduce", "US", 120.0, 100.0, 100.0, 1.0, 61, 1,
            )],
            &[60],
        );
        let action = &result.actions[0];
        assert!(action
            .fact_labels
            .contains(&"ex_post_opportunity_loss".to_string()));
        assert!(!action
            .fact_labels
            .iter()
            .any(|label| label.contains("wrong") || label.contains("right")));
        assert!((action.windows[0].effect.unwrap() + 0.20).abs() < 1e-12);
    }

    #[test]
    fn missing_action_day_fx_is_unavailable_and_never_assumed_to_be_one() {
        let mut action = forward_action("buy", "open", "US", 120.0, 100.0, 100.0, 1.0, 61, 1);
        action.action_day_fx_to_base = None;
        let result = calculate_forward_effect(&[action], &[60]);
        assert_eq!(result.windows[0].status.status, MetricStatus::Unavailable);
        assert_eq!(result.windows[0].matured_actions, 0);
        assert_eq!(result.windows[0].pending_actions, 0);
        assert_eq!(result.windows[0].amount_weighted_excess_return, None);
        assert!(result.actions[0]
            .fact_labels
            .contains(&"data_insufficient".to_string()));
    }

    #[test]
    fn forward_effect_uses_exact_canonical_session_endpoints_without_quote_shifting() {
        let mut sparse = forward_action("buy", "open", "US", 120.0, 110.0, 100.0, 1.0, 61, 1);
        let target_date = sparse.market_session_dates[60];
        let stock_target = sparse
            .stock_prices_local
            .iter()
            .find(|point| point.date == target_date)
            .cloned()
            .unwrap();
        let benchmark_target = sparse
            .benchmark_prices_local
            .iter()
            .find(|point| point.date == target_date)
            .cloned()
            .unwrap();
        sparse.stock_prices_local = vec![sparse.stock_prices_local[0].clone(), stock_target];
        sparse.benchmark_prices_local =
            vec![sparse.benchmark_prices_local[0].clone(), benchmark_target];
        let result = calculate_forward_effect(&[sparse], &[60]);
        assert_eq!(
            result.actions[0].windows[0].status.status,
            MetricStatus::Available
        );
        assert!((result.actions[0].windows[0].effect.unwrap() - 0.10).abs() < 1e-12);

        let mut missing_exact =
            forward_action("buy", "open", "US", 120.0, 110.0, 100.0, 1.0, 62, 1);
        let target_date = missing_exact.market_session_dates[60];
        missing_exact
            .stock_prices_local
            .retain(|point| point.date != target_date);
        let result = calculate_forward_effect(&[missing_exact], &[60]);
        assert_eq!(
            result.actions[0].windows[0].status.status,
            MetricStatus::Unavailable
        );
        assert_eq!(result.actions[0].windows[0].effect, None);
    }

    #[test]
    fn immutable_forward_input_errors_outrank_pending_and_zero_notional_is_not_usable() {
        let mut missing_fx = forward_action("buy", "open", "US", 110.0, 100.0, 100.0, 1.0, 30, 1);
        missing_fx.action_day_fx_to_base = None;
        let result = calculate_forward_effect(&[missing_fx], &[60]);
        assert_eq!(
            result.actions[0].windows[0].status.status,
            MetricStatus::Unavailable
        );
        assert_eq!(result.windows[0].pending_actions, 0);

        let zero = forward_action("zero", "add", "US", 120.0, 100.0, 0.0, 1.0, 61, 1);
        let result = calculate_forward_effect(&[zero], &[60]);
        assert_eq!(
            result.actions[0].windows[0].status.status,
            MetricStatus::Unavailable
        );
        assert_eq!(result.actions[0].windows[0].notional_base, None);
        assert_eq!(result.windows[0].matured_actions, 0);
        assert_eq!(result.windows[0].amount_weighted_excess_return, None);
    }

    #[test]
    fn degraded_matured_action_degrades_window_without_discarding_its_value() {
        let mut action = forward_action("buy", "open", "US", 120.0, 100.0, 100.0, 1.0, 61, 1);
        action.availability = MetricAvailability {
            status: MetricStatus::Degraded,
            note: Some("price coverage below 95%".to_string()),
        };
        let result = calculate_forward_effect(&[action], &[60]);
        assert_eq!(result.windows[0].status.status, MetricStatus::Degraded);
        assert_eq!(result.windows[0].matured_actions, 1);
        assert!((result.windows[0].amount_weighted_excess_return.unwrap() - 0.2).abs() < 1e-12);
    }

    fn snapshot(day: u32, stocks: &[(&str, f64)], cash: f64, reliable: bool) -> RiskSnapshotInput {
        RiskSnapshotInput {
            date: date(day),
            stock_values_base: stocks
                .iter()
                .map(|(symbol, value)| StockValueBase {
                    symbol: (*symbol).to_string(),
                    value_base: *value,
                })
                .collect(),
            cash_value_base: Some(cash),
            reliable,
        }
    }

    #[test]
    fn risk_structure_uses_stock_denominator_and_excludes_non_trades_from_turnover() {
        let input = RiskStructureInput {
            snapshots: vec![
                snapshot(1, &[("A", 60.0), ("B", 30.0), ("C", 10.0)], 100.0, true),
                snapshot(2, &[("A", 95.0), ("B", 5.0)], 100.0, true),
                snapshot(3, &[("A", 90.0), ("B", 10.0)], 100.0, true),
            ],
            stock_changes: vec![
                StockChangeBase::trade(100.0),
                StockChangeBase::trade(-50.0),
                StockChangeBase::transfer(1_000.0),
                StockChangeBase::split(1_000.0),
                StockChangeBase::non_trade(1_000.0),
            ],
            total_stock_trading_fees_base: Some(10.0),
            average_portfolio_nav_base: Some(1_000.0),
        };
        let result = calculate_risk_structure(&input);
        assert!((result.opening.max_stock_weight.unwrap() - 0.60).abs() < 1e-12);
        assert!((result.opening.cr5.unwrap() - 1.0).abs() < 1e-12);
        assert!((result.opening.hhi.unwrap() - 0.46).abs() < 1e-12);
        assert_eq!(result.opening.cash_ratio, Some(0.5));
        assert_eq!(result.peak.max_stock_weight, Some(0.95));
        assert_eq!(result.ending.max_stock_weight, Some(0.9));
        assert_eq!(result.one_way_turnover, Some(0.075));
        assert_eq!(result.fee_drag, Some(0.01));
        assert_eq!(
            result.concentration_availability.status,
            MetricStatus::Available
        );
        assert_eq!(result.turnover_availability.status, MetricStatus::Available);
        assert_eq!(result.fee_availability.status, MetricStatus::Available);
        assert!(result
            .fact_labels
            .contains(&"concentration_changed_materially".to_string()));
    }

    #[test]
    fn risk_structure_preserves_unknown_snapshots_and_zero_fee_import_hint() {
        let result = calculate_risk_structure(&RiskStructureInput {
            snapshots: vec![snapshot(1, &[("A", 100.0)], 0.0, false)],
            stock_changes: vec![],
            total_stock_trading_fees_base: Some(0.0),
            average_portfolio_nav_base: Some(100.0),
        });
        assert_eq!(result.availability.status, MetricStatus::Degraded);
        assert_eq!(
            result.concentration_availability.status,
            MetricStatus::Unavailable
        );
        assert_eq!(result.turnover_availability.status, MetricStatus::Available);
        assert_eq!(result.fee_availability.status, MetricStatus::Available);
        assert_eq!(result.opening.max_stock_weight, None);
        assert_eq!(result.fee_drag, Some(0.0));
        assert!(result
            .data_hints
            .contains(&"fees_may_be_incompletely_imported".to_string()));
    }

    #[test]
    fn risk_period_peaks_are_computed_independently_for_each_stock_metric() {
        let result = calculate_risk_structure(&RiskStructureInput {
            snapshots: vec![
                snapshot(1, &[("A", 60.0), ("B", 20.0), ("C", 20.0)], 0.0, true),
                snapshot(2, &[("A", 55.0), ("B", 45.0)], 0.0, true),
            ],
            stock_changes: vec![],
            total_stock_trading_fees_base: Some(0.0),
            average_portfolio_nav_base: Some(100.0),
        });
        assert_eq!(result.peak.max_stock_weight, Some(0.6));
        assert!((result.peak.hhi.unwrap() - 0.505).abs() < 1e-12);
    }

    #[test]
    fn concentration_aggregates_normalized_symbols_across_accounts() {
        let result = calculate_risk_structure(&RiskStructureInput {
            snapshots: vec![snapshot(
                1,
                &[(" aapl ", 30.0), ("AAPL", 30.0), ("msft", 40.0)],
                0.0,
                true,
            )],
            stock_changes: vec![],
            total_stock_trading_fees_base: Some(0.0),
            average_portfolio_nav_base: Some(100.0),
        });
        assert_eq!(result.opening.max_stock_weight, Some(0.6));
        assert_eq!(result.opening.cr5, Some(1.0));
        assert!((result.opening.hhi.unwrap() - 0.52).abs() < 1e-12);
    }

    fn campaign_summary(status: StockCampaignStatus) -> StockCampaignSummary {
        let fragment = AccountCampaignFragment {
            fragment_id: "fragment-1".to_string(),
            logical_campaign_id: "campaign-1".to_string(),
            account_id: "acct".to_string(),
            symbol: "AAPL".to_string(),
            market: "US".to_string(),
            started_at: "2024-01-01T09:30:00Z".to_string(),
            ended_at: (status == StockCampaignStatus::Completed)
                .then(|| "2024-01-03T16:00:00Z".to_string()),
            status: status.clone(),
            action_ids: vec!["buy".to_string(), "sell".to_string()],
            transfer_in: None,
            transfer_out: None,
        };
        StockCampaignSummary {
            campaign_id: "campaign-1".to_string(),
            account_ids: vec!["acct".to_string()],
            action_ids: fragment.action_ids.clone(),
            fragments: vec![fragment],
            campaign_status: status,
            availability: available(),
            symbol: "AAPL".to_string(),
            market: "US".to_string(),
            started_at: "2024-01-01T09:30:00Z".to_string(),
            ended_at: Some("2024-01-03T16:00:00Z".to_string()),
            contribution: Some(10.0),
        }
    }

    fn flow(day: u32, kind: CampaignCashFlowKind, amount: f64, shares: f64) -> CampaignCashFlow {
        CampaignCashFlow {
            date: date(day),
            kind,
            amount_base: amount,
            shares,
            account_id: "acct".to_string(),
            action_id: None,
        }
    }

    fn review_action(id: &str) -> StockActionReview {
        StockActionReview {
            action_id: id.to_string(),
            transaction_ids: vec![format!("transaction-{id}")],
            account_id: "acct".to_string(),
            symbol: "AAPL".to_string(),
            market: "US".to_string(),
            action_type: "open".to_string(),
            traded_at: "2024-01-01T09:30:00Z".to_string(),
            weighted_average_price: Some(10.0),
            gross_amount: Some(100.0),
            currency: Some("USD".to_string()),
            shares_before: Some(0.0),
            shares_after: Some(10.0),
            portfolio_weight_before: Some(0.0),
            portfolio_weight_after: Some(0.1),
            fees: Some(2.0),
            contribution: Some(10.0),
            observation_windows: vec![],
            status: MetricStatus::Available,
            fact_labels: vec![],
        }
    }

    #[test]
    fn completed_campaign_uses_cash_flow_pnl_and_cash_flow_aware_mae_mfe() {
        let detail = calculate_campaign_detail(&CampaignDetailInput {
            summary: campaign_summary(StockCampaignStatus::Completed),
            cash_flows: vec![
                flow(1, CampaignCashFlowKind::Buy, 100.0, 10.0),
                flow(1, CampaignCashFlowKind::Fee, 2.0, 0.0),
                flow(2, CampaignCashFlowKind::Dividend, 5.0, 0.0),
                flow(3, CampaignCashFlowKind::Sell, 130.0, 10.0),
            ],
            daily_prices: vec![
                CampaignPricePoint::complete(date(1), 8.0, 12.0, 10.0),
                CampaignPricePoint::complete(date(2), 9.0, 14.0, 11.0),
                CampaignPricePoint::complete(date(3), 12.0, 13.0, 13.0),
            ],
            benchmark_prices: sessions(3, 1, 100.0, 110.0),
            current_price_local: None,
            current_fx_to_base: None,
            actions: vec![review_action("buy")],
            forward_actions: vec![forward_action(
                "buy", "open", "US", 120.0, 110.0, 100.0, 1.0, 121, 1,
            )],
            annotations: vec![],
        });
        assert_eq!(detail.pnl.buy_outlays_base, 100.0);
        assert_eq!(detail.pnl.sell_proceeds_base, 130.0);
        assert_eq!(detail.pnl.dividends_base, 5.0);
        assert_eq!(detail.pnl.trading_fees_base, 2.0);
        assert_eq!(detail.pnl.total_pnl_base, Some(33.0));
        assert_eq!(detail.pnl.max_invested_capital_base, Some(102.0));
        assert_eq!(detail.mae_base, Some(-22.0));
        assert_eq!(detail.mfe_base, Some(43.0));
        assert!((detail.mae_percent.unwrap() - (-22.0 / 102.0)).abs() < 1e-12);
        assert!((detail.mfe_percent.unwrap() - (43.0 / 102.0)).abs() < 1e-12);
        assert_eq!(detail.completed_sample_count, 1);
        assert_eq!(detail.active_sample_count, 0);
        assert_eq!(detail.summary.fragments.len(), 1);
        assert_eq!(detail.actions.len(), 1);
        assert_eq!(detail.actions[0].observation_windows.len(), 3);
        assert_eq!(detail.actions[0].observation_windows[0].trading_days, 20);
        assert!(detail.actions[0]
            .fact_labels
            .contains(&"short_term_effective".to_string()));
        assert_eq!(detail.timeline.len(), 4);
        assert!(detail
            .fact_labels
            .iter()
            .all(|label| !label.contains("right") && !label.contains("wrong")));
    }

    #[test]
    fn active_campaign_includes_remaining_value_but_completed_aggregates_exclude_it() {
        let active = calculate_campaign_detail(&CampaignDetailInput {
            summary: campaign_summary(StockCampaignStatus::Active),
            cash_flows: vec![
                flow(1, CampaignCashFlowKind::Buy, 100.0, 10.0),
                flow(1, CampaignCashFlowKind::Fee, 2.0, 0.0),
                flow(2, CampaignCashFlowKind::Dividend, 5.0, 0.0),
                flow(2, CampaignCashFlowKind::Sell, 48.0, 4.0),
            ],
            daily_prices: vec![CampaignPricePoint::complete(date(1), 9.0, 11.0, 10.0)],
            benchmark_prices: sessions(2, 1, 100.0, 105.0),
            current_price_local: Some(15.0),
            current_fx_to_base: Some(1.0),
            actions: vec![],
            forward_actions: vec![],
            annotations: vec![],
        });
        assert_eq!(active.pnl.remaining_shares, 6.0);
        assert_eq!(active.pnl.remaining_market_value_base, Some(90.0));
        assert_eq!(active.pnl.total_pnl_base, Some(41.0));
        assert_eq!(
            active.pnl.label,
            "active_total_pnl_including_remaining_value"
        );

        let completed = calculate_campaign_detail(&CampaignDetailInput {
            summary: campaign_summary(StockCampaignStatus::Completed),
            cash_flows: vec![
                flow(1, CampaignCashFlowKind::Buy, 100.0, 10.0),
                flow(2, CampaignCashFlowKind::Sell, 120.0, 10.0),
            ],
            daily_prices: vec![],
            benchmark_prices: vec![],
            current_price_local: None,
            current_fx_to_base: None,
            actions: vec![],
            forward_actions: vec![],
            annotations: vec![],
        });
        assert_eq!(
            completed.excursion_availability.status,
            MetricStatus::Unavailable
        );
        let aggregate = calculate_campaign_aggregates(&[completed, active]);
        assert_eq!(aggregate.completed_sample_count, 1);
        assert_eq!(aggregate.active_sample_count, 1);
        assert_eq!(aggregate.average_completed_net_pnl_base, Some(20.0));
        assert_eq!(aggregate.completed_ranking.len(), 1);
    }

    #[test]
    fn campaign_missing_intraday_prices_degrades_only_affected_excursion_fields() {
        let detail = calculate_campaign_detail(&CampaignDetailInput {
            summary: campaign_summary(StockCampaignStatus::Active),
            cash_flows: vec![flow(1, CampaignCashFlowKind::Buy, 100.0, 10.0)],
            daily_prices: vec![CampaignPricePoint {
                date: date(1),
                currency: "USD".to_string(),
                low: Some(8.0),
                high: None,
                close: Some(10.0),
                fx_to_base: Some(1.0),
            }],
            benchmark_prices: vec![],
            current_price_local: Some(10.0),
            current_fx_to_base: Some(1.0),
            actions: vec![],
            forward_actions: vec![],
            annotations: vec![],
        });
        assert_eq!(detail.availability.status, MetricStatus::Degraded);
        assert_eq!(detail.pnl_availability.status, MetricStatus::Available);
        assert_eq!(detail.excursion_availability.status, MetricStatus::Degraded);
        assert_eq!(
            detail.benchmark_availability.status,
            MetricStatus::Unavailable
        );
        assert_eq!(detail.mae_base, Some(-20.0));
        assert_eq!(detail.mae_percent, Some(-0.2));
        assert_eq!(detail.mfe_base, None);
        assert_eq!(detail.mfe_percent, None);
        assert!(detail
            .fact_labels
            .contains(&"data_insufficient".to_string()));
    }

    #[test]
    fn active_campaign_without_current_price_keeps_pnl_unavailable_instead_of_zero() {
        let detail = calculate_campaign_detail(&CampaignDetailInput {
            summary: campaign_summary(StockCampaignStatus::Active),
            cash_flows: vec![flow(1, CampaignCashFlowKind::Buy, 100.0, 10.0)],
            daily_prices: vec![CampaignPricePoint::complete(date(1), 9.0, 11.0, 10.0)],
            benchmark_prices: vec![],
            current_price_local: None,
            current_fx_to_base: None,
            actions: vec![],
            forward_actions: vec![],
            annotations: vec![],
        });
        assert_eq!(detail.pnl.total_pnl_base, None);
        assert_eq!(detail.pnl.remaining_market_value_base, None);
        assert_eq!(detail.pnl_availability.status, MetricStatus::Unavailable);
        assert_eq!(detail.availability.status, MetricStatus::Unavailable);
    }

    #[test]
    fn campaign_ohlc_and_remaining_value_use_daily_fx_to_base() {
        let detail = calculate_campaign_detail(&CampaignDetailInput {
            summary: campaign_summary(StockCampaignStatus::Active),
            cash_flows: vec![flow(1, CampaignCashFlowKind::Buy, 200.0, 10.0)],
            daily_prices: vec![CampaignPricePoint {
                date: date(1),
                currency: "CNY".to_string(),
                low: Some(8.0),
                high: Some(12.0),
                close: Some(10.0),
                fx_to_base: Some(2.0),
            }],
            benchmark_prices: vec![],
            current_price_local: Some(12.0),
            current_fx_to_base: Some(2.0),
            actions: vec![],
            forward_actions: vec![],
            annotations: vec![],
        });
        assert_eq!(detail.pnl.remaining_market_value_base, Some(240.0));
        assert_eq!(detail.pnl.total_pnl_base, Some(40.0));
        assert_eq!(detail.mae_base, Some(-40.0));
        assert_eq!(detail.mfe_base, Some(40.0));
    }

    #[test]
    fn invalid_or_incomplete_campaign_ohlc_never_yields_partial_drawdown() {
        let inverted = calculate_campaign_detail(&CampaignDetailInput {
            summary: campaign_summary(StockCampaignStatus::Active),
            cash_flows: vec![flow(1, CampaignCashFlowKind::Buy, 100.0, 10.0)],
            daily_prices: vec![CampaignPricePoint::complete(date(1), 12.0, 8.0, 10.0)],
            benchmark_prices: vec![],
            current_price_local: Some(10.0),
            current_fx_to_base: Some(1.0),
            actions: vec![],
            forward_actions: vec![],
            annotations: vec![],
        });
        assert_eq!(
            inverted.excursion_availability.status,
            MetricStatus::Degraded
        );
        assert_eq!(inverted.mae_base, None);
        assert_eq!(inverted.mfe_base, None);
        assert_eq!(inverted.holding_period_drawdown, None);
        assert_eq!(
            inverted.drawdown_availability.status,
            MetricStatus::Degraded
        );

        let missing_close = calculate_campaign_detail(&CampaignDetailInput {
            summary: campaign_summary(StockCampaignStatus::Active),
            cash_flows: vec![flow(1, CampaignCashFlowKind::Buy, 100.0, 10.0)],
            daily_prices: vec![
                CampaignPricePoint::complete(date(1), 9.0, 11.0, 10.0),
                CampaignPricePoint {
                    date: date(2),
                    currency: "USD".to_string(),
                    low: Some(5.0),
                    high: Some(10.0),
                    close: None,
                    fx_to_base: Some(1.0),
                },
                CampaignPricePoint::complete(date(3), 9.0, 11.0, 10.0),
            ],
            benchmark_prices: vec![],
            current_price_local: Some(10.0),
            current_fx_to_base: Some(1.0),
            actions: vec![],
            forward_actions: vec![],
            annotations: vec![],
        });
        assert_eq!(
            missing_close.excursion_availability.status,
            MetricStatus::Degraded
        );
        assert_eq!(missing_close.holding_period_drawdown, None);
        assert_eq!(
            missing_close.drawdown_availability.status,
            MetricStatus::Degraded
        );
    }
}
