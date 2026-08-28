#![allow(dead_code)]

use crate::models::performance::ReturnDataPoint;
use crate::models::stock_review::{MetricAvailability, MetricStatus};
use crate::services::performance_service::build_twr_return_series;
use crate::services::stock_review_market_data::MarketReturnMode;
use chrono::NaiveDate;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShadowReturnMethod {
    ExplicitDividends,
    AdjustedClose,
    PriceOnly,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpeningPosition {
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub currency: String,
    pub quantity: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OpeningCashBalance {
    pub account_id: String,
    pub currency: String,
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShadowPricePoint {
    pub date: NaiveDate,
    pub symbol: String,
    pub market: String,
    pub currency: String,
    pub close: f64,
    pub adjusted_close: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShadowFxPoint {
    pub date: NaiveDate,
    pub currency: String,
    pub base_currency: String,
    pub rate: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExternalFlowEvent {
    pub date: NaiveDate,
    pub account_id: String,
    pub currency: String,
    /// Positive for a contribution and negative for a withdrawal.
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct CashIncomeEvent {
    pub date: NaiveDate,
    pub account_id: String,
    pub currency: String,
    pub amount: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DividendEvent {
    pub date: NaiveDate,
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub currency: String,
    pub amount_per_share: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SplitEvent {
    pub date: NaiveDate,
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub ratio: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShadowPortfolioInput {
    pub base_currency: String,
    pub return_method: ShadowReturnMethod,
    pub opening_positions: Vec<OpeningPosition>,
    pub opening_cash: Vec<OpeningCashBalance>,
    pub valuation_dates: Vec<NaiveDate>,
    pub price_points: Vec<ShadowPricePoint>,
    pub fx_points: Vec<ShadowFxPoint>,
    pub external_flows: Vec<ExternalFlowEvent>,
    pub cash_income_events: Vec<CashIncomeEvent>,
    pub dividend_events: Vec<DividendEvent>,
    pub split_events: Vec<SplitEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShadowPositionState {
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub currency: String,
    pub quantity: f64,
    pub price: Option<f64>,
    pub value_in_base: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShadowCashBalance {
    pub account_id: String,
    pub currency: String,
    pub amount: f64,
    pub value_in_base: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShadowDataIssueKind {
    MissingPrice,
    MissingFxRate,
    MissingAdjustedClose,
    InvalidInput,
    DegradedReturnMode,
    TwrUnavailable,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShadowDataIssue {
    pub kind: ShadowDataIssueKind,
    pub date: Option<NaiveDate>,
    pub account_id: Option<String>,
    pub symbol: Option<String>,
    pub market: Option<String>,
    pub currency: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FxForwardFillUsage {
    pub date: NaiveDate,
    pub currency: String,
    pub source_date: NaiveDate,
    pub forward_fill_days: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct FxCoverage {
    pub currency: String,
    pub required_days: usize,
    pub exact_days: usize,
    pub forward_filled_days: usize,
    pub missing_days: usize,
    pub coverage_ratio: Option<f64>,
    pub max_forward_fill_days: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ShadowValuationPoint {
    pub date: NaiveDate,
    pub positions: Vec<ShadowPositionState>,
    pub cash_balances: Vec<ShadowCashBalance>,
    pub external_flow_in_base: Option<f64>,
    pub total_value: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ShadowPortfolioResult {
    pub daily_valuations: Vec<ShadowValuationPoint>,
    pub twr_return_series: Vec<ReturnDataPoint>,
    pub twr_availability: MetricAvailability,
    pub twr_unavailable_from: Option<NaiveDate>,
    pub ending_value: Option<f64>,
    pub return_mode: MarketReturnMode,
    pub return_method: ShadowReturnMethod,
    pub issues: Vec<ShadowDataIssue>,
    pub fx_forward_fills: Vec<FxForwardFillUsage>,
    pub fx_coverage: Vec<FxCoverage>,
}

#[derive(Debug, Clone, Copy)]
struct AdjustedValuationBasis {
    quantity: f64,
    raw_close: f64,
    adjusted_close: f64,
}

pub fn build_shadow_series(input: &ShadowPortfolioInput) -> ShadowPortfolioResult {
    let mut positions = input.opening_positions.clone();
    let mut adjusted_bases = vec![None; positions.len()];
    let mut cash_balances = input.opening_cash.clone();
    let mut daily_valuations = Vec::with_capacity(input.valuation_dates.len());
    let mut external_flows = Vec::new();
    let mut issues = if input.return_method == ShadowReturnMethod::PriceOnly {
        vec![ShadowDataIssue {
            kind: ShadowDataIssueKind::DegradedReturnMode,
            date: None,
            account_id: None,
            symbol: None,
            market: None,
            currency: None,
            message: "Complete adjusted-close or explicit dividend data is unavailable; using raw close without dividends.".to_string(),
        }]
    } else {
        vec![]
    };
    let mut fx_forward_fills = Vec::new();
    let mut fx_coverage_by_currency = BTreeMap::<String, FxCoverage>::new();

    for date in &input.valuation_dates {
        for split in input
            .split_events
            .iter()
            .filter(|split| split.date == *date)
        {
            if split.ratio > 0.0 {
                for position in positions.iter_mut().filter(|position| {
                    position.account_id == split.account_id
                        && position.symbol == split.symbol
                        && position.market == split.market
                }) {
                    position.quantity *= split.ratio;
                }
            }
        }

        let day_external_flows = input
            .external_flows
            .iter()
            .filter(|flow| flow.date == *date)
            .collect::<Vec<_>>();
        for flow in &day_external_flows {
            add_cash(
                &mut cash_balances,
                &flow.account_id,
                &flow.currency,
                flow.amount,
            );
        }

        for income in input
            .cash_income_events
            .iter()
            .filter(|income| income.date == *date)
        {
            add_cash(
                &mut cash_balances,
                &income.account_id,
                &income.currency,
                income.amount,
            );
        }
        if input.return_method == ShadowReturnMethod::ExplicitDividends {
            for dividend in input
                .dividend_events
                .iter()
                .filter(|dividend| dividend.date == *date)
            {
                let quantity = positions
                    .iter()
                    .filter(|position| {
                        position.account_id == dividend.account_id
                            && position.symbol == dividend.symbol
                            && position.market == dividend.market
                            && position.currency == dividend.currency
                    })
                    .map(|position| position.quantity)
                    .sum::<f64>();
                add_cash(
                    &mut cash_balances,
                    &dividend.account_id,
                    &dividend.currency,
                    quantity * dividend.amount_per_share,
                );
            }
        }

        let required_currencies = positions
            .iter()
            .map(|position| position.currency.clone())
            .chain(cash_balances.iter().map(|cash| cash.currency.clone()))
            .collect::<BTreeSet<_>>();
        let mut day_fx = BTreeMap::<String, Option<f64>>::new();
        for currency in required_currencies {
            if currency == input.base_currency {
                day_fx.insert(currency, Some(1.0));
                continue;
            }

            let coverage = fx_coverage_by_currency
                .entry(currency.clone())
                .or_insert_with(|| FxCoverage {
                    currency: currency.clone(),
                    required_days: 0,
                    exact_days: 0,
                    forward_filled_days: 0,
                    missing_days: 0,
                    coverage_ratio: None,
                    max_forward_fill_days: None,
                });
            coverage.required_days += 1;
            let resolved = resolve_fx(input, *date, &currency);
            match resolved {
                Some((rate, source_date)) if source_date == *date => {
                    coverage.exact_days += 1;
                    day_fx.insert(currency, Some(rate));
                }
                Some((rate, source_date)) => {
                    let fill_days = (*date - source_date).num_days();
                    coverage.forward_filled_days += 1;
                    coverage.max_forward_fill_days = Some(
                        coverage
                            .max_forward_fill_days
                            .map_or(fill_days, |current| current.max(fill_days)),
                    );
                    fx_forward_fills.push(FxForwardFillUsage {
                        date: *date,
                        currency: currency.clone(),
                        source_date,
                        forward_fill_days: fill_days,
                    });
                    day_fx.insert(currency, Some(rate));
                }
                None => {
                    coverage.missing_days += 1;
                    issues.push(ShadowDataIssue {
                        kind: ShadowDataIssueKind::MissingFxRate,
                        date: Some(*date),
                        account_id: None,
                        symbol: None,
                        market: None,
                        currency: Some(currency.clone()),
                        message: format!(
                            "No valid {} to {} exchange rate is available on or before {}.",
                            currency, input.base_currency, date
                        ),
                    });
                    day_fx.insert(currency, None);
                }
            }
        }

        let external_flow_in_base = day_external_flows.iter().try_fold(0.0, |sum, flow| {
            day_fx
                .get(&flow.currency)
                .copied()
                .flatten()
                .map(|rate| sum + flow.amount * rate)
        });
        if let Some(flow) = external_flow_in_base.filter(|flow| *flow != 0.0) {
            external_flows.push((*date, flow));
        }

        let position_states = positions
            .iter()
            .enumerate()
            .map(|(position_index, position)| {
                let price = input.price_points.iter().find(|price| {
                    price.date == *date
                        && price.symbol == position.symbol
                        && price.market == position.market
                        && price.currency == position.currency
                });
                let selected_price = price.and_then(|price| match input.return_method {
                    ShadowReturnMethod::AdjustedClose => price
                        .adjusted_close
                        .filter(|adjusted| adjusted.is_finite() && *adjusted > 0.0),
                    ShadowReturnMethod::ExplicitDividends | ShadowReturnMethod::PriceOnly => {
                        Some(price.close)
                    }
                });
                if selected_price.is_none() {
                    let adjusted_is_missing =
                        price.is_some() && input.return_method == ShadowReturnMethod::AdjustedClose;
                    issues.push(ShadowDataIssue {
                        kind: if adjusted_is_missing {
                            ShadowDataIssueKind::MissingAdjustedClose
                        } else {
                            ShadowDataIssueKind::MissingPrice
                        },
                        date: Some(*date),
                        account_id: Some(position.account_id.clone()),
                        symbol: Some(position.symbol.clone()),
                        market: Some(position.market.clone()),
                        currency: Some(position.currency.clone()),
                        message: if adjusted_is_missing {
                            format!(
                                "Adjusted close is unavailable for {}:{} on {}.",
                                position.market, position.symbol, date
                            )
                        } else {
                            format!(
                                "Close price is unavailable for {}:{} on {}.",
                                position.market, position.symbol, date
                            )
                        },
                    });
                }
                let fx_rate = day_fx.get(&position.currency).copied().flatten();
                let value_in_currency = match input.return_method {
                    ShadowReturnMethod::AdjustedClose => {
                        price
                            .zip(selected_price)
                            .and_then(|(price, adjusted_close)| {
                                if !price.close.is_finite() || price.close < 0.0 {
                                    return None;
                                }
                                let basis = adjusted_bases[position_index].get_or_insert(
                                    AdjustedValuationBasis {
                                        quantity: position.quantity,
                                        raw_close: price.close,
                                        adjusted_close,
                                    },
                                );
                                Some(
                                    basis.quantity * basis.raw_close * adjusted_close
                                        / basis.adjusted_close,
                                )
                            })
                    }
                    ShadowReturnMethod::ExplicitDividends | ShadowReturnMethod::PriceOnly => {
                        selected_price.map(|price| price * position.quantity)
                    }
                };
                ShadowPositionState {
                    account_id: position.account_id.clone(),
                    symbol: position.symbol.clone(),
                    market: position.market.clone(),
                    currency: position.currency.clone(),
                    quantity: position.quantity,
                    price: selected_price,
                    value_in_base: value_in_currency
                        .zip(fx_rate)
                        .map(|(value, rate)| value * rate),
                }
            })
            .collect::<Vec<_>>();
        let cash_states = cash_balances
            .iter()
            .map(|cash| ShadowCashBalance {
                account_id: cash.account_id.clone(),
                currency: cash.currency.clone(),
                amount: cash.amount,
                value_in_base: day_fx
                    .get(&cash.currency)
                    .copied()
                    .flatten()
                    .map(|rate| cash.amount * rate),
            })
            .collect::<Vec<_>>();
        let all_available = position_states
            .iter()
            .all(|position| position.value_in_base.is_some())
            && cash_states.iter().all(|cash| cash.value_in_base.is_some());
        let total_value = all_available.then(|| {
            position_states
                .iter()
                .filter_map(|position| position.value_in_base)
                .chain(cash_states.iter().filter_map(|cash| cash.value_in_base))
                .sum()
        });
        daily_valuations.push(ShadowValuationPoint {
            date: *date,
            positions: position_states,
            cash_balances: cash_states,
            external_flow_in_base,
            total_value,
        });
    }

    let twr_unavailable_from = daily_valuations
        .iter()
        .find(|point| point.total_value.is_none() || point.external_flow_in_base.is_none())
        .map(|point| point.date);
    let (twr_return_series, twr_availability) = if let Some(date) = twr_unavailable_from {
        issues.push(ShadowDataIssue {
            kind: ShadowDataIssueKind::TwrUnavailable,
            date: Some(date),
            account_id: None,
            symbol: None,
            market: None,
            currency: None,
            message: format!(
                "Shadow TWR is unavailable because valuation or external-flow conversion is incomplete from {}.",
                date
            ),
        });
        (
            vec![],
            MetricAvailability {
                status: MetricStatus::Unavailable,
                note: Some(format!(
                    "Valuation or external-flow conversion is incomplete from {}.",
                    date
                )),
            },
        )
    } else {
        let daily_values = daily_valuations
            .iter()
            .filter_map(|point| point.total_value.map(|value| (point.date, value, 0.0)))
            .collect::<Vec<_>>();
        (
            build_twr_return_series(&daily_values, None, &external_flows),
            MetricAvailability {
                status: MetricStatus::Available,
                note: None,
            },
        )
    };
    let ending_value = daily_valuations.last().and_then(|point| point.total_value);
    let fx_coverage = fx_coverage_by_currency
        .into_values()
        .map(|mut coverage| {
            coverage.coverage_ratio = (coverage.required_days > 0).then(|| {
                (coverage.exact_days + coverage.forward_filled_days) as f64
                    / coverage.required_days as f64
            });
            coverage
        })
        .collect();

    ShadowPortfolioResult {
        daily_valuations,
        twr_return_series,
        twr_availability,
        twr_unavailable_from,
        ending_value,
        return_mode: match input.return_method {
            ShadowReturnMethod::PriceOnly => MarketReturnMode::PriceOnly,
            ShadowReturnMethod::ExplicitDividends | ShadowReturnMethod::AdjustedClose => {
                MarketReturnMode::TotalReturn
            }
        },
        return_method: input.return_method.clone(),
        issues,
        fx_forward_fills,
        fx_coverage,
    }
}

fn resolve_fx(
    input: &ShadowPortfolioInput,
    date: NaiveDate,
    currency: &str,
) -> Option<(f64, NaiveDate)> {
    input
        .fx_points
        .iter()
        .filter(|point| {
            point.currency == currency
                && point.base_currency == input.base_currency
                && point.date <= date
                && point.rate.is_finite()
                && point.rate > 0.0
        })
        .max_by_key(|point| point.date)
        .map(|point| (point.rate, point.date))
}

fn add_cash(balances: &mut Vec<OpeningCashBalance>, account_id: &str, currency: &str, amount: f64) {
    if let Some(cash) = balances
        .iter_mut()
        .find(|cash| cash.account_id == account_id && cash.currency == currency)
    {
        cash.amount += amount;
    } else {
        balances.push(OpeningCashBalance {
            account_id: account_id.to_string(),
            currency: currency.to_string(),
            amount,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::stock_review::MetricStatus;
    use chrono::NaiveDate;

    fn day(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn dividend_input(return_method: ShadowReturnMethod) -> ShadowPortfolioInput {
        ShadowPortfolioInput {
            base_currency: "USD".to_string(),
            return_method,
            opening_positions: vec![OpeningPosition {
                account_id: "broker".to_string(),
                symbol: "ACME".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                quantity: 10.0,
            }],
            opening_cash: vec![],
            valuation_dates: vec![day("2024-01-01"), day("2024-01-02")],
            price_points: vec![
                ShadowPricePoint {
                    date: day("2024-01-01"),
                    symbol: "ACME".to_string(),
                    market: "US".to_string(),
                    currency: "USD".to_string(),
                    close: 100.0,
                    adjusted_close: Some(100.0),
                },
                ShadowPricePoint {
                    date: day("2024-01-02"),
                    symbol: "ACME".to_string(),
                    market: "US".to_string(),
                    currency: "USD".to_string(),
                    close: 100.0,
                    adjusted_close: Some(103.0),
                },
            ],
            fx_points: vec![],
            external_flows: vec![],
            cash_income_events: vec![],
            dividend_events: vec![DividendEvent {
                date: day("2024-01-02"),
                account_id: "broker".to_string(),
                symbol: "ACME".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                amount_per_share: 2.0,
            }],
            split_events: vec![],
        }
    }

    #[test]
    fn ignores_stock_trades_but_replays_external_flows() {
        let input = ShadowPortfolioInput {
            base_currency: "USD".to_string(),
            return_method: ShadowReturnMethod::PriceOnly,
            opening_positions: vec![OpeningPosition {
                account_id: "broker".to_string(),
                symbol: "ACME".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                quantity: 10.0,
            }],
            opening_cash: vec![OpeningCashBalance {
                account_id: "broker".to_string(),
                currency: "USD".to_string(),
                amount: 1_000.0,
            }],
            valuation_dates: vec![day("2024-01-01"), day("2024-01-02")],
            price_points: vec![
                ShadowPricePoint {
                    date: day("2024-01-01"),
                    symbol: "ACME".to_string(),
                    market: "US".to_string(),
                    currency: "USD".to_string(),
                    close: 100.0,
                    adjusted_close: None,
                },
                ShadowPricePoint {
                    date: day("2024-01-02"),
                    symbol: "ACME".to_string(),
                    market: "US".to_string(),
                    currency: "USD".to_string(),
                    close: 110.0,
                    adjusted_close: None,
                },
            ],
            fx_points: vec![],
            external_flows: vec![ExternalFlowEvent {
                date: day("2024-01-02"),
                account_id: "broker".to_string(),
                currency: "USD".to_string(),
                amount: 500.0,
            }],
            cash_income_events: vec![],
            dividend_events: vec![],
            split_events: vec![],
        };

        let result = build_shadow_series(&input);

        let ending = &result.daily_valuations[1];
        assert_eq!(ending.positions[0].quantity, 10.0);
        assert_eq!(ending.cash_balances[0].amount, 1_500.0);
        assert_eq!(ending.total_value, Some(2_600.0));
    }

    #[test]
    fn split_changes_quantity_without_changing_value() {
        let input = ShadowPortfolioInput {
            base_currency: "USD".to_string(),
            return_method: ShadowReturnMethod::PriceOnly,
            opening_positions: vec![OpeningPosition {
                account_id: "broker".to_string(),
                symbol: "ACME".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                quantity: 10.0,
            }],
            opening_cash: vec![],
            valuation_dates: vec![day("2024-01-01"), day("2024-01-02")],
            price_points: vec![
                ShadowPricePoint {
                    date: day("2024-01-01"),
                    symbol: "ACME".to_string(),
                    market: "US".to_string(),
                    currency: "USD".to_string(),
                    close: 100.0,
                    adjusted_close: None,
                },
                ShadowPricePoint {
                    date: day("2024-01-02"),
                    symbol: "ACME".to_string(),
                    market: "US".to_string(),
                    currency: "USD".to_string(),
                    close: 50.0,
                    adjusted_close: None,
                },
            ],
            fx_points: vec![],
            external_flows: vec![],
            cash_income_events: vec![],
            dividend_events: vec![],
            split_events: vec![SplitEvent {
                date: day("2024-01-02"),
                account_id: "broker".to_string(),
                symbol: "ACME".to_string(),
                market: "US".to_string(),
                ratio: 2.0,
            }],
        };

        let result = build_shadow_series(&input);

        assert_eq!(result.daily_valuations[0].total_value, Some(1_000.0));
        assert_eq!(result.daily_valuations[1].positions[0].quantity, 20.0);
        assert_eq!(result.daily_valuations[1].total_value, Some(1_000.0));
    }

    #[test]
    fn explicit_dividends_add_cash_to_raw_close_valuation() {
        let result = build_shadow_series(&dividend_input(ShadowReturnMethod::ExplicitDividends));

        assert_eq!(result.return_mode, MarketReturnMode::TotalReturn);
        assert_eq!(result.daily_valuations[1].positions[0].price, Some(100.0));
        assert_eq!(result.daily_valuations[1].cash_balances[0].amount, 20.0);
        assert_eq!(result.ending_value, Some(1_020.0));
    }

    #[test]
    fn adjusted_close_supplies_total_return_without_double_counting_dividends() {
        let result = build_shadow_series(&dividend_input(ShadowReturnMethod::AdjustedClose));

        assert_eq!(result.return_mode, MarketReturnMode::TotalReturn);
        assert_eq!(result.daily_valuations[1].positions[0].price, Some(103.0));
        assert!(result.daily_valuations[1].cash_balances.is_empty());
        assert_eq!(result.ending_value, Some(1_030.0));
    }

    #[test]
    fn adjusted_close_split_uses_one_corporate_action_basis() {
        let input = ShadowPortfolioInput {
            base_currency: "USD".to_string(),
            return_method: ShadowReturnMethod::AdjustedClose,
            opening_positions: vec![OpeningPosition {
                account_id: "broker".to_string(),
                symbol: "ACME".to_string(),
                market: "US".to_string(),
                currency: "USD".to_string(),
                quantity: 10.0,
            }],
            opening_cash: vec![],
            valuation_dates: vec![day("2024-01-01"), day("2024-01-02")],
            price_points: vec![
                ShadowPricePoint {
                    date: day("2024-01-01"),
                    symbol: "ACME".to_string(),
                    market: "US".to_string(),
                    currency: "USD".to_string(),
                    close: 100.0,
                    adjusted_close: Some(50.0),
                },
                ShadowPricePoint {
                    date: day("2024-01-02"),
                    symbol: "ACME".to_string(),
                    market: "US".to_string(),
                    currency: "USD".to_string(),
                    close: 50.0,
                    adjusted_close: Some(50.0),
                },
            ],
            fx_points: vec![],
            external_flows: vec![],
            cash_income_events: vec![],
            dividend_events: vec![],
            split_events: vec![SplitEvent {
                date: day("2024-01-02"),
                account_id: "broker".to_string(),
                symbol: "ACME".to_string(),
                market: "US".to_string(),
                ratio: 2.0,
            }],
        };

        let result = build_shadow_series(&input);

        assert_eq!(result.daily_valuations[0].total_value, Some(1_000.0));
        assert_eq!(result.daily_valuations[1].positions[0].quantity, 20.0);
        assert_eq!(result.daily_valuations[1].total_value, Some(1_000.0));
        assert_eq!(result.twr_return_series[1].daily_return, 0.0);
        assert_eq!(result.twr_return_series[1].cumulative_return, 0.0);
    }

    #[test]
    fn price_only_excludes_dividends_and_reports_degraded_return_mode() {
        let result = build_shadow_series(&dividend_input(ShadowReturnMethod::PriceOnly));

        assert_eq!(result.return_mode, MarketReturnMode::PriceOnly);
        assert_eq!(result.daily_valuations[1].positions[0].price, Some(100.0));
        assert!(result.daily_valuations[1].cash_balances.is_empty());
        assert_eq!(result.ending_value, Some(1_000.0));
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.kind == ShadowDataIssueKind::DegradedReturnMode));
    }

    #[test]
    fn stock_and_cash_share_the_same_forward_filled_fx_path() {
        let input = ShadowPortfolioInput {
            base_currency: "USD".to_string(),
            return_method: ShadowReturnMethod::PriceOnly,
            opening_positions: vec![OpeningPosition {
                account_id: "hk-broker".to_string(),
                symbol: "0700".to_string(),
                market: "HK".to_string(),
                currency: "HKD".to_string(),
                quantity: 10.0,
            }],
            opening_cash: vec![OpeningCashBalance {
                account_id: "hk-broker".to_string(),
                currency: "HKD".to_string(),
                amount: 1_000.0,
            }],
            valuation_dates: vec![day("2024-01-02"), day("2024-01-03")],
            price_points: vec![
                ShadowPricePoint {
                    date: day("2024-01-02"),
                    symbol: "0700".to_string(),
                    market: "HK".to_string(),
                    currency: "HKD".to_string(),
                    close: 100.0,
                    adjusted_close: None,
                },
                ShadowPricePoint {
                    date: day("2024-01-03"),
                    symbol: "0700".to_string(),
                    market: "HK".to_string(),
                    currency: "HKD".to_string(),
                    close: 100.0,
                    adjusted_close: None,
                },
            ],
            fx_points: vec![ShadowFxPoint {
                date: day("2024-01-02"),
                currency: "HKD".to_string(),
                base_currency: "USD".to_string(),
                rate: 0.125,
            }],
            external_flows: vec![],
            cash_income_events: vec![],
            dividend_events: vec![],
            split_events: vec![],
        };

        let result = build_shadow_series(&input);

        assert_eq!(
            result.daily_valuations[0].positions[0].value_in_base,
            Some(125.0)
        );
        assert_eq!(
            result.daily_valuations[0].cash_balances[0].value_in_base,
            Some(125.0)
        );
        assert_eq!(result.daily_valuations[1].total_value, Some(250.0));
        assert_eq!(
            result.fx_forward_fills,
            vec![FxForwardFillUsage {
                date: day("2024-01-03"),
                currency: "HKD".to_string(),
                source_date: day("2024-01-02"),
                forward_fill_days: 1,
            }]
        );
        assert_eq!(
            result.fx_coverage,
            vec![FxCoverage {
                currency: "HKD".to_string(),
                required_days: 2,
                exact_days: 1,
                forward_filled_days: 1,
                missing_days: 0,
                coverage_ratio: Some(1.0),
                max_forward_fill_days: Some(1),
            }]
        );
    }

    #[test]
    fn external_deposit_is_twr_neutral_while_cash_income_is_return() {
        let input = ShadowPortfolioInput {
            base_currency: "USD".to_string(),
            return_method: ShadowReturnMethod::ExplicitDividends,
            opening_positions: vec![],
            opening_cash: vec![OpeningCashBalance {
                account_id: "broker".to_string(),
                currency: "USD".to_string(),
                amount: 1_000.0,
            }],
            valuation_dates: vec![day("2024-01-01"), day("2024-01-02"), day("2024-01-03")],
            price_points: vec![],
            fx_points: vec![],
            external_flows: vec![ExternalFlowEvent {
                date: day("2024-01-02"),
                account_id: "broker".to_string(),
                currency: "USD".to_string(),
                amount: 500.0,
            }],
            cash_income_events: vec![CashIncomeEvent {
                date: day("2024-01-03"),
                account_id: "broker".to_string(),
                currency: "USD".to_string(),
                amount: 75.0,
            }],
            dividend_events: vec![],
            split_events: vec![],
        };

        let result = build_shadow_series(&input);

        assert_eq!(result.twr_return_series.len(), 3);
        assert_eq!(result.twr_return_series[1].daily_return, 0.0);
        assert_eq!(result.twr_return_series[1].daily_pnl, 0.0);
        assert!((result.twr_return_series[2].daily_return - 5.0).abs() < 1e-9);
        assert!((result.twr_return_series[2].cumulative_return - 5.0).abs() < 1e-9);
    }

    #[test]
    fn missing_fx_makes_valuation_unavailable_instead_of_assuming_one() {
        let input = ShadowPortfolioInput {
            base_currency: "USD".to_string(),
            return_method: ShadowReturnMethod::ExplicitDividends,
            opening_positions: vec![],
            opening_cash: vec![OpeningCashBalance {
                account_id: "broker".to_string(),
                currency: "EUR".to_string(),
                amount: 1_000.0,
            }],
            valuation_dates: vec![day("2024-01-02")],
            price_points: vec![],
            fx_points: vec![],
            external_flows: vec![],
            cash_income_events: vec![],
            dividend_events: vec![],
            split_events: vec![],
        };

        let result = build_shadow_series(&input);

        assert_eq!(
            result.daily_valuations[0].cash_balances[0].value_in_base,
            None
        );
        assert_eq!(result.daily_valuations[0].total_value, None);
        assert_eq!(result.ending_value, None);
        assert!(result.twr_return_series.is_empty());
        assert!(result.issues.iter().any(|issue| {
            issue.kind == ShadowDataIssueKind::MissingFxRate
                && issue.date == Some(day("2024-01-02"))
                && issue.currency.as_deref() == Some("EUR")
        }));
        assert_eq!(result.fx_coverage[0].missing_days, 1);
        assert_eq!(result.fx_coverage[0].coverage_ratio, Some(0.0));
    }

    #[test]
    fn missing_fx_after_a_valid_day_makes_twr_unavailable() {
        let input = ShadowPortfolioInput {
            base_currency: "USD".to_string(),
            return_method: ShadowReturnMethod::ExplicitDividends,
            opening_positions: vec![],
            opening_cash: vec![OpeningCashBalance {
                account_id: "broker".to_string(),
                currency: "USD".to_string(),
                amount: 1_000.0,
            }],
            valuation_dates: vec![day("2024-01-01"), day("2024-01-02"), day("2024-01-03")],
            price_points: vec![],
            fx_points: vec![ShadowFxPoint {
                date: day("2024-01-03"),
                currency: "EUR".to_string(),
                base_currency: "USD".to_string(),
                rate: 1.0,
            }],
            external_flows: vec![ExternalFlowEvent {
                date: day("2024-01-02"),
                account_id: "broker".to_string(),
                currency: "EUR".to_string(),
                amount: 500.0,
            }],
            cash_income_events: vec![],
            dividend_events: vec![],
            split_events: vec![],
        };

        let result = build_shadow_series(&input);

        assert_eq!(result.daily_valuations[1].total_value, None);
        assert_eq!(result.daily_valuations[2].total_value, Some(1_500.0));
        assert!(result.twr_return_series.is_empty());
        assert_eq!(result.twr_availability.status, MetricStatus::Unavailable);
        assert_eq!(result.twr_unavailable_from, Some(day("2024-01-02")));
        assert!(result.issues.iter().any(|issue| {
            issue.kind == ShadowDataIssueKind::TwrUnavailable
                && issue.date == Some(day("2024-01-02"))
        }));
    }

    #[test]
    fn split_precedes_same_day_per_share_dividend() {
        let mut input = dividend_input(ShadowReturnMethod::ExplicitDividends);
        input.price_points[1].close = 50.0;
        input.dividend_events[0].amount_per_share = 1.0;
        input.split_events.push(SplitEvent {
            date: day("2024-01-02"),
            account_id: "broker".to_string(),
            symbol: "ACME".to_string(),
            market: "US".to_string(),
            ratio: 2.0,
        });

        let result = build_shadow_series(&input);

        assert_eq!(result.daily_valuations[1].positions[0].quantity, 20.0);
        assert_eq!(result.daily_valuations[1].cash_balances[0].amount, 20.0);
        assert_eq!(result.daily_valuations[1].total_value, Some(1_020.0));
    }

    #[test]
    fn missing_price_makes_the_affected_valuation_unavailable() {
        let mut input = dividend_input(ShadowReturnMethod::PriceOnly);
        input.price_points.pop();
        input.dividend_events.clear();

        let result = build_shadow_series(&input);

        assert_eq!(result.daily_valuations[1].positions[0].price, None);
        assert_eq!(result.daily_valuations[1].total_value, None);
        assert_eq!(result.ending_value, None);
        assert!(result.twr_return_series.is_empty());
        assert_eq!(result.twr_availability.status, MetricStatus::Unavailable);
        assert_eq!(result.twr_unavailable_from, Some(day("2024-01-02")));
        assert!(result.issues.iter().any(|issue| {
            issue.kind == ShadowDataIssueKind::MissingPrice
                && issue.date == Some(day("2024-01-02"))
                && issue.account_id.as_deref() == Some("broker")
                && issue.market.as_deref() == Some("US")
                && issue.symbol.as_deref() == Some("ACME")
        }));
        assert!(result.issues.iter().any(|issue| {
            issue.kind == ShadowDataIssueKind::TwrUnavailable
                && issue.date == Some(day("2024-01-02"))
        }));
    }
}
