#![allow(dead_code)]

use crate::models::portfolio_alert::{
    AllocationDirection, CategoryAllocation, ConcentrationAlert, PortfolioAlertConfig,
    PortfolioAlertScope, PortfolioAlertScopeKind, PortfolioAlertSnapshot,
};
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone, PartialEq)]
pub struct PortfolioAlertCategoryInput {
    pub id: String,
    pub name: String,
    pub color: String,
    pub icon: String,
    pub sort_order: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PortfolioAlertPositionInput {
    pub account_id: String,
    pub market: String,
    pub symbol: String,
    pub name: String,
    pub category_id: Option<String>,
    pub category_name: String,
    pub category_color: String,
    pub market_value: f64,
    pub is_cash: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum PortfolioAlertCalculation {
    Ready(PortfolioAlertSnapshot),
    Empty,
}

fn normalize_key(value: &str) -> String {
    value.trim().to_ascii_uppercase()
}

fn is_valid_number(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

fn validate_inputs(
    config: &PortfolioAlertConfig,
    categories: &[PortfolioAlertCategoryInput],
    positions: &[PortfolioAlertPositionInput],
) -> Result<(), String> {
    if !is_valid_number(config.deviation_threshold) {
        return Err("deviation_threshold must be finite and non-negative".to_string());
    }
    if !is_valid_number(config.concentration_threshold) {
        return Err("concentration_threshold must be finite and non-negative".to_string());
    }
    for target in &config.targets {
        if !is_valid_number(target.target_percent) {
            return Err(format!(
                "target_percent must be finite and non-negative for category {}",
                target.category_id
            ));
        }
    }
    for position in positions {
        if !is_valid_number(position.market_value) {
            return Err(format!(
                "market_value must be finite and non-negative for symbol {}",
                position.symbol
            ));
        }
    }
    match config.scope.kind {
        PortfolioAlertScopeKind::Overall => {}
        PortfolioAlertScopeKind::Market => {
            if config
                .scope
                .market
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                return Err("market scope requires a market".to_string());
            }
        }
        PortfolioAlertScopeKind::Account => {
            if config
                .scope
                .account_id
                .as_deref()
                .map(str::trim)
                .unwrap_or("")
                .is_empty()
            {
                return Err("account scope requires an account_id".to_string());
            }
        }
    }
    for category in categories {
        if category.id.trim().is_empty() {
            return Err("category id must not be empty".to_string());
        }
    }
    Ok(())
}

fn scope_matches(scope: &PortfolioAlertScope, position: &PortfolioAlertPositionInput) -> bool {
    match scope.kind {
        PortfolioAlertScopeKind::Overall => true,
        PortfolioAlertScopeKind::Market => scope
            .market
            .as_deref()
            .map(normalize_key)
            .is_some_and(|market| market == normalize_key(&position.market)),
        PortfolioAlertScopeKind::Account => scope
            .account_id
            .as_deref()
            .map(str::trim)
            .is_some_and(|account_id| account_id == position.account_id.trim()),
    }
}

fn relative_deviation_percent(current: f64, target: f64) -> Option<f64> {
    if target == 0.0 {
        return None;
    }
    Some((current - target).abs() / target * 100.0)
}

fn build_category_allocation(
    category_id: Option<String>,
    category_name: String,
    category_color: String,
    category_icon: String,
    target_percent: f64,
    current_market_value: f64,
    total_market_value: f64,
    deviation_threshold: f64,
) -> CategoryAllocation {
    let current_percent = current_market_value / total_market_value * 100.0;
    let target_market_value = total_market_value * target_percent / 100.0;
    let rebalance_amount = target_market_value - current_market_value;
    let relative_deviation_percent = relative_deviation_percent(current_market_value, target_market_value);
    let direction = if target_percent == 0.0 {
        (current_market_value > 0.0).then_some(AllocationDirection::Overweight)
    } else if relative_deviation_percent.is_some_and(|value| value > deviation_threshold) {
        Some(if current_market_value > target_market_value {
            AllocationDirection::Overweight
        } else {
            AllocationDirection::Underweight
        })
    } else {
        None
    };

    CategoryAllocation {
        category_id,
        category_name,
        category_color,
        category_icon,
        target_percent,
        current_percent,
        relative_deviation_percent,
        current_market_value,
        target_market_value,
        rebalance_amount,
        direction,
    }
}

pub fn calculate_portfolio_alert_snapshot(
    config: &PortfolioAlertConfig,
    categories: &[PortfolioAlertCategoryInput],
    positions: &[PortfolioAlertPositionInput],
    base_currency: &str,
    evaluated_at: &str,
) -> Result<PortfolioAlertCalculation, String> {
    validate_inputs(config, categories, positions)?;

    let filtered_positions = positions
        .iter()
        .filter(|position| scope_matches(&config.scope, position))
        .collect::<Vec<_>>();
    if filtered_positions.is_empty() {
        return Ok(PortfolioAlertCalculation::Empty);
    }

    let total_market_value: f64 = filtered_positions.iter().map(|position| position.market_value).sum();
    if total_market_value <= 0.0 {
        return Ok(PortfolioAlertCalculation::Empty);
    }

    let settings_category_ids = categories
        .iter()
        .map(|category| category.id.clone())
        .collect::<HashSet<_>>();
    let mut sorted_categories = categories.iter().collect::<Vec<_>>();
    sorted_categories.sort_by(|left, right| {
        left.sort_order
            .cmp(&right.sort_order)
            .then_with(|| left.id.cmp(&right.id))
    });

    let target_percent_by_category = config
        .targets
        .iter()
        .map(|target| (target.category_id.clone(), target.target_percent))
        .collect::<HashMap<_, _>>();

    let mut current_market_value_by_category = HashMap::<String, f64>::new();
    let mut uncategorized_market_value = 0.0;
    for position in filtered_positions.iter().copied() {
        match position
            .category_id
            .as_ref()
            .filter(|category_id| settings_category_ids.contains(*category_id))
        {
            Some(category_id) => {
                *current_market_value_by_category
                    .entry(category_id.clone())
                    .or_insert(0.0) += position.market_value;
            }
            None => {
                uncategorized_market_value += position.market_value;
            }
        }
    }

    let mut category_allocations = sorted_categories
        .into_iter()
        .map(|category| {
            let current_market_value = current_market_value_by_category
                .get(&category.id)
                .copied()
                .unwrap_or(0.0);
            let target_percent = target_percent_by_category
                .get(&category.id)
                .copied()
                .unwrap_or(0.0);
            build_category_allocation(
                Some(category.id.clone()),
                category.name.clone(),
                category.color.clone(),
                category.icon.clone(),
                target_percent,
                current_market_value,
                total_market_value,
                config.deviation_threshold,
            )
        })
        .collect::<Vec<_>>();

    category_allocations.push(build_category_allocation(
        None,
        "未分类".to_string(),
        "#8B8B8B".to_string(),
        String::new(),
        0.0,
        uncategorized_market_value,
        total_market_value,
        config.deviation_threshold,
    ));

    let mut concentration_groups = BTreeMap::<(String, String), ConcentrationAccumulator>::new();
    for position in filtered_positions
        .iter()
        .copied()
        .filter(|position| !position.is_cash)
    {
        let market = normalize_key(&position.market);
        let symbol = normalize_key(&position.symbol);
        let key = (market.clone(), symbol.clone());
        concentration_groups
            .entry(key)
            .and_modify(|accumulator| {
                accumulator.market_value += position.market_value;
            })
            .or_insert_with(|| ConcentrationAccumulator {
                market,
                symbol,
                name: position.name.clone(),
                category_id: position
                    .category_id
                    .as_ref()
                    .filter(|category_id| settings_category_ids.contains(*category_id))
                    .cloned(),
                market_value: position.market_value,
            });
    }

    let mut concentrations = concentration_groups
        .into_values()
        .filter_map(|accumulator| {
            let position_percent = accumulator.market_value / total_market_value * 100.0;
            (position_percent > config.concentration_threshold).then_some(ConcentrationAlert {
                market: accumulator.market,
                symbol: accumulator.symbol.clone(),
                normalized_symbol: accumulator.symbol,
                name: accumulator.name,
                category_id: accumulator.category_id,
                market_value: accumulator.market_value,
                position_percent,
                threshold_percent: config.concentration_threshold,
            })
        })
        .collect::<Vec<_>>();

    concentrations.sort_by(|left, right| {
        right
            .position_percent
            .partial_cmp(&left.position_percent)
            .unwrap_or(Ordering::Equal)
            .then_with(|| left.market.cmp(&right.market))
            .then_with(|| left.symbol.cmp(&right.symbol))
    });

    Ok(PortfolioAlertCalculation::Ready(PortfolioAlertSnapshot {
        config_id: config.id.clone(),
        scope: config.scope.clone(),
        base_currency: base_currency.to_string(),
        evaluated_at: evaluated_at.to_string(),
        total_market_value,
        categories: category_allocations,
        concentrations,
    }))
}

#[derive(Debug, Clone)]
struct ConcentrationAccumulator {
    market: String,
    symbol: String,
    name: String,
    category_id: Option<String>,
    market_value: f64,
}

#[cfg(test)]
mod tests {
    use crate::models::portfolio_alert::{
        AllocationDirection, PortfolioAlertConfig, PortfolioAlertScope, PortfolioAlertScopeKind,
        PortfolioAlertSnapshot,
    };
    use super::{
        calculate_portfolio_alert_snapshot, PortfolioAlertCalculation,
        PortfolioAlertCategoryInput, PortfolioAlertPositionInput,
    };

    fn config_with_targets<const N: usize>(
        deviation_threshold: f64,
        targets: [(&str, f64); N],
    ) -> PortfolioAlertConfig {
        PortfolioAlertConfig {
            id: "config-1".to_string(),
            scope: PortfolioAlertScope {
                kind: PortfolioAlertScopeKind::Overall,
                market: None,
                account_id: None,
            },
            base_currency: "USD".to_string(),
            deviation_threshold,
            concentration_threshold: 20.0,
            is_active: true,
            targets: targets
                .into_iter()
                .map(|(category_id, target_percent)| crate::models::portfolio_alert::PortfolioAlertTarget {
                    category_id: category_id.to_string(),
                    target_percent,
                })
                .collect(),
            last_snapshot: None,
            last_evaluated_at: None,
        }
    }

    fn config_with_concentration(concentration_threshold: f64) -> PortfolioAlertConfig {
        PortfolioAlertConfig {
            concentration_threshold,
            ..config_with_targets(20.0, [("growth", 50.0), ("cash", 50.0)])
        }
    }

    fn category(id: &str, name: &str, color: &str, icon: &str, sort_order: i64) -> PortfolioAlertCategoryInput {
        PortfolioAlertCategoryInput {
            id: id.to_string(),
            name: name.to_string(),
            color: color.to_string(),
            icon: icon.to_string(),
            sort_order,
        }
    }

    fn default_categories() -> Vec<PortfolioAlertCategoryInput> {
        vec![
            category("growth", "Growth", "#00AA00", "growth", 1),
            category("cash", "Cash", "#CCCCCC", "cash", 2),
        ]
    }

    fn value_and_growth_categories() -> Vec<PortfolioAlertCategoryInput> {
        vec![
            category("value", "Value", "#FF0000", "value", 2),
            category("growth", "Growth", "#00AA00", "growth", 1),
        ]
    }

    fn position(
        account_id: &str,
        market: &str,
        symbol: &str,
        name: &str,
        category_id: Option<&str>,
        category_name: &str,
        category_color: &str,
        market_value: f64,
        is_cash: bool,
    ) -> PortfolioAlertPositionInput {
        PortfolioAlertPositionInput {
            account_id: account_id.to_string(),
            market: market.to_string(),
            symbol: symbol.to_string(),
            name: name.to_string(),
            category_id: category_id.map(|value| value.to_string()),
            category_name: category_name.to_string(),
            category_color: category_color.to_string(),
            market_value,
            is_cash,
        }
    }

    fn positions<const N: usize>(
        items: [(&str, f64, bool); N],
    ) -> Vec<PortfolioAlertPositionInput> {
        items
            .into_iter()
            .map(|(symbol, market_value, is_cash)| {
                let (category_id, category_name, category_color) = if is_cash
                    || symbol.eq_ignore_ascii_case("cash")
                {
                    (Some("cash"), "Cash", "#CCCCCC")
                } else {
                    (Some("growth"), "Growth", "#00AA00")
                };
                position(
                    "acct-a",
                    "US",
                    symbol,
                    symbol,
                    category_id,
                    category_name,
                    category_color,
                    market_value,
                    is_cash,
                )
            })
            .collect()
    }

    fn uncategorized_position(market_value: f64) -> Vec<PortfolioAlertPositionInput> {
        vec![position(
            "acct-a",
            "US",
            "misc",
            "Misc",
            None,
            "未分类",
            "#8B8B8B",
            market_value,
            false,
        )]
    }

    fn same_symbol_in_two_accounts(
        market: &str,
        symbol: &str,
        first_market_value: f64,
        second_market_value: f64,
        total_market_value: f64,
    ) -> Vec<PortfolioAlertPositionInput> {
        let residual_cash = total_market_value - first_market_value - second_market_value;
        vec![
            position(
                "acct-a",
                market,
                symbol,
                "Apple",
                Some("growth"),
                "Growth",
                "#00AA00",
                first_market_value,
                false,
            ),
            position(
                "acct-b",
                market,
                &symbol.to_ascii_lowercase(),
                "Apple",
                Some("growth"),
                "Growth",
                "#00AA00",
                second_market_value,
                false,
            ),
            position(
                "acct-a",
                market,
                "cash",
                "Cash",
                Some("cash"),
                "Cash",
                "#CCCCCC",
                residual_cash,
                true,
            ),
        ]
    }

    fn calculate(
        config: &PortfolioAlertConfig,
        categories: &[PortfolioAlertCategoryInput],
        positions: &[PortfolioAlertPositionInput],
    ) -> PortfolioAlertCalculation {
        calculate_portfolio_alert_snapshot(config, categories, positions, "USD", "2026-09-06T00:00:00Z")
            .unwrap()
    }

    fn snapshot(
        config: &PortfolioAlertConfig,
        categories: &[PortfolioAlertCategoryInput],
        positions: &[PortfolioAlertPositionInput],
    ) -> PortfolioAlertSnapshot {
        match calculate(config, categories, positions) {
            PortfolioAlertCalculation::Ready(snapshot) => snapshot,
            PortfolioAlertCalculation::Empty => panic!("expected ready snapshot"),
        }
    }

    fn allocation<'a>(
        snapshot: &'a PortfolioAlertSnapshot,
        category_id: &str,
    ) -> &'a crate::models::portfolio_alert::CategoryAllocation {
        snapshot
            .categories
            .iter()
            .find(|row| row.category_id.as_deref() == Some(category_id))
            .expect("allocation row")
    }

    fn uncategorized_allocation(
        snapshot: &PortfolioAlertSnapshot,
    ) -> &crate::models::portfolio_alert::CategoryAllocation {
        snapshot
            .categories
            .iter()
            .find(|row| row.category_id.is_none())
            .expect("uncategorized row")
    }

    #[test]
    fn allocation_uses_relative_deviation_and_strict_greater_than() {
        let config = config_with_targets(20.0, [("growth", 50.0), ("cash", 50.0)]);
        let categories = default_categories();
        let at_boundary = positions([("growth", 60.0, false), ("cash", 40.0, true)]);
        let beyond = positions([("growth", 60.01, false), ("cash", 39.99, true)]);

        let boundary = snapshot(&config, &categories, &at_boundary);
        let growth = allocation(&boundary, "growth");
        assert_eq!(growth.direction, None);
        assert_eq!(growth.relative_deviation_percent, Some(20.0));
        assert_eq!(growth.rebalance_amount, -10.0);

        let breached = snapshot(&config, &categories, &beyond);
        assert_eq!(
            allocation(&breached, "growth").direction,
            Some(AllocationDirection::Overweight)
        );
    }

    #[test]
    fn positive_value_against_zero_target_is_overweight() {
        let config = config_with_targets(20.0, [("growth", 100.0)]);
        let snapshot = snapshot(&config, &[], &uncategorized_position(1.0));
        let row = uncategorized_allocation(&snapshot);

        assert_eq!(row.target_percent, 0.0);
        assert_eq!(row.relative_deviation_percent, None);
        assert_eq!(row.direction, Some(AllocationDirection::Overweight));
    }

    #[test]
    fn cash_affects_allocation_but_is_excluded_from_concentration() {
        let config = config_with_concentration(20.0);
        let categories = default_categories();
        let snapshot = snapshot(
            &config,
            &categories,
            &[
                position("acct-a", "US", "cash", "Cash", Some("cash"), "Cash", "#CCCCCC", 60.0, true),
                position("acct-a", "US", "growth", "Growth", Some("growth"), "Growth", "#00AA00", 40.0, false),
            ],
        );

        assert_eq!(allocation(&snapshot, "cash").current_percent, 60.0);
        assert_eq!(snapshot.concentrations.len(), 1);
        assert_eq!(snapshot.concentrations[0].position_percent, 40.0);
    }

    #[test]
    fn concentration_aggregates_same_market_and_symbol_across_accounts() {
        let config = config_with_concentration(20.0);
        let categories = default_categories();
        let snapshot = snapshot(
            &config,
            &categories,
            &same_symbol_in_two_accounts("US", "AAPL", 12.0, 13.0, 100.0),
        );

        assert_eq!(snapshot.concentrations[0].symbol, "AAPL");
        assert_eq!(snapshot.concentrations[0].market_value, 25.0);
        assert_eq!(snapshot.concentrations[0].position_percent, 25.0);
    }

    #[test]
    fn empty_positions_or_non_positive_total_return_empty() {
        let config = config_with_targets(20.0, [("growth", 100.0)]);
        let categories = default_categories();

        assert!(matches!(
            calculate(&config, &categories, &[]),
            PortfolioAlertCalculation::Empty
        ));

        assert!(matches!(
            calculate(
                &config,
                &categories,
                &[position(
                    "acct-a",
                    "US",
                    "growth",
                    "Growth",
                    Some("growth"),
                    "Growth",
                    "#00AA00",
                    0.0,
                    false,
                )],
            ),
            PortfolioAlertCalculation::Empty
        ));
    }

    #[test]
    fn zero_target_category_with_zero_current_value_is_normal() {
        let config = config_with_targets(20.0, [("growth", 0.0), ("cash", 100.0)]);
        let categories = default_categories();
        let snapshot = snapshot(
            &config,
            &categories,
            &[position(
                "acct-a",
                "US",
                "cash",
                "Cash",
                Some("cash"),
                "Cash",
                "#CCCCCC",
                100.0,
                true,
            )],
        );

        let growth = allocation(&snapshot, "growth");
        assert_eq!(growth.current_percent, 0.0);
        assert_eq!(growth.direction, None);
        assert_eq!(growth.relative_deviation_percent, None);
    }

    #[test]
    fn positive_target_with_zero_current_value_is_underweight_below_hundred_percent_threshold() {
        let config = config_with_targets(20.0, [("growth", 50.0), ("cash", 50.0)]);
        let categories = default_categories();
        let snapshot = snapshot(
            &config,
            &categories,
            &[position(
                "acct-a",
                "US",
                "cash",
                "Cash",
                Some("cash"),
                "Cash",
                "#CCCCCC",
                100.0,
                true,
            )],
        );

        let growth = allocation(&snapshot, "growth");
        assert_eq!(growth.relative_deviation_percent, Some(100.0));
        assert_eq!(growth.direction, Some(AllocationDirection::Underweight));
    }

    #[test]
    fn rejects_negative_or_non_finite_values() {
        let config = config_with_targets(20.0, [("growth", 100.0)]);
        let categories = default_categories();

        let negative = calculate_portfolio_alert_snapshot(
            &config,
            &categories,
            &[position(
                "acct-a",
                "US",
                "growth",
                "Growth",
                Some("growth"),
                "Growth",
                "#00AA00",
                -1.0,
                false,
            )],
            "USD",
            "2026-09-06T00:00:00Z",
        );
        assert!(negative.is_err());

        let mut finite = config_with_targets(20.0, [("growth", 100.0)]);
        finite.deviation_threshold = f64::NAN;
        let non_finite = calculate_portfolio_alert_snapshot(
            &finite,
            &categories,
            &uncategorized_position(1.0),
            "USD",
            "2026-09-06T00:00:00Z",
        );
        assert!(non_finite.is_err());
    }

    #[test]
    fn categories_are_sorted_by_settings_order_and_deleted_categories_merge_into_uncategorized() {
        let config = config_with_targets(20.0, [("growth", 50.0), ("value", 50.0)]);
        let categories = value_and_growth_categories();
        let positions = [
            position(
                "acct-a",
                "US",
                "value",
                "Value",
                Some("value"),
                "Value",
                "#FF0000",
                40.0,
                false,
            ),
            position(
                "acct-a",
                "US",
                "growth",
                "Growth",
                Some("growth"),
                "Growth",
                "#00AA00",
                40.0,
                false,
            ),
            position(
                "acct-a",
                "US",
                "legacy",
                "Legacy",
                Some("deleted"),
                "Deleted",
                "#999999",
                15.0,
                false,
            ),
            position(
                "acct-a",
                "US",
                "misc",
                "Misc",
                None,
                "未分类",
                "#8B8B8B",
                5.0,
                false,
            ),
        ];
        let snapshot = snapshot(&config, &categories, &positions);

        assert_eq!(
            snapshot
                .categories
                .iter()
                .map(|row| row.category_id.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("growth"), Some("value"), None]
        );
        let uncategorized = uncategorized_allocation(&snapshot);
        assert_eq!(uncategorized.category_name, "未分类");
        assert_eq!(uncategorized.current_market_value, 20.0);
    }

    #[test]
    fn scope_filters_positions_by_account() {
        let mut config = config_with_targets(20.0, [("growth", 50.0), ("cash", 50.0)]);
        let categories = default_categories();
        config.scope = PortfolioAlertScope {
            kind: PortfolioAlertScopeKind::Account,
            market: None,
            account_id: Some("acct-a".to_string()),
        };
        let positions = [
            position(
                "acct-a",
                "US",
                "growth",
                "Growth",
                Some("growth"),
                "Growth",
                "#00AA00",
                60.0,
                false,
            ),
            position(
                "acct-a",
                "US",
                "cash",
                "Cash",
                Some("cash"),
                "Cash",
                "#CCCCCC",
                40.0,
                true,
            ),
            position(
                "acct-b",
                "US",
                "growth",
                "Growth",
                Some("growth"),
                "Growth",
                "#00AA00",
                40.0,
                false,
            ),
            position(
                "acct-b",
                "US",
                "cash",
                "Cash",
                Some("cash"),
                "Cash",
                "#CCCCCC",
                60.0,
                true,
            ),
        ];
        let snapshot = snapshot(&config, &categories, &positions);

        assert_eq!(snapshot.total_market_value, 100.0);
        assert_eq!(allocation(&snapshot, "growth").current_percent, 60.0);
    }

    #[test]
    fn concentration_exact_threshold_is_not_flagged() {
        let config = config_with_concentration(20.0);
        let categories = default_categories();
        let snapshot = snapshot(
            &config,
            &categories,
            &[
                position("acct-a", "US", "growth", "Growth", Some("growth"), "Growth", "#00AA00", 20.0, false),
                position("acct-a", "US", "cash", "Cash", Some("cash"), "Cash", "#CCCCCC", 80.0, true),
            ],
        );

        assert!(snapshot.concentrations.is_empty());
    }
}
