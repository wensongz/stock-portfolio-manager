use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Portfolio context
// ─────────────────────────────────────────────────────────────────────────────

fn market_currency(market: Option<&str>) -> Option<&'static str> {
    match market {
        Some("CN") => Some("CNY"),
        Some("HK") => Some("HKD"),
        Some("US") => Some("USD"),
        _ => None,
    }
}

fn uses_market_native_currency(
    details: &[crate::models::dashboard::HoldingDetail],
    market: Option<&str>,
) -> bool {
    market_currency(market)
        .is_some_and(|currency| details.iter().all(|detail| detail.currency == currency))
}

fn render_holdings_context(
    details: &[crate::models::dashboard::HoldingDetail],
    rates: Result<&crate::models::quote::ExchangeRates, &str>,
    market: Option<&str>,
) -> String {
    let currency = market_currency(market).unwrap_or("USD");
    let can_aggregate = uses_market_native_currency(details, market) || rates.is_ok();
    let to_display_currency = |amount: f64, source_currency: &str| {
        if source_currency == currency {
            amount
        } else {
            crate::services::exchange_rate_service::convert_currency(
                amount,
                source_currency,
                currency,
                rates.expect("cross-currency totals require exchange rates"),
            )
        }
    };
    let mut out = format!("# 当前投资组合快照\n\n## 账户总览（单位：{currency}）\n");
    if details.is_empty() {
        out.push_str("（暂无持仓）\n\n");
    } else if can_aggregate {
        let total_market_value = details
            .iter()
            .map(|detail| to_display_currency(detail.market_value, &detail.currency))
            .sum::<f64>();
        let total_cost_value = details
            .iter()
            .map(|detail| to_display_currency(detail.cost_value, &detail.currency))
            .sum::<f64>();
        let total_daily_pnl = details
            .iter()
            .map(|detail| to_display_currency(detail.daily_pnl, &detail.currency))
            .sum::<f64>();
        let total_pnl = total_market_value - total_cost_value;
        let total_pnl_pct = if total_cost_value > 0.0 {
            total_pnl / total_cost_value * 100.0
        } else {
            0.0
        };
        out.push_str(&format!(
            "- 持仓数量：{}\n- 总市值：{:.2}\n- 总成本：{:.2}\n- 累计盈亏：{:.2} ({:.2}%)\n- 当日盈亏：{:.2}\n\n",
            details.len(),
            total_market_value,
            total_cost_value,
            total_pnl,
            total_pnl_pct,
            total_daily_pnl,
        ));
    } else if let Err(error) = rates {
        out.push_str(&format!(
            "- 持仓数量：{}\n- 汇率不可用，已省略跨币种汇总：{}\n\n",
            details.len(),
            error
        ));
    }

    out.push_str("## 当前持仓\n");
    let mut sorted = details.to_vec();
    if can_aggregate {
        sorted.sort_by(|a, b| {
            to_display_currency(b.market_value, &b.currency)
                .partial_cmp(&to_display_currency(a.market_value, &a.currency))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    } else {
        sorted.sort_by(|a, b| {
            (&a.market, &a.symbol, &a.account_id).cmp(&(&b.market, &b.symbol, &b.account_id))
        });
    }
    let value_currency = if can_aggregate { currency } else { "原币" };
    out.push_str(&format!(
        "| 代码 | 名称 | 市场 | 账户 | 类别 | 持仓 | 均价(原币) | 现价(原币) | 市值({value_currency}) | 盈亏% |\n",
    ));
    out.push_str("|------|------|------|------|------|------|------|------|-----------|-------|\n");
    for detail in &sorted {
        let market_value = if can_aggregate {
            format!(
                "{:.2}",
                to_display_currency(detail.market_value, &detail.currency)
            )
        } else {
            format!("{:.2} {}", detail.market_value, detail.currency)
        };
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {:.4} | {:.4} {} | {:.4} {} | {} | {:.2} |\n",
            detail.symbol,
            detail.name,
            detail.market,
            detail.account_name,
            detail.category_name,
            detail.shares,
            detail.avg_cost,
            detail.currency,
            detail.current_price,
            detail.currency,
            market_value,
            detail.pnl_percent.unwrap_or(0.0),
        ));
    }
    out.push('\n');
    out
}

/// Assemble a Markdown snapshot of the current portfolio for the LLM prompt.
///
/// Uses cache-only quotes (no network) and pulls the last year of performance
/// metrics. Every section is guarded so an empty portfolio still yields a
/// short, valid context string rather than an error.
pub async fn build_portfolio_context(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
    scope: Option<&PortfolioScope>,
) -> Result<String, String> {
    let model = PortfolioReadModel::load(db, quote_cache, None, QuoteReadMode::CacheOnly).await?;
    let details: Vec<_> = model
        .holdings()
        .iter()
        .filter(|holding| scope.is_none_or(|scope| scope.matches_holding(holding)))
        .cloned()
        .collect();
    let market = scope.and_then(|scope| scope.market.as_deref());
    // A market snapshot whose holdings already use that market's currency needs
    // no FX lookup, including when no rates have ever been cached.
    let rates = if uses_market_native_currency(&details, market) {
        cache
            .get_stale()
            .ok_or_else(|| "当前市场原币汇总无需汇率".to_string())
    } else {
        get_cached_rates(cache, db).await
    };
    let mut out = render_holdings_context(
        &details,
        rates.as_ref().map_err(std::string::String::as_str),
        market,
    );

    // ── Recent transactions ────────────────────────────────────────────────
    out.push_str("## 近期交易（最近 20 条）\n");
    match fetch_recent_transactions(db, 20, scope) {
        Ok(txns) if !txns.is_empty() => {
            out.push_str("| 日期 | 代码 | 名称 | 类型 | 持仓 | 价格(原币) | 金额(原币) |\n");
            out.push_str("|------|------|------|------|------|------|------|\n");
            for t in &txns {
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {:.4} | {:.4} {} | {:.2} {} |\n",
                    t.traded_at,
                    t.symbol,
                    t.name,
                    t.transaction_type,
                    t.shares,
                    t.price,
                    t.currency,
                    t.total_amount,
                    t.currency
                ));
            }
            out.push('\n');
        }
        _ => out.push_str("（暂无交易记录）\n\n"),
    }

    // ── Performance metrics (last 1 year) ──────────────────────────────────
    let performance_currency = match portfolio_performance_currency(db, scope) {
        Ok(currency) => currency,
        Err(error) => {
            out.push_str(&format!(
                "## 绩效指标（近 1 年）\n（已省略历史绩效：{error}）\n"
            ));
            return Ok(out.trim_end().to_string());
        }
    };
    out.push_str(&format!(
        "## 绩效指标（近 1 年，金额单位：{performance_currency}）\n"
    ));
    let end = Utc::now().date_naive();
    let start = end - Duration::days(365);
    let filter = scope
        .map(PortfolioScope::performance_filter)
        .unwrap_or_default();
    match performance_service::get_performance_summary(db, start, end, &filter) {
        Ok(p) if p.end_value > 0.0 || !p.return_series.is_empty() => {
            let sharpe = p
                .sharpe_ratio
                .map(|value| format!("{:.2}", value))
                .unwrap_or_else(|| "—".to_string());
            out.push_str(&format!(
                "- 期初市值：{:.2}\n- 期末市值：{:.2}\n- 累计收益率：{:.2}%\n- 年化收益率：{:.2}%\n- 累计盈亏：{:.2}\n- 最大回撤：{:.2}%\n- 波动率：{:.2}%\n- 夏普比率：{}\n\n",
                p.start_value,
                p.end_value,
                p.total_return,
                p.annualized_return,
                p.total_pnl,
                p.max_drawdown,
                p.volatility,
                sharpe,
            ));
        }
        _ => out.push_str("（暂无足够的历史数据）\n\n"),
    }

    Ok(out.trim_end().to_string())
}

struct TxnRow {
    traded_at: String,
    symbol: String,
    name: String,
    transaction_type: String,
    shares: f64,
    price: f64,
    total_amount: f64,
    currency: String,
}

fn fetch_recent_transactions(
    db: &Database,
    limit: usize,
    scope: Option<&PortfolioScope>,
) -> Result<Vec<TxnRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT traded_at, symbol, name, transaction_type, shares, price, total_amount, currency
             FROM transactions
             WHERE (?1 IS NULL OR account_id = ?1)
               AND (?2 IS NULL OR market = ?2)
             ORDER BY traded_at DESC
             LIMIT ?3",
        )
        .map_err(|e| e.to_string())?;
    let account_id = scope.and_then(|scope| scope.account_id.as_deref());
    let market = scope.and_then(|scope| scope.market.as_deref());
    let rows = stmt
        .query_map(rusqlite::params![account_id, market, limit as i64], |row| {
            Ok(TxnRow {
                traded_at: row.get(0)?,
                symbol: row.get(1)?,
                name: row.get(2)?,
                transaction_type: row.get(3)?,
                shares: row.get(4)?,
                price: row.get(5)?,
                total_amount: row.get(6)?,
                currency: row.get(7)?,
            })
        })
        .map_err(|e| e.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| e.to_string())?;
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::dashboard::HoldingDetail;

    fn rates() -> crate::models::quote::ExchangeRates {
        crate::models::quote::ExchangeRates {
            usd_cny: 10.0,
            usd_hkd: 20.0,
            cny_hkd: 2.0,
            updated_at: "2026-09-26T00:00:00Z".to_string(),
        }
    }

    fn holding(market: &str, currency: &str) -> HoldingDetail {
        HoldingDetail {
            id: market.to_string(),
            account_id: format!("account-{market}"),
            account_name: "账户".to_string(),
            symbol: format!("stock-{market}"),
            name: format!("持仓-{market}"),
            market: market.to_string(),
            category_name: "分红股".to_string(),
            category_color: "#fff".to_string(),
            shares: 100.0,
            avg_cost: 9.0,
            current_price: 10.0,
            daily_change_percent: None,
            market_value: 1000.0,
            cost_value: 900.0,
            pnl: 100.0,
            pnl_percent: Some(11.11),
            daily_pnl: 20.0,
            currency: currency.to_string(),
            market_value_usd: 0.0,
        }
    }

    #[test]
    fn market_context_uses_native_totals_and_prices_with_or_without_rates() {
        let rates = rates();
        for (market, currency) in [("CN", "CNY"), ("HK", "HKD"), ("US", "USD")] {
            for available_rates in [Ok(&rates), Err("offline")] {
                let rendered = render_holdings_context(
                    &[holding(market, currency)],
                    available_rates,
                    Some(market),
                );
                assert!(
                    rendered.contains(&format!("账户总览（单位：{currency}）")),
                    "{market}: {rendered}"
                );
                assert!(rendered.contains("总市值：1000.00"), "{market}: {rendered}");
                assert!(rendered.contains("总成本：900.00"));
                assert!(rendered.contains("累计盈亏：100.00 (11.11%)"));
                assert!(rendered.contains("当日盈亏：20.00"));
                assert!(rendered.contains(&format!("市值({currency})")));
                assert!(
                    rendered.contains(&format!("9.0000 {currency} | 10.0000 {currency} | 1000.00"))
                );
                assert!(!rendered.contains("汇率不可用"));
            }
        }
    }

    #[test]
    fn full_portfolio_keeps_usd_totals_and_native_price_units() {
        let details = vec![
            holding("CN", "CNY"),
            holding("HK", "HKD"),
            holding("US", "USD"),
        ];
        let rates = rates();
        let rendered = render_holdings_context(&details, Ok(&rates), None);
        assert!(rendered.contains("账户总览（单位：USD）"));
        assert!(rendered.contains("总市值：1150.00"));
        assert!(rendered.contains("总成本：1035.00"));
        assert!(rendered.contains("累计盈亏：115.00 (11.11%)"));
        assert!(rendered.contains("当日盈亏：23.00"));
        assert!(rendered.contains("9.0000 CNY | 10.0000 CNY | 100.00"));
        assert!(rendered.contains("9.0000 HKD | 10.0000 HKD | 50.00"));
        assert!(rendered.find("stock-US").unwrap() < rendered.find("stock-CN").unwrap());
        assert!(rendered.find("stock-CN").unwrap() < rendered.find("stock-HK").unwrap());
    }

    fn database_with_market_holdings() -> (Database, QuoteCache) {
        let db = Database::new(":memory:").unwrap();
        let quote_cache = QuoteCache::new();
        {
            let conn = db.conn.lock().unwrap();
            for (market, currency) in [("CN", "CNY"), ("HK", "HKD"), ("US", "USD")] {
                for suffix in ["a", "b"] {
                    let id = format!("{market}-{suffix}");
                    conn.execute(
                        "INSERT INTO accounts (id, name, market, created_at, updated_at)
                         VALUES (?1, ?1, ?2, '2026-01-01', '2026-01-01')",
                        rusqlite::params![id, market],
                    )
                    .unwrap();
                    conn.execute(
                        "INSERT INTO holdings (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
                         VALUES (?1, ?1, ?1, ?1, ?2, 100, 9, ?3, '2026-01-01', '2026-01-01')",
                        rusqlite::params![id, market, currency],
                    ).unwrap();
                    conn.execute(
                        "INSERT INTO transactions (id, account_id, symbol, name, market, transaction_type, shares, price, total_amount, currency, traded_at, created_at)
                         VALUES (?1, ?1, ?1, ?1, ?2, 'BUY', 100, 9, 900, ?3, '2026-01-01', '2026-01-01')",
                        rusqlite::params![id, market, currency],
                    ).unwrap();
                    quote_cache.set(crate::models::StockQuote {
                        symbol: id,
                        market: market.to_string(),
                        current_price: 10.0,
                        change: 0.2,
                        ..Default::default()
                    });
                }
            }
        }
        (db, quote_cache)
    }

    #[tokio::test]
    async fn market_context_filters_holdings_and_transactions_before_native_totals() {
        let (db, quote_cache) = database_with_market_holdings();
        for with_rates in [true, false] {
            let cache = ExchangeRateCache::new();
            if with_rates {
                cache.set(rates());
            }
            for (market, currency) in [("CN", "CNY"), ("HK", "HKD"), ("US", "USD")] {
                for account_id in [None, Some(format!("{market}-a"))] {
                    let scope = PortfolioScope {
                        market: Some(market.to_string()),
                        account_id: account_id.clone(),
                        ..Default::default()
                    };
                    let rendered = build_portfolio_context(&db, &cache, &quote_cache, Some(&scope))
                        .await
                        .unwrap();
                    let expected_total = if account_id.is_some() {
                        "1000.00"
                    } else {
                        "2000.00"
                    };
                    assert!(
                        rendered.contains(&format!("总市值：{expected_total}")),
                        "{market}: {rendered}"
                    );
                    assert!(rendered.contains(&format!("账户总览（单位：{currency}）")));
                    assert!(rendered.contains(&format!("9.0000 {currency} | 900.00 {currency}")));
                    assert!(
                        rendered.contains(&format!("绩效指标（近 1 年，金额单位：{currency}）"))
                    );
                    for other in ["CN", "HK", "US"] {
                        if other != market {
                            assert!(!rendered.contains(&format!("{other}-a")));
                            assert!(!rendered.contains(&format!("{other}-b")));
                        }
                    }
                    if account_id.is_some() {
                        assert!(!rendered.contains(&format!("{market}-b")));
                    }
                }
            }
            if !with_rates {
                assert!(cache.get_stale().is_none());
            }
        }
    }

    #[tokio::test]
    async fn full_portfolio_context_keeps_all_markets_and_usd() {
        let (db, quote_cache) = database_with_market_holdings();
        let cache = ExchangeRateCache::new();
        cache.set(rates());
        let rendered = build_portfolio_context(&db, &cache, &quote_cache, None)
            .await
            .unwrap();
        assert!(rendered.contains("账户总览（单位：USD）"));
        assert!(rendered.contains("持仓数量：6"));
        assert!(rendered.contains("总市值：2300.00"));
        assert!(rendered.contains("总成本：2070.00"));
        assert!(rendered.contains("绩效指标（近 1 年，金额单位：USD）"));
        for (market, currency) in [("CN", "CNY"), ("HK", "HKD"), ("US", "USD")] {
            assert!(rendered.contains(&format!("{market}-a")));
            assert!(rendered.contains(&format!("{market}-b")));
            assert!(rendered.contains(&format!("9.0000 {currency} | 900.00 {currency}")));
        }
    }

    #[tokio::test]
    async fn market_context_omits_unconverted_history_for_foreign_currency_cash() {
        let (db, quotes) = database_with_market_holdings();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO holdings (id, account_id, symbol, name, market, shares, avg_cost, currency, created_at, updated_at)
                 VALUES ('foreign-cash', 'HK-a', '$CASH-USD', '美元现金', 'HK', 100, 1, 'USD', '2026-01-01', '2026-01-01')",
                [],
            ).unwrap();
            conn.execute(
                "INSERT INTO daily_holding_snapshots (date, account_id, symbol, market, shares, close_price, market_value)
                 VALUES (?1, 'HK-a', '$CASH-USD', 'HK', 100, 1, 100)",
                [Utc::now().date_naive().to_string()],
            ).unwrap();
        }
        let cache = ExchangeRateCache::new();
        cache.set(rates());
        let scope = PortfolioScope {
            market: Some("HK".to_string()),
            ..Default::default()
        };
        let rendered = build_portfolio_context(&db, &cache, &quotes, Some(&scope))
            .await
            .unwrap();
        assert!(rendered.contains("账户总览（单位：HKD）"));
        assert!(rendered.contains("总市值：4000.00"));
        assert!(!rendered.contains("期末市值："), "{rendered}");
        assert!(rendered.contains("历史绩效"));
        assert!(rendered.contains("币种"));
    }

    #[test]
    fn market_context_converts_foreign_currency_holdings_or_keeps_native_units() {
        let details = vec![holding("CN", "HKD")];
        let rates = rates();
        let converted = render_holdings_context(&details, Ok(&rates), Some("CN"));
        assert!(converted.contains("总市值：500.00"));
        assert!(converted.contains("市值(CNY)"));
        assert!(converted.contains("9.0000 HKD | 10.0000 HKD | 500.00"));

        let unavailable = render_holdings_context(&details, Err("offline"), Some("CN"));
        assert!(unavailable.contains("汇率不可用，已省略跨币种汇总：offline"));
        assert!(unavailable.contains("1000.00 HKD"));
        assert!(!unavailable.contains("总市值："));
        assert!(!unavailable.contains("市值(CNY)"));
    }

    #[test]
    fn holdings_context_omits_cross_currency_totals_when_rates_are_unavailable() {
        let details = vec![
            holding("CN", "CNY"),
            holding("HK", "HKD"),
            holding("US", "USD"),
        ];

        let rendered = render_holdings_context(&details, Err("offline"), None);

        assert!(rendered.contains("汇率不可用，已省略跨币种汇总：offline"));
        assert!(rendered.contains("1000.00 CNY"));
        assert!(rendered.contains("1000.00 HKD"));
        assert!(rendered.contains("1000.00 USD"));
        assert!(!rendered.contains("总市值："));
        assert!(!rendered.contains("市值(USD)"));
    }

    #[test]
    fn portfolio_scope_matches_only_the_selected_market_or_account() {
        let holding = HoldingDetail {
            id: "holding".to_string(),
            account_id: "account-a".to_string(),
            account_name: "长期账户".to_string(),
            symbol: "600000".to_string(),
            name: "浦发银行".to_string(),
            market: "CN".to_string(),
            category_name: "分红股".to_string(),
            category_color: "#fff".to_string(),
            shares: 100.0,
            avg_cost: 9.0,
            current_price: 10.0,
            daily_change_percent: None,
            market_value: 1000.0,
            cost_value: 900.0,
            pnl: 100.0,
            pnl_percent: Some(11.11),
            daily_pnl: 20.0,
            currency: "CNY".to_string(),
            market_value_usd: 0.0,
        };

        assert!(PortfolioScope::default().matches_holding(&holding));
        assert!(PortfolioScope {
            market: Some("CN".to_string()),
            account_id: None,
            ..PortfolioScope::default()
        }
        .matches_holding(&holding));
        assert!(!PortfolioScope {
            market: Some("US".to_string()),
            account_id: None,
            ..PortfolioScope::default()
        }
        .matches_holding(&holding));
        assert!(PortfolioScope {
            market: None,
            account_id: Some("account-a".to_string()),
            ..PortfolioScope::default()
        }
        .matches_holding(&holding));
        assert!(!PortfolioScope {
            market: None,
            account_id: Some("account-b".to_string()),
            ..PortfolioScope::default()
        }
        .matches_holding(&holding));
    }
}
