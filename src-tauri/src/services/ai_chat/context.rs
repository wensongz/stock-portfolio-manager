use super::*;

// ─────────────────────────────────────────────────────────────────────────────
// Portfolio context
// ─────────────────────────────────────────────────────────────────────────────

/// Assemble a Markdown snapshot of the current portfolio for the LLM prompt.
///
/// Uses cache-only quotes (no network) and pulls the last year of performance
/// metrics. Every section is guarded so an empty portfolio still yields a
/// short, valid context string rather than an error.
pub async fn build_portfolio_context(
    db: &Database,
    cache: &ExchangeRateCache,
    quote_cache: &QuoteCache,
) -> Result<String, String> {
    let details = build_holding_details_pub(db, quote_cache, true).await?;
    let rates =
        get_cached_rates(cache, db)
            .await
            .unwrap_or_else(|_| crate::models::quote::ExchangeRates {
                usd_cny: 7.2,
                usd_hkd: 7.8,
                cny_hkd: 7.8 / 7.2,
                updated_at: Utc::now().to_rfc3339(),
            });

    // Normalise every holding's market value to USD for cross-currency totals.
    let to_usd = |amount: f64, currency: &str| {
        crate::services::exchange_rate_service::convert_currency(amount, currency, "USD", &rates)
    };
    let total_market_value_usd: f64 = details
        .iter()
        .map(|d| to_usd(d.market_value, &d.currency))
        .sum();
    let total_cost_value_usd: f64 = details
        .iter()
        .map(|d| to_usd(d.cost_value, &d.currency))
        .sum();
    let total_daily_pnl_usd: f64 = details
        .iter()
        .map(|d| to_usd(d.daily_pnl, &d.currency))
        .sum();

    let mut out = String::new();
    out.push_str("# 当前投资组合快照\n\n");

    // ── Overview ───────────────────────────────────────────────────────────
    out.push_str("## 账户总览（单位：USD）\n");
    if details.is_empty() {
        out.push_str("（暂无持仓）\n\n");
    } else {
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
    }

    // ── Holdings table ─────────────────────────────────────────────────────
    out.push_str("## 当前持仓\n");
    out.push_str("| 代码 | 名称 | 市场 | 账户 | 类别 | 持仓 | 均价 | 现价 | 市值(USD) | 盈亏% |\n");
    out.push_str("|------|------|------|------|------|------|------|------|-----------|-------|\n");
    let mut sorted = details.clone();
    sorted.sort_by(|a, b| {
        to_usd(b.market_value, &b.currency)
            .partial_cmp(&to_usd(a.market_value, &a.currency))
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    for d in &sorted {
        let pnl_pct = d.pnl_percent.unwrap_or(0.0);
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {:.4} | {:.4} | {:.4} | {:.2} | {:.2} |\n",
            d.symbol,
            d.name,
            d.market,
            d.account_name,
            d.category_name,
            d.shares,
            d.avg_cost,
            d.current_price,
            to_usd(d.market_value, &d.currency),
            pnl_pct,
        ));
    }
    out.push('\n');

    // ── Recent transactions ────────────────────────────────────────────────
    out.push_str("## 近期交易（最近 20 条）\n");
    match fetch_recent_transactions(db, 20) {
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
    let filter = PerformanceFilter {
        market: None,
        account_id: None,
    };
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

fn fetch_recent_transactions(db: &Database, limit: usize) -> Result<Vec<TxnRow>, String> {
    let conn = db.conn.lock().map_err(|e| e.to_string())?;
    let mut stmt = conn
        .prepare(
            "SELECT traded_at, symbol, name, transaction_type, shares, price, total_amount
             FROM transactions
             ORDER BY traded_at DESC
             LIMIT ?1",
        )
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map(rusqlite::params![limit as i64], |row| {
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
