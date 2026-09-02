use crate::models::{
    AccountStatistics, CategoryStatistics, ExchangeRates, MarketStatistics, PieSlice, PnlItem,
    StatisticsOverview,
};
use crate::services::exchange_rate_service::convert_currency;
use crate::services::portfolio_read_service::PortfolioReadModel;

pub fn overview(
    model: &PortfolioReadModel,
    rates: &ExchangeRates,
    base_currency: &str,
) -> StatisticsOverview {
    let details = model.holdings();
    let mut market_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut category_map: std::collections::HashMap<(String, Option<String>), f64> =
        std::collections::HashMap::new();
    let mut account_map: std::collections::HashMap<(String, String), f64> =
        std::collections::HashMap::new();
    let mut stock_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut total_market_value = 0.0;
    let mut total_cost = 0.0;

    for detail in details {
        let market_value =
            convert_currency(detail.market_value, &detail.currency, base_currency, rates);
        let cost_value =
            convert_currency(detail.cost_value, &detail.currency, base_currency, rates);
        *market_map.entry(detail.market.clone()).or_insert(0.0) += market_value;
        *category_map
            .entry((
                detail.category_name.clone(),
                Some(detail.category_color.clone()),
            ))
            .or_insert(0.0) += market_value;
        *account_map
            .entry((detail.account_id.clone(), detail.account_name.clone()))
            .or_insert(0.0) += market_value;
        *stock_map
            .entry(format!("{} {}", detail.symbol, detail.name))
            .or_insert(0.0) += market_value;
        total_market_value += market_value;
        total_cost += cost_value;
    }

    let market_label = |market: &str| match market {
        "US" => "🇺🇸 美股".to_string(),
        "CN" => "🇨🇳 A股".to_string(),
        "HK" => "🇭🇰 港股".to_string(),
        _ => market.to_string(),
    };
    let mut market_distribution: Vec<PieSlice> = market_map
        .into_iter()
        .map(|(name, value)| PieSlice {
            name: market_label(&name),
            value,
            color: None,
        })
        .collect();
    market_distribution.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut category_distribution: Vec<PieSlice> = category_map
        .into_iter()
        .map(|((name, color), value)| PieSlice { name, value, color })
        .collect();
    category_distribution.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut account_distribution: Vec<PieSlice> = account_map
        .into_iter()
        .map(|((_, name), value)| PieSlice {
            name,
            value,
            color: None,
        })
        .collect();
    account_distribution.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut stock_distribution: Vec<PieSlice> = stock_map
        .into_iter()
        .map(|(name, value)| PieSlice {
            name,
            value,
            color: None,
        })
        .collect();
    stock_distribution.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    struct SymbolAgg {
        name: String,
        pnl_base: f64,
        cost_base: f64,
        market_value_base: f64,
    }
    let mut symbol_map: std::collections::HashMap<String, SymbolAgg> =
        std::collections::HashMap::new();
    for detail in details {
        let pnl_base = convert_currency(detail.pnl, &detail.currency, base_currency, rates);
        let cost_base = convert_currency(detail.cost_value, &detail.currency, base_currency, rates);
        let market_value_base =
            convert_currency(detail.market_value, &detail.currency, base_currency, rates);
        let entry = symbol_map
            .entry(detail.symbol.clone())
            .or_insert_with(|| SymbolAgg {
                name: detail.name.clone(),
                pnl_base: 0.0,
                cost_base: 0.0,
                market_value_base: 0.0,
            });
        entry.pnl_base += pnl_base;
        entry.cost_base += cost_base;
        entry.market_value_base += market_value_base;
    }
    let mut pnl_items: Vec<PnlItem> = symbol_map
        .into_iter()
        .map(|(symbol, aggregate)| PnlItem {
            symbol,
            name: aggregate.name,
            pnl: aggregate.pnl_base,
            pnl_percent: if aggregate.cost_base > 0.0 {
                Some(aggregate.pnl_base / aggregate.cost_base * 100.0)
            } else {
                None
            },
            market_value: aggregate.market_value_base,
        })
        .collect();
    pnl_items.sort_by(|a, b| {
        b.pnl
            .partial_cmp(&a.pnl)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let top_gainers = pnl_items
        .iter()
        .filter(|item| item.pnl > 0.0)
        .take(5)
        .cloned()
        .collect();
    let top_losers = pnl_items
        .iter()
        .rev()
        .filter(|item| item.pnl < 0.0)
        .take(5)
        .cloned()
        .collect();
    let total_pnl = total_market_value - total_cost;
    let total_pnl_percent = if total_cost != 0.0 {
        total_pnl / total_cost * 100.0
    } else {
        0.0
    };

    StatisticsOverview {
        total_market_value,
        total_cost,
        total_pnl,
        total_pnl_percent,
        market_distribution,
        category_distribution,
        account_distribution,
        stock_distribution,
        top_gainers,
        top_losers,
        holdings: model.holdings_with_usd(rates),
    }
}

pub fn by_market(model: &PortfolioReadModel, market: &str) -> MarketStatistics {
    let details: Vec<_> = model
        .holdings()
        .iter()
        .filter(|detail| detail.market == market)
        .cloned()
        .collect();
    let mut account_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut category_map: std::collections::HashMap<(String, Option<String>), f64> =
        std::collections::HashMap::new();
    let mut stock_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut total_market_value = 0.0;
    let mut total_cost = 0.0;

    for detail in &details {
        *account_map
            .entry(detail.account_name.clone())
            .or_insert(0.0) += detail.market_value;
        *category_map
            .entry((
                detail.category_name.clone(),
                Some(detail.category_color.clone()),
            ))
            .or_insert(0.0) += detail.market_value;
        *stock_map
            .entry(format!("{} {}", detail.symbol, detail.name))
            .or_insert(0.0) += detail.market_value;
        total_market_value += detail.market_value;
        total_cost += detail.cost_value;
    }

    let mut account_distribution: Vec<PieSlice> = account_map
        .into_iter()
        .map(|(name, value)| PieSlice {
            name,
            value,
            color: None,
        })
        .collect();
    account_distribution.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut category_distribution: Vec<PieSlice> = category_map
        .into_iter()
        .map(|((name, color), value)| PieSlice { name, value, color })
        .collect();
    category_distribution.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut stock_distribution: Vec<PieSlice> = stock_map
        .into_iter()
        .map(|(name, value)| PieSlice {
            name,
            value,
            color: None,
        })
        .collect();
    stock_distribution.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let total_pnl = total_market_value - total_cost;
    let total_pnl_percent = if total_cost != 0.0 {
        total_pnl / total_cost * 100.0
    } else {
        0.0
    };

    MarketStatistics {
        market: market.to_string(),
        total_market_value,
        total_cost,
        total_pnl,
        total_pnl_percent,
        account_distribution,
        category_distribution,
        stock_distribution,
        holdings: details,
    }
}

pub fn by_account(model: &PortfolioReadModel, account_id: &str) -> AccountStatistics {
    let details: Vec<_> = model
        .holdings()
        .iter()
        .filter(|detail| detail.account_id == account_id)
        .cloned()
        .collect();
    let account_name = details
        .first()
        .map(|detail| detail.account_name.clone())
        .unwrap_or_default();
    let market = details
        .first()
        .map(|detail| detail.market.clone())
        .unwrap_or_default();
    let mut category_map: std::collections::HashMap<(String, Option<String>), f64> =
        std::collections::HashMap::new();
    let mut stock_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut total_market_value = 0.0;
    let mut total_cost = 0.0;

    for detail in &details {
        *category_map
            .entry((
                detail.category_name.clone(),
                Some(detail.category_color.clone()),
            ))
            .or_insert(0.0) += detail.market_value;
        *stock_map
            .entry(format!("{} {}", detail.symbol, detail.name))
            .or_insert(0.0) += detail.market_value;
        total_market_value += detail.market_value;
        total_cost += detail.cost_value;
    }

    let mut category_distribution: Vec<PieSlice> = category_map
        .into_iter()
        .map(|((name, color), value)| PieSlice { name, value, color })
        .collect();
    category_distribution.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let mut stock_distribution: Vec<PieSlice> = stock_map
        .into_iter()
        .map(|(name, value)| PieSlice {
            name,
            value,
            color: None,
        })
        .collect();
    stock_distribution.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let total_pnl = total_market_value - total_cost;
    let total_pnl_percent = if total_cost != 0.0 {
        total_pnl / total_cost * 100.0
    } else {
        0.0
    };

    AccountStatistics {
        account_id: account_id.to_string(),
        account_name,
        market,
        total_market_value,
        total_cost,
        total_pnl,
        total_pnl_percent,
        category_distribution,
        stock_distribution,
        holdings: details,
    }
}

pub fn by_category(
    model: &PortfolioReadModel,
    rates: &ExchangeRates,
    base_currency: &str,
    category_id: &str,
    category_name: &str,
    category_color: &str,
) -> CategoryStatistics {
    let details: Vec<_> = model
        .holdings_with_usd(rates)
        .into_iter()
        .filter(|detail| detail.category_name == category_name)
        .collect();
    let mut market_map: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut total_market_value = 0.0;
    let mut total_cost = 0.0;

    for detail in &details {
        let market_value =
            convert_currency(detail.market_value, &detail.currency, base_currency, rates);
        let cost_value =
            convert_currency(detail.cost_value, &detail.currency, base_currency, rates);
        let market_label = match detail.market.as_str() {
            "US" => "🇺🇸 美股",
            "CN" => "🇨🇳 A股",
            "HK" => "🇭🇰 港股",
            _ => detail.market.as_str(),
        };
        *market_map.entry(market_label.to_string()).or_insert(0.0) += market_value;
        total_market_value += market_value;
        total_cost += cost_value;
    }

    let mut market_distribution: Vec<PieSlice> = market_map
        .into_iter()
        .map(|(name, value)| PieSlice {
            name,
            value,
            color: None,
        })
        .collect();
    market_distribution.sort_by(|a, b| {
        b.value
            .partial_cmp(&a.value)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    let total_pnl = total_market_value - total_cost;
    let total_pnl_percent = if total_cost != 0.0 {
        total_pnl / total_cost * 100.0
    } else {
        0.0
    };

    CategoryStatistics {
        category_id: category_id.to_string(),
        category_name: category_name.to_string(),
        category_color: category_color.to_string(),
        total_market_value,
        total_cost,
        total_pnl,
        total_pnl_percent,
        market_distribution,
        holdings: details,
    }
}

#[cfg(test)]
mod tests {
    use super::{by_account, by_category, by_market, overview};
    use crate::models::{ExchangeRates, HoldingDetail};
    use crate::services::portfolio_read_service::PortfolioReadModel;

    #[allow(clippy::too_many_arguments)]
    fn holding(
        id: &str,
        account_id: &str,
        account_name: &str,
        symbol: &str,
        market: &str,
        shares: f64,
        avg_cost: f64,
        current_price: f64,
        currency: &str,
    ) -> HoldingDetail {
        let market_value = shares * current_price;
        let cost_value = shares * avg_cost;
        let pnl = market_value - cost_value;
        HoldingDetail {
            id: id.to_string(),
            account_id: account_id.to_string(),
            account_name: account_name.to_string(),
            symbol: symbol.to_string(),
            name: symbol.to_string(),
            market: market.to_string(),
            category_name: "成长".to_string(),
            category_color: "#1677ff".to_string(),
            shares,
            avg_cost,
            current_price,
            market_value,
            cost_value,
            pnl,
            pnl_percent: Some(pnl / cost_value * 100.0),
            daily_pnl: 0.0,
            currency: currency.to_string(),
            market_value_usd: market_value,
        }
    }

    fn fixture() -> (PortfolioReadModel, ExchangeRates) {
        let model = PortfolioReadModel::from_holdings_for_test(vec![
            holding(
                "h-us",
                "acct-us",
                "US Broker",
                "AAPL",
                "US",
                10.0,
                10.0,
                12.0,
                "USD",
            ),
            holding(
                "h-cn",
                "acct-cn",
                "CN Broker",
                "600519",
                "CN",
                100.0,
                8.0,
                10.0,
                "CNY",
            ),
        ]);
        let rates = ExchangeRates {
            usd_cny: 5.0,
            usd_hkd: 7.8,
            cny_hkd: 1.56,
            updated_at: "2026-09-02T09:30:00Z".to_string(),
        };
        (model, rates)
    }

    fn assert_close(actual: f64, expected: f64) {
        assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
    }

    fn with_category(
        mut detail: HoldingDetail,
        category_name: &str,
        category_color: &str,
    ) -> HoldingDetail {
        detail.category_name = category_name.to_string();
        detail.category_color = category_color.to_string();
        detail
    }

    fn ranked_holding(
        id: &str,
        account_id: &str,
        symbol: &str,
        cost: f64,
        pnl: f64,
    ) -> HoldingDetail {
        let mut detail = holding(
            id,
            account_id,
            account_id,
            symbol,
            "US",
            1.0,
            cost,
            cost + pnl,
            "USD",
        );
        if cost == 0.0 {
            detail.pnl_percent = None;
        }
        detail
    }

    #[test]
    fn overview_preserves_cross_currency_totals_and_exposes_same_holdings() {
        let (model, rates) = fixture();
        let result = overview(&model, &rates, "USD");

        assert_close(result.total_market_value, 320.0);
        assert_close(result.total_cost, 260.0);
        assert_close(result.total_pnl, 60.0);
        assert_close(result.total_pnl_percent, 60.0 / 260.0 * 100.0);
        assert_eq!(result.market_distribution[0].name, "🇨🇳 A股");
        assert_close(result.market_distribution[0].value, 200.0);
        assert_eq!(result.market_distribution[1].name, "🇺🇸 美股");
        assert_close(result.market_distribution[1].value, 120.0);
        assert_eq!(result.holdings.len(), 2);
        let cn = result
            .holdings
            .iter()
            .find(|item| item.market == "CN")
            .unwrap();
        assert_close(cn.market_value_usd, 200.0);
    }

    #[test]
    fn market_statistics_keep_native_currency_values() {
        let (model, _rates) = fixture();
        let result = by_market(&model, "US");

        assert_eq!(result.market, "US");
        assert_close(result.total_market_value, 120.0);
        assert_close(result.total_cost, 100.0);
        assert_close(result.total_pnl, 20.0);
        assert_eq!(result.holdings.len(), 1);
        assert_eq!(result.holdings[0].symbol, "AAPL");
    }

    #[test]
    fn account_statistics_keep_identity_distributions_and_holdings() {
        let (model, _rates) = fixture();
        let result = by_account(&model, "acct-us");

        assert_eq!(result.account_name, "US Broker");
        assert_eq!(result.market, "US");
        assert_close(result.total_market_value, 120.0);
        assert_close(result.total_cost, 100.0);
        assert_eq!(result.category_distribution[0].name, "成长");
        assert_close(result.category_distribution[0].value, 120.0);
        assert_eq!(result.stock_distribution[0].name, "AAPL AAPL");
        assert_eq!(result.holdings.len(), 1);
    }

    #[test]
    fn category_statistics_use_base_currency_and_usd_holdings() {
        let (model, rates) = fixture();
        let result = by_category(&model, &rates, "USD", "growth", "成长", "#1677ff");

        assert_eq!(result.category_id, "growth");
        assert_eq!(result.category_name, "成长");
        assert_eq!(result.category_color, "#1677ff");
        assert_close(result.total_market_value, 320.0);
        assert_close(result.total_cost, 260.0);
        assert_eq!(result.market_distribution[0].name, "🇨🇳 A股");
        assert_close(result.market_distribution[0].value, 200.0);
        assert_eq!(result.market_distribution[1].name, "🇺🇸 美股");
        assert_close(result.market_distribution[1].value, 120.0);
        assert_eq!(result.holdings.len(), 2);
        let cn = result
            .holdings
            .iter()
            .find(|item| item.market == "CN")
            .unwrap();
        assert_close(cn.market_value_usd, 200.0);
    }

    #[test]
    fn distributions_preserve_multiple_markets_accounts_and_categories() {
        let model = PortfolioReadModel::from_holdings_for_test(vec![
            with_category(
                holding(
                    "us-growth",
                    "acct-us-growth",
                    "US Growth",
                    "USG",
                    "US",
                    1.0,
                    10.0,
                    20.0,
                    "USD",
                ),
                "成长",
                "#1677ff",
            ),
            with_category(
                holding(
                    "us-income",
                    "acct-us-income",
                    "US Income",
                    "USI",
                    "US",
                    1.0,
                    5.0,
                    15.0,
                    "USD",
                ),
                "收益",
                "#52c41a",
            ),
            with_category(
                holding(
                    "cn-growth",
                    "acct-cn",
                    "CN Broker",
                    "CNG",
                    "CN",
                    1.0,
                    10.0,
                    20.0,
                    "CNY",
                ),
                "成长",
                "#1677ff",
            ),
            with_category(
                holding(
                    "hk-income",
                    "acct-hk",
                    "HK Broker",
                    "HKI",
                    "HK",
                    1.0,
                    4.0,
                    12.0,
                    "HKD",
                ),
                "收益",
                "#52c41a",
            ),
        ]);
        let rates = ExchangeRates {
            usd_cny: 2.0,
            usd_hkd: 4.0,
            cny_hkd: 2.0,
            updated_at: "2026-09-02T09:30:00Z".to_string(),
        };

        let result = overview(&model, &rates, "USD");

        assert_close(result.total_market_value, 48.0);
        assert_close(result.total_cost, 21.0);
        assert_close(result.total_pnl, 27.0);
        assert_eq!(
            result
                .market_distribution
                .iter()
                .map(|slice| (slice.name.as_str(), slice.value))
                .collect::<Vec<_>>(),
            vec![("🇺🇸 美股", 35.0), ("🇨🇳 A股", 10.0), ("🇭🇰 港股", 3.0)]
        );
        assert_eq!(
            result
                .category_distribution
                .iter()
                .map(|slice| (slice.name.as_str(), slice.value, slice.color.as_deref()))
                .collect::<Vec<_>>(),
            vec![
                ("成长", 30.0, Some("#1677ff")),
                ("收益", 18.0, Some("#52c41a")),
            ]
        );
        assert_eq!(
            result
                .account_distribution
                .iter()
                .map(|slice| (slice.name.as_str(), slice.value))
                .collect::<Vec<_>>(),
            vec![
                ("US Growth", 20.0),
                ("US Income", 15.0),
                ("CN Broker", 10.0),
                ("HK Broker", 3.0),
            ]
        );

        let us = by_market(&model, "US");
        assert_close(us.total_market_value, 35.0);
        assert_close(us.total_cost, 15.0);
        assert_eq!(us.account_distribution[0].name, "US Growth");
        assert_close(us.account_distribution[0].value, 20.0);

        let growth = by_category(&model, &rates, "USD", "growth", "成长", "#1677ff");
        assert_close(growth.total_market_value, 30.0);
        assert_close(growth.total_cost, 15.0);
        assert_eq!(growth.holdings.len(), 2);
        let cn = growth
            .holdings
            .iter()
            .find(|holding| holding.market == "CN")
            .unwrap();
        assert_close(cn.market_value, 20.0);
        assert_close(cn.market_value_usd, 10.0);
    }

    #[test]
    fn overview_aggregates_symbols_and_filters_sorted_top_five_by_sign() {
        let model = PortfolioReadModel::from_holdings_for_test(vec![
            ranked_holding("dup-a", "acct-a", "DUP", 100.0, 8.0),
            ranked_holding("dup-b", "acct-b", "DUP", 100.0, 7.0),
            ranked_holding("g2", "acct-a", "G2", 100.0, 14.0),
            ranked_holding("g3", "acct-a", "G3", 100.0, 13.0),
            ranked_holding("g4", "acct-a", "G4", 100.0, 12.0),
            ranked_holding("g5", "acct-a", "G5", 100.0, 11.0),
            ranked_holding("g6", "acct-a", "G6", 100.0, 10.0),
            ranked_holding("free", "acct-a", "FREE", 0.0, 16.0),
            ranked_holding("l1", "acct-a", "L1", 100.0, -15.0),
            ranked_holding("l2", "acct-a", "L2", 100.0, -14.0),
            ranked_holding("l3", "acct-a", "L3", 100.0, -13.0),
            ranked_holding("l4", "acct-a", "L4", 100.0, -12.0),
            ranked_holding("l5", "acct-a", "L5", 100.0, -11.0),
            ranked_holding("l6", "acct-a", "L6", 100.0, -10.0),
        ]);
        let rates = ExchangeRates {
            usd_cny: 5.0,
            usd_hkd: 7.8,
            cny_hkd: 1.56,
            updated_at: "2026-09-02T09:30:00Z".to_string(),
        };

        let result = overview(&model, &rates, "USD");

        assert_eq!(
            result
                .top_gainers
                .iter()
                .map(|item| item.symbol.as_str())
                .collect::<Vec<_>>(),
            vec!["FREE", "DUP", "G2", "G3", "G4"]
        );
        assert_eq!(
            result
                .top_losers
                .iter()
                .map(|item| item.symbol.as_str())
                .collect::<Vec<_>>(),
            vec!["L1", "L2", "L3", "L4", "L5"]
        );
        assert!(result.top_gainers.iter().all(|item| item.pnl > 0.0));
        assert!(result.top_losers.iter().all(|item| item.pnl < 0.0));

        let free = &result.top_gainers[0];
        assert_close(free.pnl, 16.0);
        assert_eq!(free.pnl_percent, None);
        assert_close(free.market_value, 16.0);

        let duplicate = &result.top_gainers[1];
        assert_close(duplicate.pnl, 15.0);
        assert_eq!(duplicate.pnl_percent, Some(7.5));
        assert_close(duplicate.market_value, 215.0);

        let largest_loss = &result.top_losers[0];
        assert_close(largest_loss.pnl, -15.0);
        assert_eq!(largest_loss.pnl_percent, Some(-15.0));
        assert_close(largest_loss.market_value, 85.0);
    }
}
