use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Portfolio context
// ─────────────────────────────────────────────────────────────────────────────

fn render_holdings_context(
    details: &[crate::models::dashboard::HoldingDetail],
    rates: Result<&crate::models::quote::ExchangeRates, &str>,
) -> String {
    let mut out = String::from("# 当前投资组合快照\n\n## 账户总览（单位：USD）\n");
    if details.is_empty() {
        out.push_str("（暂无持仓）\n\n");
    } else if let Ok(rates) = rates {
        let to_usd = |amount: f64, currency: &str| {
            crate::services::exchange_rate_service::convert_currency(amount, currency, "USD", rates)
        };
        let total_market_value_usd = details
            .iter()
            .map(|detail| to_usd(detail.market_value, &detail.currency))
            .sum::<f64>();
        let total_cost_value_usd = details
            .iter()
            .map(|detail| to_usd(detail.cost_value, &detail.currency))
            .sum::<f64>();
        let total_daily_pnl_usd = details
            .iter()
            .map(|detail| to_usd(detail.daily_pnl, &detail.currency))
            .sum::<f64>();
        let total_pnl = total_market_value_usd - total_cost_value_usd;
        let total_pnl_pct = if total_cost_value_usd > 0.0 {
            total_pnl / total_cost_value_usd * 100.0
        } else {
            0.0
        };
        out.push_str(&format!(
            "- 持仓数量：{}\n- 总市值：{:.2}\n- 总成本：{:.2}\n- 累计盈亏：{:.2} ({:.2}%)\n- 当日盈亏：{:.2}\n\n",
            details.len(),
            total_market_value_usd,
            total_cost_value_usd,
            total_pnl,
            total_pnl_pct,
            total_daily_pnl_usd,
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
    if let Ok(rates) = rates {
        sorted.sort_by(|a, b| {
            let to_usd = |amount: f64, currency: &str| {
                crate::services::exchange_rate_service::convert_currency(
                    amount, currency, "USD", rates,
                )
            };
            to_usd(b.market_value, &b.currency)
                .partial_cmp(&to_usd(a.market_value, &a.currency))
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        out.push_str(
            "| 代码 | 名称 | 市场 | 账户 | 类别 | 持仓 | 均价 | 现价 | 市值(USD) | 盈亏% |\n",
        );
        out.push_str(
            "|------|------|------|------|------|------|------|------|-----------|-------|\n",
        );
        for detail in &sorted {
            let market_value_usd = crate::services::exchange_rate_service::convert_currency(
                detail.market_value,
                &detail.currency,
                "USD",
                rates,
            );
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {:.4} | {:.4} | {:.4} | {:.2} | {:.2} |\n",
                detail.symbol,
                detail.name,
                detail.market,
                detail.account_name,
                detail.category_name,
                detail.shares,
                detail.avg_cost,
                detail.current_price,
                market_value_usd,
                detail.pnl_percent.unwrap_or(0.0),
            ));
        }
    } else {
        sorted.sort_by(|a, b| {
            (&a.market, &a.symbol, &a.account_id).cmp(&(&b.market, &b.symbol, &b.account_id))
        });
        out.push_str(
            "| 代码 | 名称 | 市场 | 账户 | 类别 | 持仓 | 均价 | 现价 | 市值(原币) | 盈亏% |\n",
        );
        out.push_str(
            "|------|------|------|------|------|------|------|------|------------|-------|\n",
        );
        for detail in &sorted {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} | {:.4} | {:.4} | {:.4} | {:.2} {} | {:.2} |\n",
                detail.symbol,
                detail.name,
                detail.market,
                detail.account_name,
                detail.category_name,
                detail.shares,
                detail.avg_cost,
                detail.current_price,
                detail.market_value,
                detail.currency,
                detail.pnl_percent.unwrap_or(0.0),
            ));
        }
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
    let rates = get_cached_rates(cache, db).await;
    let mut out = render_holdings_context(
        &details,
        rates.as_ref().map_err(std::string::String::as_str),
    );

    // ── Recent transactions ────────────────────────────────────────────────
    out.push_str("## 近期交易（最近 20 条）\n");
    match fetch_recent_transactions(db, 20, scope) {
        Ok(txns) if !txns.is_empty() => {
            out.push_str("| 日期 | 代码 | 名称 | 类型 | 持仓 | 价格 | 金额 |\n");
            out.push_str("|------|------|------|------|------|------|------|\n");
            for t in &txns {
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {:.4} | {:.4} | {:.2} |\n",
                    t.traded_at,
                    t.symbol,
                    t.name,
                    t.transaction_type,
                    t.shares,
                    t.price,
                    t.total_amount
                ));
            }
            out.push('\n');
        }
        _ => out.push_str("（暂无交易记录）\n\n"),
    }

    // ── Performance metrics (last 1 year) ──────────────────────────────────
    out.push_str("## 绩效指标（近 1 年）\n");
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
}

fn fetch_recent_transactions(
    db: &Database,
    limit: usize,
    scope: Option<&PortfolioScope>,
) -> Result<Vec<TxnRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT traded_at, symbol, name, transaction_type, shares, price, total_amount
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

    #[test]
    fn holdings_context_omits_cross_currency_totals_when_rates_are_unavailable() {
        let details = vec![HoldingDetail {
            id: "holding".to_string(),
            account_id: "acct".to_string(),
            account_name: "账户".to_string(),
            symbol: "600000".to_string(),
            name: "浦发银行".to_string(),
            market: "CN".to_string(),
            category_name: "分红股".to_string(),
            category_color: "#fff".to_string(),
            shares: 100.0,
            avg_cost: 9.0,
            current_price: 10.0,
            market_value: 1000.0,
            cost_value: 900.0,
            pnl: 100.0,
            pnl_percent: Some(11.11),
            daily_pnl: 20.0,
            currency: "CNY".to_string(),
            market_value_usd: 0.0,
        }];

        let rendered = render_holdings_context(&details, Err("offline"));

        assert!(rendered.contains("汇率不可用，已省略跨币种汇总：offline"));
        assert!(rendered.contains("1000.00 CNY"));
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
