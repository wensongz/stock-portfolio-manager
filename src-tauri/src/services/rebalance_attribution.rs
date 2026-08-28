#![allow(dead_code)]

use crate::models::stock_review::{
    MetricAvailability, MetricStatus, RebalanceAttributionItem, RebalanceAttributionSummary,
};
use crate::services::stock_review_quality::classify_residual_status;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

const EPSILON: f64 = 1e-9;
const PERCENTAGE_BASIS_LABEL: &str =
    "explanatory_approximation_average_nav_not_exact_twr_decomposition";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttributionInput {
    pub base_currency: String,
    pub average_portfolio_nav: Option<f64>,
    pub valuations: Vec<AttributionValuationPoint>,
    pub prices: Vec<AttributionPricePoint>,
    pub fx_rates: Vec<AttributionFxPoint>,
    pub batches: Vec<AttributionBatch>,
    pub splits: Vec<AttributionSplit>,
    pub dividends: Vec<AttributionDividend>,
    pub fees: Vec<AttributionFee>,
    pub cash_returns: Vec<AttributionCashReturn>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttributionValuationPoint {
    pub date: NaiveDate,
    pub positions: Vec<AttributionPositionBalance>,
    pub cash_balances: Vec<AttributionCashBalance>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttributionPositionBalance {
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub currency: String,
    pub actual_quantity: f64,
    pub shadow_quantity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttributionCashBalance {
    pub account_id: String,
    pub currency: String,
    pub actual_amount: f64,
    pub shadow_amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttributionPricePoint {
    pub date: NaiveDate,
    pub symbol: String,
    pub market: String,
    pub currency: String,
    pub close: f64,
}

impl AttributionPricePoint {
    pub fn new(date: NaiveDate, symbol: &str, market: &str, currency: &str, close: f64) -> Self {
        Self {
            date,
            symbol: symbol.to_string(),
            market: market.to_string(),
            currency: currency.to_string(),
            close,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttributionFxPoint {
    pub date: NaiveDate,
    pub currency: String,
    pub base_currency: String,
    pub rate: f64,
}

impl AttributionFxPoint {
    pub fn new(date: NaiveDate, currency: &str, base_currency: &str, rate: f64) -> Self {
        Self {
            date,
            currency: currency.to_string(),
            base_currency: base_currency.to_string(),
            rate,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttributionBatch {
    pub action_id: String,
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub currency: String,
    pub action_type: String,
    pub effective_date: NaiveDate,
    pub quantity_delta: f64,
}

impl AttributionBatch {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        action_id: &str,
        account_id: &str,
        symbol: &str,
        market: &str,
        currency: &str,
        action_type: &str,
        effective_date: NaiveDate,
        quantity_delta: f64,
    ) -> Self {
        Self {
            action_id: action_id.to_string(),
            account_id: account_id.to_string(),
            symbol: symbol.to_string(),
            market: market.to_string(),
            currency: currency.to_string(),
            action_type: action_type.to_string(),
            effective_date,
            quantity_delta,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttributionSplit {
    pub date: NaiveDate,
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub ratio: f64,
}

impl AttributionSplit {
    pub fn new(date: NaiveDate, account_id: &str, symbol: &str, market: &str, ratio: f64) -> Self {
        Self {
            date,
            account_id: account_id.to_string(),
            symbol: symbol.to_string(),
            market: market.to_string(),
            ratio,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttributionDividend {
    pub date: NaiveDate,
    pub symbol: String,
    pub market: String,
    pub currency: String,
    pub amount_per_share: f64,
}

impl AttributionDividend {
    pub fn new(
        date: NaiveDate,
        symbol: &str,
        market: &str,
        currency: &str,
        amount_per_share: f64,
    ) -> Self {
        Self {
            date,
            symbol: symbol.to_string(),
            market: market.to_string(),
            currency: currency.to_string(),
            amount_per_share,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttributionFee {
    pub date: NaiveDate,
    pub action_id: String,
    pub currency: String,
    pub amount: f64,
}

impl AttributionFee {
    pub fn new(date: NaiveDate, action_id: &str, currency: &str, amount: f64) -> Self {
        Self {
            date,
            action_id: action_id.to_string(),
            currency: currency.to_string(),
            amount,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AttributionCashReturn {
    pub date: NaiveDate,
    pub currency: String,
    pub return_rate: f64,
}

impl AttributionCashReturn {
    pub fn new(date: NaiveDate, currency: &str, return_rate: f64) -> Self {
        Self {
            date,
            currency: currency.to_string(),
            return_rate,
        }
    }
}

pub fn calculate_rebalance_attribution(input: &AttributionInput) -> RebalanceAttributionSummary {
    match calculate(input) {
        Ok(summary) => summary,
        Err(note) => unavailable_summary(note),
    }
}

fn calculate(input: &AttributionInput) -> Result<RebalanceAttributionSummary, String> {
    let average_nav = input
        .average_portfolio_nav
        .filter(|value| value.is_finite() && *value > 0.0)
        .ok_or_else(|| {
            "Average portfolio NAV must be finite and positive for precise attribution.".to_string()
        })?;
    if input.valuations.len() < 2 {
        return Err(
            "At least two daily valuation points are required for attribution.".to_string(),
        );
    }
    if input
        .valuations
        .windows(2)
        .any(|window| window[0].date >= window[1].date)
    {
        return Err("Attribution valuation dates must be strictly increasing.".to_string());
    }
    validate_numbers(input)?;
    validate_cash_balance_series(input)?;
    validate_batches_explain_positions(input)?;

    let mut action_amounts = input
        .batches
        .iter()
        .map(|batch| (batch.action_id.clone(), 0.0))
        .collect::<BTreeMap<_, _>>();
    let mut dividend_contribution = 0.0;
    let mut fee_contribution = 0.0;
    let mut currency_contribution = 0.0;
    let mut cash_contribution = 0.0;

    for window in input.valuations.windows(2) {
        let prior = &window[0];
        let current = &window[1];
        for batch in input
            .batches
            .iter()
            .filter(|batch| batch.effective_date <= prior.date)
        {
            let prior_price = price(input, prior.date, batch)?;
            let current_price = price(input, current.date, batch)?;
            let prior_fx = fx(input, prior.date, &batch.currency)?;
            let current_fx = fx(input, current.date, &batch.currency)?;
            let prior_quantity = batch_quantity_on(input, batch, prior.date);
            let interval_split_ratio = split_ratio_between(input, batch, prior.date, current.date);
            let current_quantity = prior_quantity * interval_split_ratio;
            let adjusted_prior_price = prior_price / interval_split_ratio;
            let dividend = input
                .dividends
                .iter()
                .filter(|dividend| {
                    dividend.date == current.date
                        && dividend.symbol == batch.symbol
                        && dividend.market == batch.market
                        && dividend.currency == batch.currency
                })
                .map(|dividend| dividend.amount_per_share)
                .sum::<f64>();
            let price_amount =
                current_quantity * (current_price - adjusted_prior_price) * current_fx;
            let dividend_amount = current_quantity * dividend * current_fx;
            *action_amounts
                .get_mut(&batch.action_id)
                .expect("all batch IDs are initialized") += price_amount + dividend_amount;
            dividend_contribution += dividend_amount;
            currency_contribution += prior_quantity * prior_price * (current_fx - prior_fx);
        }

        for cash in &prior.cash_balances {
            let difference = cash.actual_amount - cash.shadow_amount;
            if difference.abs() <= EPSILON {
                continue;
            }
            let prior_fx = fx(input, prior.date, &cash.currency)?;
            let current_fx = fx(input, current.date, &cash.currency)?;
            let return_rate = cash_return_rate(input, current.date, &cash.currency)?;
            cash_contribution += difference * return_rate * current_fx;
            currency_contribution += difference * (current_fx - prior_fx);
        }
    }

    for fee in &input.fees {
        let batch = input
            .batches
            .iter()
            .find(|batch| batch.action_id == fee.action_id)
            .ok_or_else(|| format!("Fee references unknown action {}.", fee.action_id))?;
        if fee.currency != batch.currency {
            return Err(format!(
                "Fee currency for action {} does not match its attribution batch.",
                fee.action_id
            ));
        }
        let amount = -fee.amount * fx(input, fee.date, &fee.currency)?;
        *action_amounts
            .get_mut(&fee.action_id)
            .expect("the referenced batch exists") += amount;
        fee_contribution += amount;
    }

    let mut action_contributions = input
        .batches
        .iter()
        .map(|batch| {
            let amount = action_amounts[&batch.action_id];
            RebalanceAttributionItem {
                market: batch.market.clone(),
                symbol: batch.symbol.clone(),
                action_type: batch.action_type.clone(),
                action_id: batch.action_id.clone(),
                amount,
                percentage_of_average_nav: Some(amount / average_nav),
            }
        })
        .collect::<Vec<_>>();
    action_contributions.sort_by(|left, right| {
        right
            .amount
            .partial_cmp(&left.amount)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.action_id.cmp(&right.action_id))
    });
    let contributors = action_contributions
        .iter()
        .filter(|item| item.amount > EPSILON)
        .cloned()
        .collect::<Vec<_>>();
    let mut detractors = action_contributions
        .iter()
        .filter(|item| item.amount < -EPSILON)
        .cloned()
        .collect::<Vec<_>>();
    detractors.sort_by(|left, right| {
        left.amount
            .partial_cmp(&right.amount)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.action_id.cmp(&right.action_id))
    });
    let action_total = action_contributions
        .iter()
        .map(|item| item.amount)
        .sum::<f64>();
    let explained = action_total + cash_contribution + currency_contribution;
    let ending_difference = ending_value_difference(input)?;
    let residual = ending_difference - explained;
    let residual_status = classify_residual_status(Some(residual), Some(average_nav));
    let residual_note = match residual_status {
        MetricStatus::Available => None,
        MetricStatus::Degraded => Some(
            "Attribution residual exceeds 0.1% of average NAV; action attribution is an approximation."
                .to_string(),
        ),
        MetricStatus::Unavailable => Some(
            "Attribution residual exceeds 0.5% of average NAV; precise action attribution is unavailable."
                .to_string(),
        ),
        MetricStatus::Pending => None,
    };
    let buy_value_add = action_contributions
        .iter()
        .filter(|item| item.action_type == "open" || item.action_type == "add")
        .map(|item| item.amount)
        .sum();
    let sell_value_add = action_contributions
        .iter()
        .filter(|item| item.action_type == "reduce" || item.action_type == "close")
        .map(|item| item.amount)
        .sum();

    Ok(RebalanceAttributionSummary {
        availability: MetricAvailability {
            status: residual_status,
            note: residual_note,
        },
        total_value_add: Some(ending_difference),
        buy_value_add: Some(buy_value_add),
        sell_value_add: Some(sell_value_add),
        fees: Some(fee_contribution),
        action_contributions,
        contributors,
        detractors,
        dividend_contribution: Some(dividend_contribution),
        fee_contribution: Some(fee_contribution),
        currency_contribution: Some(currency_contribution),
        cash_contribution: Some(cash_contribution),
        explained_value_difference: Some(explained),
        ending_value_difference: Some(ending_difference),
        residual: Some(residual),
        residual_to_average_nav: Some(residual.abs() / average_nav),
        percentage_basis_label: PERCENTAGE_BASIS_LABEL.to_string(),
    })
}

fn validate_numbers(input: &AttributionInput) -> Result<(), String> {
    let valid = input.valuations.iter().all(|point| {
        point.positions.iter().all(|position| {
            position.actual_quantity.is_finite() && position.shadow_quantity.is_finite()
        }) && point
            .cash_balances
            .iter()
            .all(|cash| cash.actual_amount.is_finite() && cash.shadow_amount.is_finite())
    }) && input
        .prices
        .iter()
        .all(|point| point.close.is_finite() && point.close >= 0.0)
        && input
            .fx_rates
            .iter()
            .all(|point| point.rate.is_finite() && point.rate > 0.0)
        && input
            .batches
            .iter()
            .all(|batch| batch.quantity_delta.is_finite())
        && input
            .splits
            .iter()
            .all(|split| split.ratio.is_finite() && split.ratio > 0.0)
        && input
            .dividends
            .iter()
            .all(|event| event.amount_per_share.is_finite() && event.amount_per_share >= 0.0)
        && input
            .fees
            .iter()
            .all(|fee| fee.amount.is_finite() && fee.amount >= 0.0)
        && input
            .cash_returns
            .iter()
            .all(|cash_return| cash_return.return_rate.is_finite());
    valid
        .then_some(())
        .ok_or_else(|| "Attribution input contains an invalid numeric value.".to_string())
}

fn validate_cash_balance_series(input: &AttributionInput) -> Result<(), String> {
    let expected = input.valuations[0]
        .cash_balances
        .iter()
        .map(|cash| (cash.account_id.as_str(), cash.currency.as_str()))
        .collect::<BTreeSet<_>>();
    if expected.len() != input.valuations[0].cash_balances.len() {
        return Err("A daily valuation contains a duplicate cash balance.".to_string());
    }
    for valuation in input.valuations.iter().skip(1) {
        let present = valuation
            .cash_balances
            .iter()
            .map(|cash| (cash.account_id.as_str(), cash.currency.as_str()))
            .collect::<BTreeSet<_>>();
        if present.len() != valuation.cash_balances.len() || present != expected {
            return Err(format!(
                "Daily cash balance coverage is incomplete on {}.",
                valuation.date
            ));
        }
    }
    Ok(())
}

fn validate_batches_explain_positions(input: &AttributionInput) -> Result<(), String> {
    for valuation in &input.valuations {
        let position_keys = valuation
            .positions
            .iter()
            .map(|position| {
                (
                    position.account_id.as_str(),
                    position.symbol.as_str(),
                    position.market.as_str(),
                    position.currency.as_str(),
                )
            })
            .collect::<BTreeSet<_>>();
        for position in &valuation.positions {
            let batch_delta = input
                .batches
                .iter()
                .filter(|batch| {
                    batch.effective_date <= valuation.date
                        && batch.account_id == position.account_id
                        && batch.symbol == position.symbol
                        && batch.market == position.market
                        && batch.currency == position.currency
                })
                .map(|batch| batch_quantity_on(input, batch, valuation.date))
                .sum::<f64>();
            let ledger_delta = position.actual_quantity - position.shadow_quantity;
            if (batch_delta - ledger_delta).abs() > EPSILON {
                return Err(format!(
                    "Action batches do not explain the actual-shadow quantity delta for {}:{} on {}.",
                    position.market, position.symbol, valuation.date
                ));
            }
        }
        for batch in input
            .batches
            .iter()
            .filter(|batch| batch.effective_date <= valuation.date)
        {
            let key = (
                batch.account_id.as_str(),
                batch.symbol.as_str(),
                batch.market.as_str(),
                batch.currency.as_str(),
            );
            if !position_keys.contains(&key) {
                return Err(format!(
                    "Valuation is missing the position delta for action {} on {}.",
                    batch.action_id, valuation.date
                ));
            }
        }
    }
    Ok(())
}

fn batch_quantity_on(input: &AttributionInput, batch: &AttributionBatch, date: NaiveDate) -> f64 {
    batch.quantity_delta
        * input
            .splits
            .iter()
            .filter(|split| {
                split.date > batch.effective_date
                    && split.date <= date
                    && split.account_id == batch.account_id
                    && split.symbol == batch.symbol
                    && split.market == batch.market
            })
            .map(|split| split.ratio)
            .product::<f64>()
}

fn split_ratio_between(
    input: &AttributionInput,
    batch: &AttributionBatch,
    start_exclusive: NaiveDate,
    end_inclusive: NaiveDate,
) -> f64 {
    input
        .splits
        .iter()
        .filter(|split| {
            split.date > batch.effective_date
                && split.date > start_exclusive
                && split.date <= end_inclusive
                && split.account_id == batch.account_id
                && split.symbol == batch.symbol
                && split.market == batch.market
        })
        .map(|split| split.ratio)
        .product::<f64>()
}

fn price(
    input: &AttributionInput,
    date: NaiveDate,
    batch: &AttributionBatch,
) -> Result<f64, String> {
    input
        .prices
        .iter()
        .find(|point| {
            point.date == date
                && point.symbol == batch.symbol
                && point.market == batch.market
                && point.currency == batch.currency
        })
        .map(|point| point.close)
        .ok_or_else(|| {
            format!(
                "Missing attribution price for {}:{} on {}.",
                batch.market, batch.symbol, date
            )
        })
}

fn fx(input: &AttributionInput, date: NaiveDate, currency: &str) -> Result<f64, String> {
    if currency == input.base_currency {
        return Ok(1.0);
    }
    input
        .fx_rates
        .iter()
        .find(|point| {
            point.date == date
                && point.currency == currency
                && point.base_currency == input.base_currency
        })
        .map(|point| point.rate)
        .ok_or_else(|| {
            format!(
                "Missing {} to {} attribution FX rate on {}.",
                currency, input.base_currency, date
            )
        })
}

fn cash_return_rate(
    input: &AttributionInput,
    date: NaiveDate,
    currency: &str,
) -> Result<f64, String> {
    input
        .cash_returns
        .iter()
        .find(|cash_return| cash_return.date == date && cash_return.currency == currency)
        .map(|cash_return| cash_return.return_rate)
        .ok_or_else(|| format!("Missing cash return rate for {} on {}.", currency, date))
}

fn ending_value_difference(input: &AttributionInput) -> Result<f64, String> {
    let ending = input
        .valuations
        .last()
        .expect("at least two valuations were validated");
    let position_value = ending.positions.iter().try_fold(0.0, |sum, position| {
        let lookup = AttributionBatch {
            action_id: String::new(),
            account_id: position.account_id.clone(),
            symbol: position.symbol.clone(),
            market: position.market.clone(),
            currency: position.currency.clone(),
            action_type: String::new(),
            effective_date: ending.date,
            quantity_delta: 0.0,
        };
        let local_price = price(input, ending.date, &lookup)?;
        let rate = fx(input, ending.date, &position.currency)?;
        Ok::<_, String>(
            sum + (position.actual_quantity - position.shadow_quantity) * local_price * rate,
        )
    })?;
    ending
        .cash_balances
        .iter()
        .try_fold(position_value, |sum, cash| {
            fx(input, ending.date, &cash.currency)
                .map(|rate| sum + (cash.actual_amount - cash.shadow_amount) * rate)
        })
}

fn unavailable_summary(note: String) -> RebalanceAttributionSummary {
    RebalanceAttributionSummary {
        availability: MetricAvailability {
            status: MetricStatus::Unavailable,
            note: Some(note),
        },
        total_value_add: None,
        buy_value_add: None,
        sell_value_add: None,
        fees: None,
        action_contributions: vec![],
        contributors: vec![],
        detractors: vec![],
        dividend_contribution: None,
        fee_contribution: None,
        currency_contribution: None,
        cash_contribution: None,
        explained_value_difference: None,
        ending_value_difference: None,
        residual: None,
        residual_to_average_nav: None,
        percentage_basis_label: PERCENTAGE_BASIS_LABEL.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn day(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn identity_input() -> AttributionInput {
        AttributionInput {
            base_currency: "USD".to_string(),
            average_portfolio_nav: Some(10_000.0),
            valuations: vec![
                AttributionValuationPoint {
                    date: day("2024-01-01"),
                    positions: vec![
                        AttributionPositionBalance {
                            account_id: "broker".to_string(),
                            symbol: "A".to_string(),
                            market: "US".to_string(),
                            currency: "USD".to_string(),
                            actual_quantity: 10.0,
                            shadow_quantity: 0.0,
                        },
                        AttributionPositionBalance {
                            account_id: "broker".to_string(),
                            symbol: "B".to_string(),
                            market: "US".to_string(),
                            currency: "USD".to_string(),
                            actual_quantity: 0.0,
                            shadow_quantity: 5.0,
                        },
                    ],
                    cash_balances: vec![
                        AttributionCashBalance {
                            account_id: "broker".to_string(),
                            currency: "USD".to_string(),
                            actual_amount: 0.0,
                            shadow_amount: 850.0,
                        },
                        AttributionCashBalance {
                            account_id: "broker".to_string(),
                            currency: "EUR".to_string(),
                            actual_amount: 100.0,
                            shadow_amount: 0.0,
                        },
                    ],
                },
                AttributionValuationPoint {
                    date: day("2024-01-02"),
                    positions: vec![
                        AttributionPositionBalance {
                            account_id: "broker".to_string(),
                            symbol: "A".to_string(),
                            market: "US".to_string(),
                            currency: "USD".to_string(),
                            actual_quantity: 10.0,
                            shadow_quantity: 0.0,
                        },
                        AttributionPositionBalance {
                            account_id: "broker".to_string(),
                            symbol: "B".to_string(),
                            market: "US".to_string(),
                            currency: "USD".to_string(),
                            actual_quantity: 0.0,
                            shadow_quantity: 5.0,
                        },
                    ],
                    cash_balances: vec![
                        AttributionCashBalance {
                            account_id: "broker".to_string(),
                            currency: "USD".to_string(),
                            actual_amount: 8.0,
                            shadow_amount: 850.0,
                        },
                        AttributionCashBalance {
                            account_id: "broker".to_string(),
                            currency: "EUR".to_string(),
                            actual_amount: 102.0,
                            shadow_amount: 0.0,
                        },
                    ],
                },
            ],
            prices: vec![
                AttributionPricePoint::new(day("2024-01-01"), "A", "US", "USD", 100.0),
                AttributionPricePoint::new(day("2024-01-02"), "A", "US", "USD", 110.0),
                AttributionPricePoint::new(day("2024-01-01"), "B", "US", "USD", 50.0),
                AttributionPricePoint::new(day("2024-01-02"), "B", "US", "USD", 40.0),
            ],
            fx_rates: vec![
                AttributionFxPoint::new(day("2024-01-01"), "EUR", "USD", 1.0),
                AttributionFxPoint::new(day("2024-01-02"), "EUR", "USD", 1.1),
            ],
            batches: vec![
                AttributionBatch::new(
                    "buy-a",
                    "broker",
                    "A",
                    "US",
                    "USD",
                    "open",
                    day("2024-01-01"),
                    10.0,
                ),
                AttributionBatch::new(
                    "sell-b",
                    "broker",
                    "B",
                    "US",
                    "USD",
                    "close",
                    day("2024-01-01"),
                    -5.0,
                ),
            ],
            splits: vec![],
            dividends: vec![AttributionDividend::new(
                day("2024-01-02"),
                "A",
                "US",
                "USD",
                1.0,
            )],
            fees: vec![AttributionFee::new(day("2024-01-02"), "buy-a", "USD", 2.0)],
            cash_returns: vec![
                AttributionCashReturn::new(day("2024-01-02"), "USD", 0.0),
                AttributionCashReturn::new(day("2024-01-02"), "EUR", 0.02),
            ],
        }
    }

    #[test]
    fn attributes_action_cash_and_currency_effects_and_keeps_residual_explicit() {
        // Removing prior-period quantities, reversing the negative B batch, folding FX into
        // stock P&L, or dropping fees/dividends must make this hand-derived identity fail.
        let input = identity_input();

        let summary = calculate_rebalance_attribution(&input);

        assert_eq!(summary.availability.status, MetricStatus::Available);
        assert_eq!(summary.contributors.len(), 2);
        assert!(summary.detractors.is_empty());
        let a = summary
            .action_contributions
            .iter()
            .find(|item| item.action_id == "buy-a")
            .unwrap();
        let b = summary
            .action_contributions
            .iter()
            .find(|item| item.action_id == "sell-b")
            .unwrap();
        assert!((a.amount - 108.0).abs() < 1e-9);
        assert!((b.amount - 50.0).abs() < 1e-9);
        assert!((summary.cash_contribution.unwrap() - 2.2).abs() < 1e-9);
        assert!((summary.currency_contribution.unwrap() - 10.0).abs() < 1e-9);
        assert!((summary.explained_value_difference.unwrap() - 170.2).abs() < 1e-9);
        assert!((summary.ending_value_difference.unwrap() - 170.2).abs() < 1e-9);
        assert!(summary.residual.unwrap().abs() < 1e-9);
        assert_eq!(
            summary.percentage_basis_label,
            "explanatory_approximation_average_nav_not_exact_twr_decomposition"
        );
        assert!((a.percentage_of_average_nav.unwrap() - 0.0108).abs() < 1e-9);
    }

    #[test]
    fn missing_non_base_fx_makes_precise_attribution_unavailable() {
        // Defaulting the missing EUR rate to one must make this fail.
        let mut input = identity_input();
        input
            .fx_rates
            .retain(|point| point.date != day("2024-01-02"));

        let summary = calculate_rebalance_attribution(&input);

        assert_eq!(summary.availability.status, MetricStatus::Unavailable);
        assert!(summary
            .availability
            .note
            .as_deref()
            .unwrap()
            .contains("Missing EUR to USD attribution FX rate"));
        assert_eq!(summary.explained_value_difference, None);
        assert_eq!(summary.residual, None);
    }

    #[test]
    fn missing_cash_return_rate_is_not_silently_treated_as_zero() {
        // An empty rate series is missing data, not evidence that each currency earned 0%.
        let mut input = identity_input();
        input.cash_returns.clear();

        let summary = calculate_rebalance_attribution(&input);

        assert_eq!(summary.availability.status, MetricStatus::Unavailable);
        assert!(summary
            .availability
            .note
            .as_deref()
            .unwrap()
            .contains("Missing cash return rate"));
        assert_eq!(summary.cash_contribution, None);
    }

    #[test]
    fn missing_daily_cash_balance_is_not_silently_treated_as_zero() {
        // Removing a currency from one valuation is incomplete input, not a zero balance.
        let mut input = identity_input();
        input.valuations[1]
            .cash_balances
            .retain(|cash| cash.currency != "EUR");

        let summary = calculate_rebalance_attribution(&input);

        assert_eq!(summary.availability.status, MetricStatus::Unavailable);
        assert!(summary
            .availability
            .note
            .as_deref()
            .unwrap()
            .contains("cash balance"));
        assert_eq!(summary.ending_value_difference, None);
    }

    #[test]
    fn residual_above_point_one_percent_degrades_calculator_summary() {
        // 20 / 10,000 = 0.2%, inside the degraded band.
        let mut input = identity_input();
        input.valuations[1]
            .cash_balances
            .iter_mut()
            .find(|cash| cash.currency == "USD")
            .unwrap()
            .actual_amount += 20.0;

        let summary = calculate_rebalance_attribution(&input);

        assert_eq!(summary.availability.status, MetricStatus::Degraded);
        assert!((summary.residual.unwrap() - 20.0).abs() < 1e-9);
        assert!((summary.residual_to_average_nav.unwrap() - 0.002).abs() < 1e-9);
    }

    #[test]
    fn residual_above_point_five_percent_disables_calculator_summary() {
        // 60 / 10,000 = 0.6%, above the unavailable threshold.
        let mut input = identity_input();
        input.valuations[1]
            .cash_balances
            .iter_mut()
            .find(|cash| cash.currency == "USD")
            .unwrap()
            .actual_amount += 60.0;

        let summary = calculate_rebalance_attribution(&input);

        assert_eq!(summary.availability.status, MetricStatus::Unavailable);
        assert!((summary.residual.unwrap() - 60.0).abs() < 1e-9);
        assert!((summary.residual_to_average_nav.unwrap() - 0.006).abs() < 1e-9);
    }

    #[test]
    fn missing_average_nav_makes_calculator_summary_unavailable() {
        // Defaulting a missing average NAV to any numeric denominator must make this fail.
        let mut input = identity_input();
        input.average_portfolio_nav = None;

        let summary = calculate_rebalance_attribution(&input);

        assert_eq!(summary.availability.status, MetricStatus::Unavailable);
        assert!(summary
            .availability
            .note
            .as_deref()
            .unwrap()
            .contains("Average portfolio NAV"));
        assert_eq!(summary.residual, None);
        assert_eq!(summary.action_contributions, vec![]);
    }

    #[test]
    fn non_positive_or_non_finite_average_nav_makes_calculator_summary_unavailable() {
        // Zero, negative, and NaN cannot be valid percentage denominators.
        for invalid_nav in [0.0, -1.0, f64::NAN] {
            let mut input = identity_input();
            input.average_portfolio_nav = Some(invalid_nav);

            let summary = calculate_rebalance_attribution(&input);

            assert_eq!(
                summary.availability.status,
                MetricStatus::Unavailable,
                "invalid average NAV {invalid_nav:?} must be unavailable"
            );
            assert_eq!(summary.residual_to_average_nav, None);
        }
    }

    #[test]
    fn residual_exactly_point_one_percent_remains_available() {
        // 10 / 10,000 = exactly 0.1%, the inclusive available boundary.
        let mut input = identity_input();
        input.valuations[1]
            .cash_balances
            .iter_mut()
            .find(|cash| cash.currency == "USD")
            .unwrap()
            .actual_amount += 10.0;

        let summary = calculate_rebalance_attribution(&input);

        assert_eq!(summary.availability.status, MetricStatus::Available);
        assert!((summary.residual.unwrap() - 10.0).abs() < 1e-9);
        assert!((summary.residual_to_average_nav.unwrap() - 0.001).abs() < 1e-12);
    }

    #[test]
    fn residual_exactly_point_five_percent_remains_degraded() {
        // 50 / 10,000 = exactly 0.5%, the inclusive degraded boundary.
        let mut input = identity_input();
        input.valuations[1]
            .cash_balances
            .iter_mut()
            .find(|cash| cash.currency == "USD")
            .unwrap()
            .actual_amount += 50.0;

        let summary = calculate_rebalance_attribution(&input);

        assert_eq!(summary.availability.status, MetricStatus::Degraded);
        assert!((summary.residual.unwrap() - 50.0).abs() < 1e-9);
        assert!((summary.residual_to_average_nav.unwrap() - 0.005).abs() < 1e-12);
    }

    #[test]
    fn split_adjusts_action_batch_quantity_and_price_basis_without_creating_an_action() {
        // 10 shares at 100 split 2:1 to 20 at 50 (zero P&L), then rise to 55 (+100).
        let input = AttributionInput {
            base_currency: "USD".to_string(),
            average_portfolio_nav: Some(10_000.0),
            valuations: vec![
                AttributionValuationPoint {
                    date: day("2024-01-01"),
                    positions: vec![AttributionPositionBalance {
                        account_id: "broker".to_string(),
                        symbol: "A".to_string(),
                        market: "US".to_string(),
                        currency: "USD".to_string(),
                        actual_quantity: 10.0,
                        shadow_quantity: 0.0,
                    }],
                    cash_balances: vec![AttributionCashBalance {
                        account_id: "broker".to_string(),
                        currency: "USD".to_string(),
                        actual_amount: 0.0,
                        shadow_amount: 1_000.0,
                    }],
                },
                AttributionValuationPoint {
                    date: day("2024-01-02"),
                    positions: vec![AttributionPositionBalance {
                        account_id: "broker".to_string(),
                        symbol: "A".to_string(),
                        market: "US".to_string(),
                        currency: "USD".to_string(),
                        actual_quantity: 20.0,
                        shadow_quantity: 0.0,
                    }],
                    cash_balances: vec![AttributionCashBalance {
                        account_id: "broker".to_string(),
                        currency: "USD".to_string(),
                        actual_amount: 0.0,
                        shadow_amount: 1_000.0,
                    }],
                },
                AttributionValuationPoint {
                    date: day("2024-01-03"),
                    positions: vec![AttributionPositionBalance {
                        account_id: "broker".to_string(),
                        symbol: "A".to_string(),
                        market: "US".to_string(),
                        currency: "USD".to_string(),
                        actual_quantity: 20.0,
                        shadow_quantity: 0.0,
                    }],
                    cash_balances: vec![AttributionCashBalance {
                        account_id: "broker".to_string(),
                        currency: "USD".to_string(),
                        actual_amount: 0.0,
                        shadow_amount: 1_000.0,
                    }],
                },
            ],
            prices: vec![
                AttributionPricePoint::new(day("2024-01-01"), "A", "US", "USD", 100.0),
                AttributionPricePoint::new(day("2024-01-02"), "A", "US", "USD", 50.0),
                AttributionPricePoint::new(day("2024-01-03"), "A", "US", "USD", 55.0),
            ],
            fx_rates: vec![],
            batches: vec![AttributionBatch::new(
                "buy-a",
                "broker",
                "A",
                "US",
                "USD",
                "open",
                day("2024-01-01"),
                10.0,
            )],
            splits: vec![AttributionSplit::new(
                day("2024-01-02"),
                "broker",
                "A",
                "US",
                2.0,
            )],
            dividends: vec![],
            fees: vec![],
            cash_returns: vec![
                AttributionCashReturn::new(day("2024-01-02"), "USD", 0.0),
                AttributionCashReturn::new(day("2024-01-03"), "USD", 0.0),
            ],
        };

        let summary = calculate_rebalance_attribution(&input);

        assert_eq!(summary.availability.status, MetricStatus::Available);
        assert_eq!(summary.action_contributions.len(), 1);
        assert_eq!(summary.action_contributions[0].action_id, "buy-a");
        assert!((summary.action_contributions[0].amount - 100.0).abs() < 1e-9);
        assert!((summary.ending_value_difference.unwrap() - 100.0).abs() < 1e-9);
        assert!(summary.residual.unwrap().abs() < 1e-9);
    }

    #[test]
    fn detractors_are_ordered_most_negative_first() {
        let positions = vec![
            AttributionPositionBalance {
                account_id: "broker".to_string(),
                symbol: "X".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                actual_quantity: 0.0,
                shadow_quantity: 1.0,
            },
            AttributionPositionBalance {
                account_id: "broker".to_string(),
                symbol: "Y".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                actual_quantity: 0.0,
                shadow_quantity: 1.0,
            },
        ];
        let input = AttributionInput {
            base_currency: "USD".to_string(),
            average_portfolio_nav: Some(10_000.0),
            valuations: vec![
                AttributionValuationPoint {
                    date: day("2024-01-01"),
                    positions: positions.clone(),
                    cash_balances: vec![AttributionCashBalance {
                        account_id: "broker".to_string(),
                        currency: "USD".to_string(),
                        actual_amount: 200.0,
                        shadow_amount: 0.0,
                    }],
                },
                AttributionValuationPoint {
                    date: day("2024-01-02"),
                    positions,
                    cash_balances: vec![AttributionCashBalance {
                        account_id: "broker".to_string(),
                        currency: "USD".to_string(),
                        actual_amount: 200.0,
                        shadow_amount: 0.0,
                    }],
                },
            ],
            prices: vec![
                AttributionPricePoint::new(day("2024-01-01"), "X", "US", "USD", 100.0),
                AttributionPricePoint::new(day("2024-01-02"), "X", "US", "USD", 200.0),
                AttributionPricePoint::new(day("2024-01-01"), "Y", "US", "USD", 100.0),
                AttributionPricePoint::new(day("2024-01-02"), "Y", "US", "USD", 101.0),
            ],
            fx_rates: vec![],
            batches: vec![
                AttributionBatch::new(
                    "sell-x",
                    "broker",
                    "X",
                    "US",
                    "USD",
                    "close",
                    day("2024-01-01"),
                    -1.0,
                ),
                AttributionBatch::new(
                    "sell-y",
                    "broker",
                    "Y",
                    "US",
                    "USD",
                    "close",
                    day("2024-01-01"),
                    -1.0,
                ),
            ],
            splits: vec![],
            dividends: vec![],
            fees: vec![],
            cash_returns: vec![AttributionCashReturn::new(day("2024-01-02"), "USD", 0.0)],
        };

        let summary = calculate_rebalance_attribution(&input);

        assert_eq!(summary.availability.status, MetricStatus::Available);
        assert_eq!(summary.detractors.len(), 2);
        assert_eq!(summary.detractors[0].action_id, "sell-x");
        assert_eq!(summary.detractors[0].amount, -100.0);
        assert_eq!(summary.detractors[1].action_id, "sell-y");
        assert_eq!(summary.detractors[1].amount, -1.0);
    }

    #[test]
    fn negative_action_contribution_is_returned_as_a_detractor() {
        // If B rises after the sale, the -5-share differential loses 50 USD of opportunity value.
        let mut input = identity_input();
        input
            .prices
            .iter_mut()
            .find(|point| point.symbol == "B" && point.date == day("2024-01-02"))
            .unwrap()
            .close = 60.0;

        let summary = calculate_rebalance_attribution(&input);

        assert_eq!(summary.availability.status, MetricStatus::Available);
        assert_eq!(summary.detractors.len(), 1);
        assert_eq!(summary.detractors[0].action_id, "sell-b");
        assert!((summary.detractors[0].amount + 50.0).abs() < 1e-9);
        assert!((summary.residual.unwrap()).abs() < 1e-9);
    }
}
