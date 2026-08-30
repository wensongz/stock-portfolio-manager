#[derive(Debug, Clone, PartialEq)]
pub struct EndpointEffectInput {
    pub action_type: String,
    pub quantity: f64,
    pub trade_price: f64,
    pub trade_notional_local: f64,
    pub end_price: Option<f64>,
    pub fee_local: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EndpointEffectOutput {
    pub price_effect_local: Option<f64>,
    pub price_effect_percent: Option<f64>,
}

fn finite(value: f64) -> bool {
    value.is_finite()
}

pub fn calculate_endpoint_effect(input: &EndpointEffectInput) -> EndpointEffectOutput {
    let Some(end_price) = input.end_price else {
        return EndpointEffectOutput {
            price_effect_local: None,
            price_effect_percent: None,
        };
    };
    if ![
        input.quantity,
        input.trade_price,
        end_price,
        input.fee_local,
    ]
    .into_iter()
    .all(finite)
    {
        return EndpointEffectOutput {
            price_effect_local: None,
            price_effect_percent: None,
        };
    }
    let direction = match input.action_type.as_str() {
        "open" | "add" => 1.0,
        "reduce" | "close" => -1.0,
        _ => {
            return EndpointEffectOutput {
                price_effect_local: None,
                price_effect_percent: None,
            };
        }
    };
    let price_effect_local =
        input.quantity.abs() * (end_price - input.trade_price) * direction - input.fee_local;
    let denominator = input.trade_notional_local.abs();
    EndpointEffectOutput {
        price_effect_local: Some(price_effect_local),
        price_effect_percent: (finite(denominator) && denominator > 0.0)
            .then_some(price_effect_local / denominator),
    }
}

pub fn calculate_directional_excess(
    action_type: &str,
    stock_return: f64,
    benchmark_return: f64,
) -> Option<f64> {
    if !finite(stock_return) || !finite(benchmark_return) {
        return None;
    }
    match action_type {
        "open" | "add" => Some(stock_return - benchmark_return),
        "reduce" | "close" => Some(benchmark_return - stock_return),
        _ => None,
    }
}

fn is_buy(action_type: &str) -> bool {
    matches!(action_type, "open" | "add")
}

fn is_sell(action_type: &str) -> bool {
    matches!(action_type, "reduce" | "close")
}

fn summarize_group(actions: &[&StockOperationEffect]) -> StockOperationGroupSummary {
    let calculable = actions
        .iter()
        .copied()
        .filter(|action| action.price_effect_local.is_some())
        .collect::<Vec<_>>();
    let positive_count = calculable
        .iter()
        .filter(|action| action.price_effect_local.is_some_and(|value| value > 0.0))
        .count();
    let negative_count = calculable
        .iter()
        .filter(|action| action.price_effect_local.is_some_and(|value| value < 0.0))
        .count();

    let price_effect_base = if calculable.is_empty() {
        None
    } else if calculable
        .iter()
        .all(|action| action.price_effect_base.is_some())
    {
        Some(
            calculable
                .iter()
                .filter_map(|action| action.price_effect_base)
                .sum(),
        )
    } else {
        None
    };

    let positive_notional_ratio = if calculable.is_empty()
        || calculable
            .iter()
            .any(|action| action.trade_notional_base.is_none())
    {
        None
    } else {
        let denominator: f64 = calculable
            .iter()
            .filter_map(|action| action.trade_notional_base)
            .map(f64::abs)
            .sum();
        let positive: f64 = calculable
            .iter()
            .filter(|action| action.price_effect_local.is_some_and(|value| value > 0.0))
            .filter_map(|action| action.trade_notional_base)
            .map(f64::abs)
            .sum();
        (denominator > 0.0 && denominator.is_finite()).then_some(positive / denominator)
    };

    let benchmark_actions = actions
        .iter()
        .copied()
        .filter(|action| action.directional_excess_return.is_some())
        .collect::<Vec<_>>();
    let weighted_excess_return = if benchmark_actions.is_empty()
        || benchmark_actions
            .iter()
            .any(|action| action.trade_notional_base.is_none())
    {
        None
    } else {
        let denominator: f64 = benchmark_actions
            .iter()
            .filter_map(|action| action.trade_notional_base)
            .map(f64::abs)
            .sum();
        let numerator: f64 = benchmark_actions
            .iter()
            .filter_map(|action| {
                Some(action.directional_excess_return? * action.trade_notional_base?.abs())
            })
            .sum();
        (denominator > 0.0 && denominator.is_finite()).then_some(numerator / denominator)
    };

    StockOperationGroupSummary {
        action_count: actions.len(),
        positive_count,
        negative_count,
        missing_effect_count: actions.len() - calculable.len(),
        price_effect_base,
        positive_notional_ratio,
        weighted_excess_return,
    }
}

fn complete_notional_sum(actions: &[&StockOperationEffect]) -> Option<f64> {
    if actions
        .iter()
        .any(|action| action.trade_notional_base.is_none())
    {
        return None;
    }
    Some(
        actions
            .iter()
            .filter_map(|action| action.trade_notional_base)
            .map(f64::abs)
            .sum(),
    )
}

fn complete_fee_sum(actions: &[&StockOperationEffect]) -> Option<f64> {
    if actions
        .iter()
        .any(|action| action.fee_local != 0.0 && action.fee_base.is_none())
    {
        return None;
    }
    Some(actions.iter().filter_map(|action| action.fee_base).sum())
}

pub fn summarize_actions(actions: &[StockOperationEffect]) -> StockOperationReviewSummary {
    let all = actions.iter().collect::<Vec<_>>();
    let buys = actions
        .iter()
        .filter(|action| is_buy(&action.action_type))
        .collect::<Vec<_>>();
    let sells = actions
        .iter()
        .filter(|action| is_sell(&action.action_type))
        .collect::<Vec<_>>();
    let largest_absolute_weight_change = actions
        .iter()
        .filter_map(|action| action.weight_change)
        .map(f64::abs)
        .filter(|value| value.is_finite())
        .max_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal));

    StockOperationReviewSummary {
        total: summarize_group(&all),
        buys: summarize_group(&buys),
        sells: summarize_group(&sells),
        position_impact: StockPositionImpactSummary {
            invested_amount_base: complete_notional_sum(&buys),
            recovered_amount_base: complete_notional_sum(&sells),
            largest_absolute_weight_change,
            total_fees_base: complete_fee_sum(&all),
            missing_weight_count: actions
                .iter()
                .filter(|action| action.weight_change.is_none())
                .count(),
        },
    }
}

pub fn summarize_securities(
    actions: &[StockOperationEffect],
) -> Vec<StockOperationSecuritySummary> {
    let mut grouped = BTreeMap::<(String, String, String), Vec<&StockOperationEffect>>::new();
    for action in actions {
        let key = (
            action.account_id.clone(),
            normalize_stock_market(&action.market).unwrap_or_else(|| action.market.clone()),
            normalize_stock_symbol(&action.symbol).unwrap_or_else(|| action.symbol.clone()),
        );
        grouped.entry(key).or_default().push(action);
    }

    let mut summaries = grouped
        .into_values()
        .filter_map(|group| {
            let first = *group.first()?;
            let group_summary = summarize_group(&group);
            let local_effects = group
                .iter()
                .filter_map(|action| action.price_effect_local)
                .collect::<Vec<_>>();
            let name = group
                .iter()
                .map(|action| action.name.trim())
                .find(|name| !name.is_empty())
                .unwrap_or(first.name.as_str())
                .to_string();
            Some(StockOperationSecuritySummary {
                account_id: first.account_id.clone(),
                account_name: first.account_name.clone(),
                symbol: first.symbol.clone(),
                name,
                market: first.market.clone(),
                currency: first.currency.clone(),
                open_count: group
                    .iter()
                    .filter(|action| action.action_type == "open")
                    .count(),
                add_count: group
                    .iter()
                    .filter(|action| action.action_type == "add")
                    .count(),
                reduce_count: group
                    .iter()
                    .filter(|action| action.action_type == "reduce")
                    .count(),
                close_count: group
                    .iter()
                    .filter(|action| action.action_type == "close")
                    .count(),
                net_shares: group
                    .iter()
                    .map(|action| {
                        if is_buy(&action.action_type) {
                            action.quantity.abs()
                        } else {
                            -action.quantity.abs()
                        }
                    })
                    .sum(),
                buy_notional_local: group
                    .iter()
                    .filter(|action| is_buy(&action.action_type))
                    .map(|action| action.trade_notional_local.abs())
                    .sum(),
                sell_notional_local: group
                    .iter()
                    .filter(|action| is_sell(&action.action_type))
                    .map(|action| action.trade_notional_local.abs())
                    .sum(),
                price_effect_local: (!local_effects.is_empty()).then(|| local_effects.iter().sum()),
                price_effect_base: group_summary.price_effect_base,
                weighted_excess_return: group_summary.weighted_excess_return,
                largest_absolute_weight_change: group
                    .iter()
                    .filter_map(|action| action.weight_change)
                    .map(f64::abs)
                    .filter(|value| value.is_finite())
                    .max_by(|left, right| left.partial_cmp(right).unwrap_or(Ordering::Equal)),
                positive_count: group_summary.positive_count,
                negative_count: group_summary.negative_count,
                missing_effect_count: group_summary.missing_effect_count,
            })
        })
        .collect::<Vec<_>>();

    summaries.sort_by(
        |left, right| match (left.price_effect_base, right.price_effect_base) {
            (Some(left_value), Some(right_value)) => right_value
                .partial_cmp(&left_value)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.market.cmp(&right.market))
                .then_with(|| left.symbol.cmp(&right.symbol))
                .then_with(|| left.account_id.cmp(&right.account_id)),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => left
                .market
                .cmp(&right.market)
                .then_with(|| left.symbol.cmp(&right.symbol))
                .then_with(|| left.account_id.cmp(&right.account_id)),
        },
    );
    summaries
}

#[cfg(test)]
mod tests {
    use super::{
        calculate_directional_excess, calculate_endpoint_effect, summarize_actions,
        summarize_securities, EndpointEffectInput,
    };
    use crate::models::stock_operation_review::StockOperationEffect;
    use chrono::NaiveDate;

    fn assert_close(actual: Option<f64>, expected: f64) {
        assert!((actual.expect("expected a calculated value") - expected).abs() < 1e-12);
    }

    fn input(
        action_type: &str,
        quantity: f64,
        trade_price: f64,
        end_price: Option<f64>,
        fee_local: f64,
    ) -> EndpointEffectInput {
        EndpointEffectInput {
            action_type: action_type.to_string(),
            quantity,
            trade_price,
            trade_notional_local: quantity * trade_price,
            end_price,
            fee_local,
        }
    }

    fn action(
        action_id: &str,
        symbol: &str,
        action_type: &str,
        notional_base: Option<f64>,
        effect_local: Option<f64>,
        effect_base: Option<f64>,
        excess: Option<f64>,
        weight_change: Option<f64>,
    ) -> StockOperationEffect {
        StockOperationEffect {
            action_id: action_id.to_string(),
            transaction_ids: vec![format!("tx-{action_id}")],
            account_id: "account-1".to_string(),
            account_name: "账户一".to_string(),
            symbol: symbol.to_string(),
            name: "测试股票".to_string(),
            market: "US".to_string(),
            action_type: action_type.to_string(),
            trade_date: NaiveDate::from_ymd_opt(2026, 7, 1).unwrap(),
            quantity: 10.0,
            trade_price: 10.0,
            trade_notional_local: notional_base.unwrap_or(100.0),
            trade_notional_base: notional_base,
            fee_local: 2.0,
            fee_base: Some(2.0),
            currency: "USD".to_string(),
            shares_before: 0.0,
            shares_after: 10.0,
            prior_nav_date: None,
            prior_nav_base: None,
            weight_before: None,
            weight_after: None,
            weight_change,
            operation_size_ratio: None,
            evaluation_date: Some(NaiveDate::from_ymd_opt(2026, 8, 29).unwrap()),
            end_price: effect_local.map(|_| 11.0),
            price_effect_local: effect_local,
            price_effect_base: effect_base,
            price_effect_percent: effect_local.map(|value| value / 100.0),
            benchmark_symbol: Some("^GSPC".to_string()),
            benchmark_start_date: Some(NaiveDate::from_ymd_opt(2026, 7, 1).unwrap()),
            benchmark_end_date: Some(NaiveDate::from_ymd_opt(2026, 8, 29).unwrap()),
            benchmark_return: excess.map(|_| 0.05),
            directional_excess_return: excess,
            fact_labels: Vec::new(),
            issues: Vec::new(),
        }
    }

    #[test]
    fn buy_and_add_gain_when_the_stock_rises() {
        for action_type in ["open", "add"] {
            let result =
                calculate_endpoint_effect(&input(action_type, 100.0, 10.0, Some(12.0), 5.0));
            assert_eq!(result.price_effect_local, Some(195.0));
            assert_eq!(result.price_effect_percent, Some(0.195));
        }
    }

    #[test]
    fn reduce_and_close_gain_when_the_stock_falls() {
        for action_type in ["reduce", "close"] {
            let result =
                calculate_endpoint_effect(&input(action_type, 100.0, 10.0, Some(8.0), 5.0));
            assert_eq!(result.price_effect_local, Some(195.0));
            assert_eq!(result.price_effect_percent, Some(0.195));
        }
    }

    #[test]
    fn sell_after_which_the_stock_rises_is_an_opportunity_loss() {
        let result = calculate_endpoint_effect(&input("reduce", 100.0, 10.0, Some(12.0), 5.0));
        assert_eq!(result.price_effect_local, Some(-205.0));
        assert_eq!(result.price_effect_percent, Some(-0.205));
    }

    #[test]
    fn missing_endpoint_price_hides_only_endpoint_effect() {
        let result = calculate_endpoint_effect(&input("open", 100.0, 10.0, None, 5.0));
        assert_eq!(result.price_effect_local, None);
        assert_eq!(result.price_effect_percent, None);
    }

    #[test]
    fn zero_notional_hides_percentage_but_keeps_amount() {
        let mut value = input("open", 100.0, 10.0, Some(12.0), 5.0);
        value.trade_notional_local = 0.0;
        let result = calculate_endpoint_effect(&value);
        assert_eq!(result.price_effect_local, Some(195.0));
        assert_eq!(result.price_effect_percent, None);
    }

    #[test]
    fn non_finite_inputs_are_not_calculated() {
        let result = calculate_endpoint_effect(&input("open", f64::NAN, 10.0, Some(12.0), 5.0));
        assert_eq!(result.price_effect_local, None);
        assert_eq!(result.price_effect_percent, None);
    }

    #[test]
    fn benchmark_excess_is_adjusted_for_operation_direction() {
        assert_close(calculate_directional_excess("open", 0.20, 0.10), 0.10);
        assert_close(calculate_directional_excess("add", 0.05, 0.10), -0.05);
        assert_close(calculate_directional_excess("reduce", -0.20, -0.05), 0.15);
        assert_close(calculate_directional_excess("close", 0.20, 0.05), -0.15);
        assert_eq!(calculate_directional_excess("unknown", 0.20, 0.05), None);
        assert_eq!(calculate_directional_excess("open", f64::NAN, 0.05), None);
    }

    #[test]
    fn summaries_keep_buy_sell_and_position_impact_separate() {
        let mut buy = action(
            "buy",
            "AAPL",
            "open",
            Some(1_000.0),
            Some(100.0),
            Some(100.0),
            Some(0.10),
            Some(0.04),
        );
        buy.quantity = 100.0;
        let mut add = action(
            "add",
            "AAPL",
            "add",
            Some(500.0),
            Some(-50.0),
            Some(-50.0),
            Some(-0.05),
            None,
        );
        add.quantity = 50.0;
        let mut sell = action(
            "sell",
            "AAPL",
            "reduce",
            Some(300.0),
            Some(30.0),
            Some(30.0),
            Some(0.02),
            Some(-0.06),
        );
        sell.quantity = 30.0;

        let summary = summarize_actions(&[buy, add, sell]);
        assert_eq!(summary.total.action_count, 3);
        assert_eq!(summary.total.positive_count, 2);
        assert_eq!(summary.total.negative_count, 1);
        assert_eq!(summary.total.price_effect_base, Some(80.0));
        assert_close(summary.total.positive_notional_ratio, 1_300.0 / 1_800.0);
        assert_close(summary.total.weighted_excess_return, 81.0 / 1_800.0);
        assert_eq!(summary.buys.price_effect_base, Some(50.0));
        assert_close(summary.buys.positive_notional_ratio, 2.0 / 3.0);
        assert_close(summary.buys.weighted_excess_return, 0.05);
        assert_eq!(summary.sells.price_effect_base, Some(30.0));
        assert_close(summary.sells.positive_notional_ratio, 1.0);
        assert_close(summary.sells.weighted_excess_return, 0.02);
        assert_eq!(summary.position_impact.invested_amount_base, Some(1_500.0));
        assert_eq!(summary.position_impact.recovered_amount_base, Some(300.0));
        assert_eq!(
            summary.position_impact.largest_absolute_weight_change,
            Some(0.06)
        );
        assert_eq!(summary.position_impact.total_fees_base, Some(6.0));
        assert_eq!(summary.position_impact.missing_weight_count, 1);
    }

    #[test]
    fn missing_fx_hides_only_aggregate_amounts_that_would_be_partial() {
        let converted = action(
            "converted",
            "AAPL",
            "open",
            Some(1_000.0),
            Some(100.0),
            Some(100.0),
            Some(0.10),
            Some(0.04),
        );
        let mut missing_fx = action(
            "missing-fx",
            "MSFT",
            "add",
            None,
            Some(50.0),
            None,
            Some(0.05),
            Some(0.02),
        );
        missing_fx.fee_base = None;
        let summary = summarize_actions(&[converted, missing_fx]);
        assert_eq!(summary.total.price_effect_base, None);
        assert_eq!(summary.total.positive_notional_ratio, None);
        assert_eq!(summary.total.weighted_excess_return, None);
        assert_eq!(summary.position_impact.invested_amount_base, None);
        assert_eq!(summary.position_impact.total_fees_base, None);
    }

    #[test]
    fn missing_endpoint_is_counted_but_excluded_from_calculable_sum() {
        let calculated = action(
            "calculated",
            "AAPL",
            "open",
            Some(1_000.0),
            Some(100.0),
            Some(100.0),
            Some(0.10),
            Some(0.04),
        );
        let missing = action(
            "missing",
            "MSFT",
            "add",
            Some(500.0),
            None,
            None,
            None,
            Some(0.02),
        );
        let summary = summarize_actions(&[calculated, missing]);
        assert_eq!(summary.total.missing_effect_count, 1);
        assert_eq!(summary.total.price_effect_base, Some(100.0));
        assert_close(summary.total.positive_notional_ratio, 1.0);
        assert_close(summary.total.weighted_excess_return, 0.10);
    }

    #[test]
    fn securities_group_normalized_symbols_per_account() {
        let first = action(
            "first",
            "aapl",
            "open",
            Some(1_000.0),
            Some(100.0),
            Some(100.0),
            Some(0.10),
            Some(0.04),
        );
        let mut second = action(
            "second",
            "AAPL",
            "reduce",
            Some(300.0),
            Some(-20.0),
            Some(-20.0),
            Some(-0.02),
            Some(-0.03),
        );
        second.name = "Apple".to_string();
        let securities = summarize_securities(&[first, second]);
        assert_eq!(securities.len(), 1);
        assert_eq!(securities[0].open_count, 1);
        assert_eq!(securities[0].reduce_count, 1);
        assert_eq!(securities[0].price_effect_base, Some(80.0));
        assert_eq!(securities[0].largest_absolute_weight_change, Some(0.04));
    }
}
use crate::models::stock_operation_review::{
    StockOperationEffect, StockOperationGroupSummary, StockOperationReviewSummary,
    StockOperationSecuritySummary, StockPositionImpactSummary,
};
use crate::services::stock_operation_builder::{normalize_stock_market, normalize_stock_symbol};
use std::cmp::Ordering;
use std::collections::BTreeMap;
