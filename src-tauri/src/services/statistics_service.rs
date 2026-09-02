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
}
