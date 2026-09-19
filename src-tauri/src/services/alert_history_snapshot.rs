//! Human-readable values frozen at the moment an alert triggers.
//! Names are copied into history so later renames/deletions cannot rewrite it.
use crate::models::{
    alert::PriceAlert,
    alert_history::{AlertHistoryDetail, AlertHistoryMessage},
    portfolio_alert::{
        PortfolioAlertBreach, PortfolioAlertBreachKind, PortfolioAlertConfig,
        PortfolioAlertScopeKind, PortfolioAlertSnapshot,
    },
};
use crate::services::portfolio_alert_calculator::PortfolioAlertPositionInput;
use rusqlite::{Connection, OptionalExtension};
use std::collections::{BTreeSet, HashMap};
use uuid::Uuid;

fn detail(label: &str, value: impl Into<String>) -> AlertHistoryDetail {
    AlertHistoryDetail {
        label: label.into(),
        value: value.into(),
    }
}

fn market_name(market: &str) -> &str {
    match market {
        "US" => "美股",
        "HK" => "港股",
        "CN" => "A股",
        other => other,
    }
}

fn market_currency(market: &str) -> &str {
    match market {
        "HK" => "HKD",
        "CN" => "CNY",
        _ => "USD",
    }
}

fn amount(value: f64, currency: &str) -> String {
    format!("{value:.2} {currency}")
}

pub(super) fn price_alert_history(
    connection: &Connection,
    alert: &PriceAlert,
    current_value: f64,
    quote: (f64, f64, f64),
    triggered_at: &str,
) -> Result<AlertHistoryMessage, String> {
    let holding = connection
        .query_row(
            "SELECT a.name, h.shares, h.avg_cost, h.currency, c.name
         FROM holdings h JOIN accounts a ON a.id = h.account_id
         LEFT JOIN categories c ON c.id = h.category_id WHERE h.id = ?1",
            [alert.holding_id.as_deref()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, f64>(1)?,
                    row.get::<_, f64>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let account_name = holding.as_ref().map(|row| row.0.clone());
    let scope_name = account_name
        .as_ref()
        .map(|name| format!("账户：{name}"))
        .unwrap_or_else(|| "未关联账户".into());
    let currency = holding
        .as_ref()
        .map(|row| row.3.as_str())
        .unwrap_or_else(|| market_currency(&alert.market));
    let (metric, condition, is_percent) = match alert.alert_type.as_str() {
        "PRICE_ABOVE" => ("价格", "超过", false),
        "PRICE_BELOW" => ("价格", "低于", false),
        "CHANGE_ABOVE" => ("日涨跌幅", "超过", true),
        "CHANGE_BELOW" => ("日涨跌幅", "低于", true),
        "PNL_ABOVE" => ("持仓盈亏比例", "超过", true),
        "PNL_BELOW" => ("持仓盈亏比例", "低于", true),
        _ => return Err("unsupported price alert type".into()),
    };
    let format_value = |value| {
        if is_percent {
            format!("{value:.2}%")
        } else {
            amount(value, currency)
        }
    };
    let threshold = format_value(alert.threshold);
    let actual = format_value(current_value);
    let mut details = vec![
        detail("股票", format!("{}（{}）", alert.name, alert.symbol)),
        detail("市场", market_name(&alert.market)),
        detail("提醒条件", format!("{metric}{condition} {threshold}")),
        detail("触发时数值", &actual),
        detail("设定阈值", threshold),
        detail("触发时股价", amount(quote.0, currency)),
        detail("触发时日涨跌幅", format!("{:.2}%", quote.1)),
        detail("规则创建时间", &alert.created_at),
    ];
    if let Some((name, shares, cost, _, category)) = &holding {
        details.extend([
            detail("账户名称", name),
            detail("投资类别", category.as_deref().unwrap_or("未分类")),
            detail("持仓数量", format!("{shares:.4}")),
            detail("持仓成本价", amount(*cost, currency)),
            detail("触发时持仓市值", amount(*shares * quote.0, currency)),
        ]);
    }
    Ok(AlertHistoryMessage {
        id: Uuid::new_v4().to_string(),
        kind: "PRICE".into(),
        title: format!("{} · {metric}{condition}提醒", alert.name),
        message: format!(
            "{}（{}）{metric} {actual}，已{condition}设定阈值 {}。",
            alert.name,
            alert.symbol,
            format_value(alert.threshold)
        ),
        scope_name,
        account_name,
        triggered_at: triggered_at.into(),
        details,
    })
}

pub(super) fn portfolio_alert_history(
    connection: &Connection,
    config: &PortfolioAlertConfig,
    snapshot: &PortfolioAlertSnapshot,
    breach: &PortfolioAlertBreach,
    positions: &[PortfolioAlertPositionInput],
) -> Result<AlertHistoryMessage, String> {
    let mut statement = connection
        .prepare("SELECT id, name FROM accounts")
        .map_err(|error| error.to_string())?;
    let accounts = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|error| error.to_string())?;
    let account_name = config.scope.account_id.as_ref().map(|id| {
        accounts
            .get(id)
            .cloned()
            .unwrap_or_else(|| "已删除账户".into())
    });
    let scope_name = match config.scope.kind {
        PortfolioAlertScopeKind::Overall => "全部账户组合".into(),
        PortfolioAlertScopeKind::Market => format!(
            "{}组合",
            market_name(config.scope.market.as_deref().unwrap_or("未知市场"))
        ),
        PortfolioAlertScopeKind::Account => {
            format!("账户：{}", account_name.as_deref().unwrap_or("已删除账户"))
        }
    };
    let currency = &snapshot.base_currency;
    let mut details = vec![
        detail("监控范围", &scope_name),
        detail("计算币种", currency),
        detail(
            "触发时组合总市值",
            amount(snapshot.total_market_value, currency),
        ),
    ];
    let related: Vec<&PortfolioAlertPositionInput>;
    let (title, message) = match breach.breach_kind {
        PortfolioAlertBreachKind::CategoryDeviation => {
            let category = snapshot
                .categories
                .iter()
                .find(|category| {
                    format!(
                        "category:{}",
                        category.category_id.as_deref().unwrap_or("uncategorized")
                    ) == breach.breach_key
                })
                .ok_or_else(|| "category alert snapshot is missing".to_string())?;
            let direction = if category.current_percent > category.target_percent {
                "超配"
            } else {
                "低配"
            };
            details.extend([
                detail("投资类别", &category.category_name),
                detail("偏离方向", direction),
                detail("目标占比", format!("{:.2}%", category.target_percent)),
                detail("触发时占比", format!("{:.2}%", category.current_percent)),
                detail(
                    "占比差额",
                    format!(
                        "{:+.2} 个百分点",
                        category.current_percent - category.target_percent
                    ),
                ),
                detail(
                    "相对偏离",
                    category
                        .relative_deviation_percent
                        .map(|v| format!("{v:.2}%"))
                        .unwrap_or_else(|| "目标占比为 0，存在非零持仓".into()),
                ),
                detail(
                    "相对偏离阈值",
                    format!("{:.2}%", config.deviation_threshold),
                ),
                detail(
                    "触发时类别市值",
                    amount(category.current_market_value, currency),
                ),
                detail(
                    "目标类别市值",
                    amount(category.target_market_value, currency),
                ),
                detail(
                    "恢复目标占比所需调整",
                    format!(
                        "{} {}",
                        if category.rebalance_amount >= 0.0 {
                            "增加"
                        } else {
                            "减少"
                        },
                        amount(category.rebalance_amount.abs(), currency)
                    ),
                ),
            ]);
            related = positions
                .iter()
                .filter(|position| position.category_id == category.category_id)
                .collect();
            (
                format!("{} · 类别{direction}预警", category.category_name),
                format!(
                    "{scope_name}中，{}当前占比 {:.2}%，目标 {:.2}%，发生{direction}。",
                    category.category_name, category.current_percent, category.target_percent
                ),
            )
        }
        PortfolioAlertBreachKind::Concentration => {
            let security = snapshot
                .concentrations
                .iter()
                .find(|security| {
                    format!(
                        "security:{}:{}",
                        security.market, security.normalized_symbol
                    ) == breach.breach_key
                })
                .ok_or_else(|| "concentration alert snapshot is missing".to_string())?;
            details.extend([
                detail("股票", format!("{}（{}）", security.name, security.symbol)),
                detail("市场", market_name(&security.market)),
                detail(
                    "触发时持仓占比",
                    format!("{:.2}%", security.position_percent),
                ),
                detail("集中度上限", format!("{:.2}%", security.threshold_percent)),
                detail(
                    "超出上限",
                    format!(
                        "{:.2} 个百分点",
                        security.position_percent - security.threshold_percent
                    ),
                ),
                detail("触发时股票总市值", amount(security.market_value, currency)),
                detail(
                    "上限对应市值",
                    amount(
                        snapshot.total_market_value * security.threshold_percent / 100.0,
                        currency,
                    ),
                ),
            ]);
            related = positions
                .iter()
                .filter(|position| {
                    position.market == security.market
                        && position.symbol == security.normalized_symbol
                })
                .collect();
            (
                format!("{} · 持仓集中度预警", security.name),
                format!(
                    "{scope_name}中，{}（{}）持仓占比 {:.2}%，超过上限 {:.2}%。",
                    security.name,
                    security.symbol,
                    security.position_percent,
                    security.threshold_percent
                ),
            )
        }
    };
    let account_names: BTreeSet<&str> = related
        .iter()
        .map(|position| {
            accounts
                .get(&position.account_id)
                .map(String::as_str)
                .unwrap_or("已删除账户")
        })
        .collect();
    if !account_names.is_empty() {
        details.push(detail(
            "涉及账户",
            account_names.into_iter().collect::<Vec<_>>().join("、"),
        ));
    }
    // Store each contributing holding with the same values used by this evaluation.
    let mut related = related;
    related.sort_by(|a, b| {
        (&a.market, &a.symbol, accounts.get(&a.account_id)).cmp(&(
            &b.market,
            &b.symbol,
            accounts.get(&b.account_id),
        ))
    });
    for position in related {
        let name = accounts
            .get(&position.account_id)
            .map(String::as_str)
            .unwrap_or("已删除账户");
        details.push(detail(
            &format!("持仓 · {}（{}）", position.name, position.symbol),
            format!(
                "{name} · {} · {} · 市值 {} · 占组合 {:.2}%",
                market_name(&position.market),
                position.category_name,
                amount(position.market_value, currency),
                position.market_value / snapshot.total_market_value * 100.0
            ),
        ));
    }
    Ok(AlertHistoryMessage {
        id: Uuid::new_v4().to_string(),
        kind: "PORTFOLIO".into(),
        title,
        message,
        scope_name,
        account_name,
        triggered_at: snapshot.evaluated_at.clone(),
        details,
    })
}
