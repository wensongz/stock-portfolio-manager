use crate::{
    db::Database,
    models::portfolio_alert::{
        AllocationDirection, MissingPortfolioAlertData, PortfolioAlertBreach,
        PortfolioAlertBreachDirection, PortfolioAlertBreachKind, PortfolioAlertConfig,
        PortfolioAlertDataStatus, PortfolioAlertEvaluation, PortfolioAlertNotification,
        PortfolioAlertScope, PortfolioAlertScopeKind, PortfolioAlertSnapshot, PortfolioAlertTarget,
        PortfolioAlertView, SavePortfolioAlertConfigInput,
    },
    models::ExchangeRates,
    services::{
        exchange_rate_service::convert_currency,
        portfolio_alert_calculator::{
            calculate_portfolio_alert_snapshot, PortfolioAlertCalculation,
            PortfolioAlertCategoryInput, PortfolioAlertPositionInput,
        },
        portfolio_read_service::{PortfolioReadModel, QuoteReadMode},
        quote_service::{is_cash_symbol, QuoteCache},
        stock_operation_builder::normalize_stock_symbol,
    },
};
use chrono::{SecondsFormat, TimeDelta, Utc};
use rusqlite::{Connection, OptionalExtension};
use std::collections::{BTreeMap, HashMap, HashSet};
use tracing::warn;
use uuid::Uuid;

const TOTAL_TOLERANCE: f64 = 0.01;

fn next_mutation_timestamp(
    connection: &Connection,
    config_id: Option<&str>,
) -> Result<String, String> {
    let now = Utc::now();
    let previous = config_id
        .map(|id| {
            connection
                .query_row(
                    "SELECT updated_at FROM portfolio_alert_configs WHERE id = ?1",
                    [id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())
        })
        .transpose()?
        .flatten()
        .and_then(|value| chrono::DateTime::parse_from_rfc3339(&value).ok())
        .map(|value| value.with_timezone(&Utc));
    let timestamp = match previous {
        Some(previous) if previous >= now => previous
            .checked_add_signed(TimeDelta::nanoseconds(1))
            .ok_or_else(|| "portfolio alert mutation timestamp overflow".to_string())?,
        _ => now,
    };
    Ok(timestamp.to_rfc3339_opts(SecondsFormat::Nanos, true))
}

#[derive(Clone)]
struct EvaluationGuard {
    config: PortfolioAlertConfig,
    revision: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PortfolioAlertPreviewPosition {
    pub account_id: String,
    pub market: String,
    pub symbol: String,
    pub name: String,
    pub category_id: Option<String>,
    pub category_name: String,
    pub category_color: String,
    pub shares: f64,
    pub current_price: f64,
    pub quote_updated_at: Option<String>,
    pub native_market_value: f64,
    pub native_currency: String,
    pub base_market_value: f64,
    pub base_currency: String,
    pub conversion_rate: f64,
    pub exchange_rate_updated_at: Option<String>,
    pub is_cash: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TrustedPortfolioAlertPreview {
    pub config: PortfolioAlertConfig,
    pub account_market: Option<String>,
    pub evaluation: PortfolioAlertEvaluation,
    pub positions: Vec<PortfolioAlertPreviewPosition>,
}

struct PreviewCalculation {
    guard: EvaluationGuard,
    account_market: Option<String>,
    base_currency: String,
    positions: Vec<PortfolioAlertPositionInput>,
    proposed_breaches: Vec<PortfolioAlertBreach>,
    initial_breaches: Vec<PortfolioAlertBreach>,
}

pub fn scope_key(scope: &PortfolioAlertScope) -> Result<String, String> {
    match scope.kind {
        PortfolioAlertScopeKind::Overall
            if scope.market.is_none() && scope.account_id.is_none() =>
        {
            Ok("overall".to_string())
        }
        PortfolioAlertScopeKind::Market if scope.account_id.is_none() => {
            let market = validated_market(scope.market.as_deref())?;
            Ok(format!("market:{market}"))
        }
        PortfolioAlertScopeKind::Account if scope.market.is_none() => {
            let account_id = scope
                .account_id
                .as_deref()
                .filter(|id| !id.is_empty())
                .ok_or_else(|| "account scope requires an account id".to_string())?;
            Ok(format!("account:{account_id}"))
        }
        _ => Err("invalid portfolio alert scope".to_string()),
    }
}

pub fn get_portfolio_alert_config_by_scope(
    db: &Database,
    scope: &PortfolioAlertScope,
) -> Result<Option<PortfolioAlertConfig>, String> {
    let key = scope_key(scope)?;
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    load_config_by_column(&conn, "scope_key", &key)
}

pub fn get_portfolio_alert_config_by_id(
    db: &Database,
    config_id: &str,
) -> Result<PortfolioAlertConfig, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    load_config_by_column(&conn, "id", config_id)?
        .ok_or_else(|| format!("portfolio alert configuration {config_id} not found"))
}

pub fn save_portfolio_alert_config(
    db: &Database,
    input: SavePortfolioAlertConfigInput,
) -> Result<PortfolioAlertConfig, String> {
    let key = scope_key(&input.scope)?;
    validate_input_basics(&input)?;

    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    validate_scope_currency(&transaction, &input)?;
    validate_target_categories(&transaction, &input.targets)?;

    let existing = match input.id.as_deref() {
        Some(id) => transaction
            .query_row(
                "SELECT id, scope_key FROM portfolio_alert_configs WHERE id = ?1",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?,
        None => transaction
            .query_row(
                "SELECT id, scope_key FROM portfolio_alert_configs WHERE scope_key = ?1",
                [&key],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .optional()
            .map_err(|error| error.to_string())?,
    };

    if let Some((_, existing_key)) = &existing {
        if existing_key != &key {
            return Err("a portfolio alert configuration cannot change scope".to_string());
        }
    }

    let id = match (&existing, input.id.as_deref()) {
        (Some((id, _)), _) => id.clone(),
        (None, Some(id)) => id.to_string(),
        (None, None) => Uuid::new_v4().to_string(),
    };
    let now = Utc::now().to_rfc3339();
    let revision = next_mutation_timestamp(
        &transaction,
        existing
            .as_ref()
            .map(|(existing_id, _)| existing_id.as_str()),
    )?;
    let (scope_kind, market, account_id) = scope_columns(&input.scope)?;

    if existing.is_some() {
        transaction
            .execute(
                "UPDATE portfolio_alert_configs
                 SET base_currency = ?1, deviation_threshold = ?2, concentration_threshold = ?3,
                     is_active = ?4, updated_at = ?5
                 WHERE id = ?6",
                rusqlite::params![
                    input.base_currency,
                    input.deviation_threshold,
                    input.concentration_threshold,
                    input.is_active as i32,
                    revision,
                    id,
                ],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM portfolio_alert_breaches WHERE config_id = ?1",
                [&id],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM portfolio_alert_targets WHERE config_id = ?1",
                [&id],
            )
            .map_err(|error| error.to_string())?;
    } else {
        transaction
            .execute(
                "INSERT INTO portfolio_alert_configs
                 (id, scope_key, scope_kind, market, account_id, base_currency,
                  deviation_threshold, concentration_threshold, is_active, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                rusqlite::params![
                    id,
                    key,
                    scope_kind,
                    market,
                    account_id,
                    input.base_currency,
                    input.deviation_threshold,
                    input.concentration_threshold,
                    input.is_active as i32,
                    now,
                    revision,
                ],
            )
            .map_err(|error| error.to_string())?;
    }

    for target in &input.targets {
        transaction
            .execute(
                "INSERT INTO portfolio_alert_targets (config_id, category_id, target_percent)
                 VALUES (?1, ?2, ?3)",
                rusqlite::params![id, target.category_id, target.target_percent],
            )
            .map_err(|error| error.to_string())?;
    }

    let config = load_config_by_column(&transaction, "id", &id)?
        .ok_or_else(|| format!("portfolio alert configuration {id} was not saved"))?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(config)
}

pub fn set_portfolio_alert_active(
    db: &Database,
    config_id: &str,
    is_active: bool,
) -> Result<PortfolioAlertConfig, String> {
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    let rows = transaction
        .execute(
            "UPDATE portfolio_alert_configs SET is_active = ?1, updated_at = ?2 WHERE id = ?3",
            rusqlite::params![
                is_active as i32,
                next_mutation_timestamp(&transaction, Some(config_id))?,
                config_id
            ],
        )
        .map_err(|error| error.to_string())?;
    if rows == 0 {
        return Err(format!(
            "portfolio alert configuration {config_id} not found"
        ));
    }
    if !is_active {
        transaction
            .execute(
                "DELETE FROM portfolio_alert_breaches WHERE config_id = ?1",
                [config_id],
            )
            .map_err(|error| error.to_string())?;
    }
    let config = load_config_by_column(&transaction, "id", config_id)?
        .ok_or_else(|| format!("portfolio alert configuration {config_id} not found"))?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok(config)
}

fn load_categories(connection: &Connection) -> Result<Vec<PortfolioAlertCategoryInput>, String> {
    let mut statement = connection
        .prepare(
            "SELECT id, name, color, icon, sort_order
             FROM categories ORDER BY sort_order, id",
        )
        .map_err(|error| error.to_string())?;
    let categories = statement
        .query_map([], |row| {
            Ok(PortfolioAlertCategoryInput {
                id: row.get(0)?,
                name: row.get(1)?,
                color: row.get(2)?,
                icon: row.get(3)?,
                sort_order: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(categories)
}

fn account_market(connection: &Connection, account_id: &str) -> Result<String, String> {
    connection
        .query_row(
            "SELECT market FROM accounts WHERE id = ?1",
            [account_id],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| error.to_string())?
        .ok_or_else(|| format!("account {account_id} not found"))
}

fn evaluation_currency_and_account_market(
    connection: &Connection,
    config: &PortfolioAlertConfig,
) -> Result<(String, Option<String>), String> {
    match config.scope.kind {
        PortfolioAlertScopeKind::Overall => Ok((config.base_currency.clone(), None)),
        PortfolioAlertScopeKind::Market => {
            let market = validated_market(config.scope.market.as_deref())?;
            Ok((native_currency(market).to_string(), None))
        }
        PortfolioAlertScopeKind::Account => {
            let account_id = config
                .scope
                .account_id
                .as_deref()
                .ok_or_else(|| "account scope requires an account id".to_string())?;
            let market = account_market(connection, account_id)?;
            Ok((
                native_currency(validated_market(Some(&market))?).to_string(),
                Some(market),
            ))
        }
    }
}

fn scope_matches_holding(
    config: &PortfolioAlertConfig,
    account_market: Option<&str>,
    holding: &crate::models::HoldingDetail,
) -> bool {
    match config.scope.kind {
        PortfolioAlertScopeKind::Overall => true,
        PortfolioAlertScopeKind::Market => config
            .scope
            .market
            .as_deref()
            .is_some_and(|market| market.eq_ignore_ascii_case(&holding.market)),
        PortfolioAlertScopeKind::Account => {
            config
                .scope
                .account_id
                .as_deref()
                .is_some_and(|account_id| account_id == holding.account_id)
                && account_market.is_none_or(|market| market.eq_ignore_ascii_case(&holding.market))
        }
    }
}

fn required_fx_is_valid(from: &str, to: &str, rates: &ExchangeRates) -> bool {
    let valid = |value: f64| value.is_finite() && value > 0.0;
    if from == to {
        return true;
    }
    match (from, to) {
        ("USD", "CNY") | ("CNY", "USD") => valid(rates.usd_cny),
        ("USD", "HKD") | ("HKD", "USD") => valid(rates.usd_hkd),
        ("CNY", "HKD") | ("HKD", "CNY") => valid(rates.usd_cny) && valid(rates.usd_hkd),
        _ => false,
    }
}

fn load_active_breaches(
    connection: &Connection,
    config_id: &str,
) -> Result<Vec<PortfolioAlertBreach>, String> {
    let mut statement = connection
        .prepare(
            "SELECT breach_key, breach_kind, direction, first_triggered_at, last_seen_at
             FROM portfolio_alert_breaches WHERE config_id = ?1 ORDER BY breach_key",
        )
        .map_err(|error| error.to_string())?;
    let breaches = statement
        .query_map([config_id], |row| {
            let kind = match row.get::<_, String>(1)?.as_str() {
                "CATEGORY_DEVIATION" => PortfolioAlertBreachKind::CategoryDeviation,
                "CONCENTRATION" => PortfolioAlertBreachKind::Concentration,
                value => {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        1,
                        rusqlite::types::Type::Text,
                        format!("invalid breach kind {value}").into(),
                    ))
                }
            };
            let direction = match row.get::<_, String>(2)?.as_str() {
                "OVERWEIGHT" => PortfolioAlertBreachDirection::Overweight,
                "UNDERWEIGHT" => PortfolioAlertBreachDirection::Underweight,
                "ABOVE_LIMIT" => PortfolioAlertBreachDirection::AboveLimit,
                value => {
                    return Err(rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Text,
                        format!("invalid breach direction {value}").into(),
                    ))
                }
            };
            Ok(PortfolioAlertBreach {
                config_id: config_id.to_string(),
                breach_key: row.get(0)?,
                breach_kind: kind,
                direction,
                first_triggered_at: row.get(3)?,
                last_seen_at: row.get(4)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    Ok(breaches)
}

fn load_evaluation_guard(
    connection: &Connection,
    config: &PortfolioAlertConfig,
) -> Result<EvaluationGuard, String> {
    let revision = connection
        .query_row(
            "SELECT updated_at FROM portfolio_alert_configs WHERE id = ?1",
            [&config.id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    Ok(EvaluationGuard {
        config: config.clone(),
        revision,
    })
}

fn verify_evaluation_guard(connection: &Connection, guard: &EvaluationGuard) -> Result<(), String> {
    let current = load_config_by_column(connection, "id", &guard.config.id)?
        .ok_or_else(|| "portfolio alert configuration changed during evaluation".to_string())?;
    if !current.is_active {
        return Err("portfolio alert configuration became inactive during evaluation".to_string());
    }
    let current_revision: String = connection
        .query_row(
            "SELECT updated_at FROM portfolio_alert_configs WHERE id = ?1",
            [&guard.config.id],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    if current_revision != guard.revision || current != guard.config {
        return Err("portfolio alert configuration changed during evaluation".to_string());
    }
    Ok(())
}

fn validate_preview_state(
    db: &Database,
    calculation: &PreviewCalculation,
) -> Result<Vec<PortfolioAlertBreach>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    verify_evaluation_guard(&conn, &calculation.guard)?;
    if let Some(expected_market) = calculation.account_market.as_deref() {
        let account_id = calculation
            .guard
            .config
            .scope
            .account_id
            .as_deref()
            .ok_or_else(|| "account preview lost its account id".to_string())?;
        let current_market = account_market(&conn, account_id)?;
        if current_market != expected_market {
            return Err("portfolio alert account market changed during preview".to_string());
        }
    }
    let current_breaches = load_active_breaches(&conn, &calculation.guard.config.id)?;
    if current_breaches != calculation.initial_breaches {
        return Err("portfolio alert breaches changed during preview".to_string());
    }
    let matched = intersect_preview_breaches(&current_breaches, &calculation.proposed_breaches);
    if matched.is_empty() {
        return Err("portfolio alert has no current matching active breach".to_string());
    }
    Ok(matched)
}

fn unchanged_evaluation(
    config: &PortfolioAlertConfig,
    status: PortfolioAlertDataStatus,
    missing_data: Vec<MissingPortfolioAlertData>,
    active_breaches: Vec<PortfolioAlertBreach>,
) -> PortfolioAlertEvaluation {
    PortfolioAlertEvaluation {
        status,
        snapshot: config.last_snapshot.clone(),
        stale: true,
        missing_data,
        active_breaches,
        newly_triggered: vec![],
    }
}

fn proposed_breaches(
    config_id: &str,
    snapshot: &PortfolioAlertSnapshot,
    evaluated_at: &str,
) -> BTreeMap<String, PortfolioAlertBreach> {
    let mut proposed = BTreeMap::new();
    for category in &snapshot.categories {
        let Some(direction) = &category.direction else {
            continue;
        };
        let key = format!(
            "category:{}",
            category.category_id.as_deref().unwrap_or("uncategorized")
        );
        proposed.insert(
            key.clone(),
            PortfolioAlertBreach {
                config_id: config_id.to_string(),
                breach_key: key,
                breach_kind: PortfolioAlertBreachKind::CategoryDeviation,
                direction: match direction {
                    AllocationDirection::Overweight => PortfolioAlertBreachDirection::Overweight,
                    AllocationDirection::Underweight => PortfolioAlertBreachDirection::Underweight,
                },
                first_triggered_at: evaluated_at.to_string(),
                last_seen_at: evaluated_at.to_string(),
            },
        );
    }
    for concentration in &snapshot.concentrations {
        let key = format!(
            "security:{}:{}",
            concentration.market, concentration.normalized_symbol
        );
        proposed.insert(
            key.clone(),
            PortfolioAlertBreach {
                config_id: config_id.to_string(),
                breach_key: key,
                breach_kind: PortfolioAlertBreachKind::Concentration,
                direction: PortfolioAlertBreachDirection::AboveLimit,
                first_triggered_at: evaluated_at.to_string(),
                last_seen_at: evaluated_at.to_string(),
            },
        );
    }
    proposed
}

fn intersect_preview_breaches(
    persisted: &[PortfolioAlertBreach],
    proposed: &[PortfolioAlertBreach],
) -> Vec<PortfolioAlertBreach> {
    proposed
        .iter()
        .filter_map(|current| {
            persisted
                .iter()
                .find(|saved| {
                    saved.breach_key == current.breach_key
                        && saved.breach_kind == current.breach_kind
                        && saved.direction == current.direction
                })
                .map(|saved| PortfolioAlertBreach {
                    first_triggered_at: saved.first_triggered_at.clone(),
                    last_seen_at: saved.last_seen_at.clone(),
                    ..current.clone()
                })
        })
        .collect()
}

fn preview_positions(
    read_model: &PortfolioReadModel,
    quote_snapshot: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    calculation: &PreviewCalculation,
) -> Result<Vec<PortfolioAlertPreviewPosition>, String> {
    let mut calculated = calculation.positions.clone();
    read_model
        .holdings()
        .iter()
        .filter(|holding| {
            scope_matches_holding(
                &calculation.guard.config,
                calculation.account_market.as_deref(),
                holding,
            )
        })
        .map(|holding| {
            let symbol = normalize_stock_symbol(&holding.symbol)
                .unwrap_or_else(|| holding.symbol.trim().to_ascii_uppercase());
            let index = calculated
                .iter()
                .position(|position| {
                    position.account_id == holding.account_id
                        && position.market == holding.market.trim().to_ascii_uppercase()
                        && position.symbol == symbol
                })
                .ok_or_else(|| format!("preview position {symbol} was not calculated"))?;
            let base_position = calculated.remove(index);
            let is_cash = base_position.is_cash;
            let native_currency = holding.currency.trim().to_ascii_uppercase();
            let native_market_value = if is_cash {
                holding.shares
            } else {
                holding.market_value
            };
            let conversion_rate = if native_currency == calculation.base_currency {
                1.0
            } else if native_market_value != 0.0 {
                base_position.market_value / native_market_value
            } else {
                convert_currency(
                    1.0,
                    &native_currency,
                    &calculation.base_currency,
                    exchange_rates
                        .ok_or_else(|| "preview exchange rates are unavailable".to_string())?,
                )
            };
            Ok(PortfolioAlertPreviewPosition {
                account_id: holding.account_id.clone(),
                market: holding.market.trim().to_ascii_uppercase(),
                symbol,
                name: holding.name.clone(),
                category_id: read_model
                    .category_id_for_holding(&holding.id)
                    .map(str::to_string),
                category_name: holding.category_name.clone(),
                category_color: holding.category_color.clone(),
                shares: holding.shares,
                current_price: if is_cash { 1.0 } else { holding.current_price },
                quote_updated_at: (!is_cash)
                    .then(|| quote_snapshot.get_stale(&holding.market, &holding.symbol))
                    .flatten()
                    .map(|quote| quote.updated_at),
                native_market_value,
                native_currency: native_currency.clone(),
                base_market_value: base_position.market_value,
                base_currency: calculation.base_currency.clone(),
                conversion_rate,
                exchange_rate_updated_at: (native_currency != calculation.base_currency)
                    .then(|| exchange_rates.map(|rates| rates.updated_at.clone()))
                    .flatten(),
                is_cash,
            })
        })
        .collect()
}

fn breach_kind_sql(kind: &PortfolioAlertBreachKind) -> &'static str {
    match kind {
        PortfolioAlertBreachKind::CategoryDeviation => "CATEGORY_DEVIATION",
        PortfolioAlertBreachKind::Concentration => "CONCENTRATION",
    }
}

fn breach_direction_sql(direction: &PortfolioAlertBreachDirection) -> &'static str {
    match direction {
        PortfolioAlertBreachDirection::Overweight => "OVERWEIGHT",
        PortfolioAlertBreachDirection::Underweight => "UNDERWEIGHT",
        PortfolioAlertBreachDirection::AboveLimit => "ABOVE_LIMIT",
    }
}

fn persist_ready_transition(
    db: &Database,
    config_id: &str,
    snapshot: &PortfolioAlertSnapshot,
    proposed: &BTreeMap<String, PortfolioAlertBreach>,
    evaluated_at: &str,
    guard: &EvaluationGuard,
) -> Result<(Vec<PortfolioAlertBreach>, Vec<PortfolioAlertBreach>), String> {
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    verify_evaluation_guard(&transaction, guard)?;
    let existing = load_active_breaches(&transaction, config_id)?
        .into_iter()
        .map(|breach| (breach.breach_key.clone(), breach))
        .collect::<HashMap<_, _>>();

    for (key, breach) in proposed
        .iter()
        .filter(|(key, _)| existing.contains_key(*key))
    {
        transaction
            .execute(
                "UPDATE portfolio_alert_breaches
                 SET breach_kind = ?1, direction = ?2, last_seen_at = ?3
                 WHERE config_id = ?4 AND breach_key = ?5",
                rusqlite::params![
                    breach_kind_sql(&breach.breach_kind),
                    breach_direction_sql(&breach.direction),
                    evaluated_at,
                    config_id,
                    key,
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    for key in existing.keys().filter(|key| !proposed.contains_key(*key)) {
        transaction
            .execute(
                "DELETE FROM portfolio_alert_breaches WHERE config_id = ?1 AND breach_key = ?2",
                rusqlite::params![config_id, key],
            )
            .map_err(|error| error.to_string())?;
    }

    let mut newly_triggered = Vec::new();
    for (key, breach) in proposed
        .iter()
        .filter(|(key, _)| !existing.contains_key(*key))
    {
        transaction
            .execute(
                "INSERT INTO portfolio_alert_breaches
                 (config_id, breach_key, breach_kind, direction, first_triggered_at, last_seen_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                rusqlite::params![
                    config_id,
                    key,
                    breach_kind_sql(&breach.breach_kind),
                    breach_direction_sql(&breach.direction),
                    evaluated_at,
                ],
            )
            .map_err(|error| error.to_string())?;
        newly_triggered.push(breach.clone());
    }

    let snapshot_json = serde_json::to_string(snapshot).map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE portfolio_alert_configs
             SET last_snapshot_json = ?1, last_evaluated_at = ?2
             WHERE id = ?3",
            rusqlite::params![snapshot_json, evaluated_at, config_id],
        )
        .map_err(|error| error.to_string())?;
    let active = load_active_breaches(&transaction, config_id)?;
    transaction.commit().map_err(|error| error.to_string())?;
    Ok((active, newly_triggered))
}

fn persist_empty_transition(
    db: &Database,
    config_id: &str,
    evaluated_at: &str,
    guard: &EvaluationGuard,
) -> Result<(), String> {
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    verify_evaluation_guard(&transaction, guard)?;
    transaction
        .execute(
            "DELETE FROM portfolio_alert_breaches WHERE config_id = ?1",
            [config_id],
        )
        .map_err(|error| error.to_string())?;
    transaction
        .execute(
            "UPDATE portfolio_alert_configs
             SET last_snapshot_json = NULL, last_evaluated_at = ?1 WHERE id = ?2",
            rusqlite::params![evaluated_at, config_id],
        )
        .map_err(|error| error.to_string())?;
    transaction.commit().map_err(|error| error.to_string())
}

pub async fn evaluate_portfolio_alert(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    config_id: &str,
    evaluated_at: &str,
) -> Result<PortfolioAlertEvaluation, String> {
    let read_model =
        PortfolioReadModel::load(db, quote_cache, None, QuoteReadMode::CacheOnly).await?;
    evaluate_portfolio_alert_inner(
        db,
        exchange_rates,
        config_id,
        evaluated_at,
        &read_model,
        true,
        || {},
    )
    .await
    .map(|(evaluation, _)| evaluation)
}

/// Recalculate an alert from the current in-memory quote snapshot without
/// changing the saved snapshot, evaluation timestamp, breach rows, or config.
pub async fn preview_portfolio_alert(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    config_id: &str,
    evaluated_at: &str,
) -> Result<TrustedPortfolioAlertPreview, String> {
    preview_portfolio_alert_inner(
        db,
        quote_cache,
        exchange_rates,
        config_id,
        evaluated_at,
        || {},
    )
    .await
}

async fn preview_portfolio_alert_inner<F>(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    config_id: &str,
    evaluated_at: &str,
    before_validate: F,
) -> Result<TrustedPortfolioAlertPreview, String>
where
    F: FnOnce(),
{
    let quote_snapshot = quote_cache.snapshot();
    let read_model =
        PortfolioReadModel::load(db, &quote_snapshot, None, QuoteReadMode::CacheOnly).await?;
    let (mut evaluation, calculation) = evaluate_portfolio_alert_inner(
        db,
        exchange_rates,
        config_id,
        evaluated_at,
        &read_model,
        false,
        || {},
    )
    .await?;
    if evaluation.status != PortfolioAlertDataStatus::Ready || evaluation.stale {
        return Err(format!(
            "portfolio alert preview is unavailable: status {:?}, stale={}",
            evaluation.status, evaluation.stale
        ));
    }
    let calculation = calculation
        .ok_or_else(|| "READY portfolio alert preview is missing its calculation".to_string())?;
    before_validate();
    evaluation.active_breaches = validate_preview_state(db, &calculation)?;
    let positions = preview_positions(&read_model, &quote_snapshot, exchange_rates, &calculation)?;
    Ok(TrustedPortfolioAlertPreview {
        config: calculation.guard.config,
        account_market: calculation.account_market,
        evaluation,
        positions,
    })
}

#[cfg(test)]
async fn preview_portfolio_alert_with_before_validate_hook<F>(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    config_id: &str,
    evaluated_at: &str,
    before_validate: F,
) -> Result<TrustedPortfolioAlertPreview, String>
where
    F: FnOnce(),
{
    preview_portfolio_alert_inner(
        db,
        quote_cache,
        exchange_rates,
        config_id,
        evaluated_at,
        before_validate,
    )
    .await
}

/// Evaluate each configuration that was active at the start of this batch.
/// A malformed or incomplete scope must not prevent the remaining scopes from
/// updating their persisted breach transitions.
pub async fn evaluate_all_active_portfolio_alerts(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    evaluated_at: &str,
) -> Result<Vec<PortfolioAlertNotification>, String> {
    evaluate_all_active_portfolio_alerts_inner(
        db,
        quote_cache,
        exchange_rates,
        evaluated_at,
        |_| {},
    )
    .await
}

async fn evaluate_all_active_portfolio_alerts_inner<F>(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    evaluated_at: &str,
    mut before_config: F,
) -> Result<Vec<PortfolioAlertNotification>, String>
where
    F: FnMut(usize),
{
    let active_ids = {
        let conn = db.conn.lock().map_err(|error| error.to_string())?;
        let mut statement = conn
            .prepare("SELECT id FROM portfolio_alert_configs WHERE is_active = 1 ORDER BY id")
            .map_err(|error| error.to_string())?;
        let ids = statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        ids
    };
    let quote_snapshot = quote_cache.snapshot();
    let read_model =
        PortfolioReadModel::load(db, &quote_snapshot, None, QuoteReadMode::CacheOnly).await?;

    let mut notifications = Vec::new();
    for (index, config_id) in active_ids.into_iter().enumerate() {
        before_config(index);
        let config = match get_portfolio_alert_config_by_id(db, &config_id) {
            Ok(config) if config.is_active => config,
            Ok(_) => continue,
            Err(error) => {
                warn!(
                    config_id = %config_id,
                    "Skipping portfolio alert evaluation because configuration could not be loaded: {error}"
                );
                continue;
            }
        };
        match evaluate_portfolio_alert_inner(
            db,
            exchange_rates,
            &config_id,
            evaluated_at,
            &read_model,
            true,
            || {},
        )
        .await
        {
            Ok((evaluation, _)) => {
                notifications.extend(evaluation.newly_triggered.into_iter().map(|breach| {
                    PortfolioAlertNotification {
                        config_id: config.id.clone(),
                        scope: config.scope.clone(),
                        message: portfolio_alert_notification_message(&breach),
                        breach,
                        triggered_at: evaluated_at.to_string(),
                    }
                }));
            }
            Err(error) => warn!(
                config_id = %config_id,
                "Portfolio alert evaluation failed after quote refresh: {error}"
            ),
        }
    }
    Ok(notifications)
}

#[cfg(test)]
async fn evaluate_all_active_with_before_config_hook<F>(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    evaluated_at: &str,
    before_config: F,
) -> Result<Vec<PortfolioAlertNotification>, String>
where
    F: FnMut(usize),
{
    evaluate_all_active_portfolio_alerts_inner(
        db,
        quote_cache,
        exchange_rates,
        evaluated_at,
        before_config,
    )
    .await
}

fn portfolio_alert_notification_message(breach: &PortfolioAlertBreach) -> String {
    match breach.breach_kind {
        PortfolioAlertBreachKind::CategoryDeviation => {
            format!("资产配置偏离预警：{}", breach.breach_key)
        }
        PortfolioAlertBreachKind::Concentration => {
            format!("持仓集中度预警：{}", breach.breach_key)
        }
    }
}

async fn evaluate_portfolio_alert_inner<F>(
    db: &Database,
    exchange_rates: Option<&ExchangeRates>,
    config_id: &str,
    evaluated_at: &str,
    read_model: &PortfolioReadModel,
    persist: bool,
    before_persist: F,
) -> Result<(PortfolioAlertEvaluation, Option<PreviewCalculation>), String>
where
    F: Fn(),
{
    let (config, guard, categories, base_currency, account_market) = {
        let conn = db.conn.lock().map_err(|error| error.to_string())?;
        let config = load_config_by_column(&conn, "id", config_id)?
            .ok_or_else(|| format!("portfolio alert configuration {config_id} not found"))?;
        if !config.is_active {
            return Err("portfolio alert configuration is inactive".to_string());
        }
        let guard = load_evaluation_guard(&conn, &config)?;
        let categories = load_categories(&conn)?;
        let (base_currency, account_market) =
            evaluation_currency_and_account_market(&conn, &config)?;
        (config, guard, categories, base_currency, account_market)
    };
    let scoped_holdings = read_model
        .holdings()
        .iter()
        .filter(|holding| scope_matches_holding(&config, account_market.as_deref(), holding))
        .collect::<Vec<_>>();

    if scoped_holdings.is_empty() {
        if persist {
            before_persist();
            persist_empty_transition(db, config_id, evaluated_at, &guard)?;
        }
        return Ok((
            PortfolioAlertEvaluation {
                status: PortfolioAlertDataStatus::Empty,
                snapshot: None,
                stale: false,
                missing_data: vec![],
                active_breaches: vec![],
                newly_triggered: vec![],
            },
            None,
        ));
    }

    let existing_breaches = {
        let conn = db.conn.lock().map_err(|error| error.to_string())?;
        load_active_breaches(&conn, config_id)?
    };
    let mut missing_data = Vec::new();
    let mut reported_missing_quotes = HashSet::new();
    for holding in &scoped_holdings {
        if is_cash_symbol(&holding.symbol) {
            continue;
        }
        let key = (
            holding.market.trim().to_ascii_uppercase(),
            holding.symbol.trim().to_ascii_uppercase(),
        );
        if read_model.missing_quote_keys().contains(&key) || !holding.current_price.is_finite() {
            if reported_missing_quotes.insert(key.clone()) {
                missing_data.push(MissingPortfolioAlertData {
                    market: Some(key.0),
                    symbol: Some(key.1),
                    currency: None,
                    reason: "cached quote is unavailable".to_string(),
                });
            }
        }
    }

    let required_currencies = scoped_holdings
        .iter()
        .map(|holding| holding.currency.trim().to_ascii_uppercase())
        .filter(|currency| currency != &base_currency)
        .collect::<HashSet<_>>();
    for currency in required_currencies {
        let valid = exchange_rates
            .is_some_and(|rates| required_fx_is_valid(&currency, &base_currency, rates));
        if !valid {
            missing_data.push(MissingPortfolioAlertData {
                market: None,
                symbol: None,
                currency: Some(currency.clone()),
                reason: format!("exchange rate from {currency} to {base_currency} is unavailable"),
            });
        }
    }
    if !missing_data.is_empty() {
        return Ok((
            unchanged_evaluation(
                &config,
                PortfolioAlertDataStatus::Incomplete,
                missing_data,
                existing_breaches,
            ),
            None,
        ));
    }

    let target_total: f64 = config
        .targets
        .iter()
        .map(|target| target.target_percent)
        .sum();
    if !target_total_is_within_tolerance(target_total) {
        return Ok((
            unchanged_evaluation(
                &config,
                PortfolioAlertDataStatus::InvalidConfig,
                vec![],
                existing_breaches,
            ),
            None,
        ));
    }

    let positions = scoped_holdings
        .into_iter()
        .map(|holding| {
            let is_cash = is_cash_symbol(&holding.symbol);
            let holding_currency = holding.currency.trim().to_ascii_uppercase();
            let native_value = if is_cash {
                holding.shares
            } else {
                holding.market_value
            };
            let market_value = if holding_currency == base_currency {
                native_value
            } else {
                convert_currency(
                    native_value,
                    &holding_currency,
                    &base_currency,
                    exchange_rates.expect("required exchange rates were validated"),
                )
            };
            PortfolioAlertPositionInput {
                account_id: holding.account_id.clone(),
                market: holding.market.trim().to_ascii_uppercase(),
                symbol: normalize_stock_symbol(&holding.symbol)
                    .unwrap_or_else(|| holding.symbol.trim().to_ascii_uppercase()),
                name: holding.name.clone(),
                category_id: read_model
                    .category_id_for_holding(&holding.id)
                    .map(str::to_string),
                category_name: holding.category_name.clone(),
                category_color: holding.category_color.clone(),
                market_value,
                is_cash,
            }
        })
        .collect::<Vec<_>>();

    let total_market_value = positions
        .iter()
        .map(|position| position.market_value)
        .sum::<f64>();
    if total_market_value <= 0.0 {
        if persist {
            before_persist();
            persist_empty_transition(db, config_id, evaluated_at, &guard)?;
        }
        return Ok((
            PortfolioAlertEvaluation {
                status: PortfolioAlertDataStatus::Empty,
                snapshot: None,
                stale: false,
                missing_data: vec![],
                active_breaches: vec![],
                newly_triggered: vec![],
            },
            None,
        ));
    }
    let negative_positions = positions
        .iter()
        .filter(|position| position.market_value < 0.0)
        .map(|position| MissingPortfolioAlertData {
            market: Some(position.market.clone()),
            symbol: Some(position.symbol.clone()),
            currency: None,
            reason: "cached quote has a negative value".to_string(),
        })
        .collect::<Vec<_>>();
    if !negative_positions.is_empty() {
        return Ok((
            unchanged_evaluation(
                &config,
                PortfolioAlertDataStatus::Incomplete,
                negative_positions,
                existing_breaches,
            ),
            None,
        ));
    }

    match calculate_portfolio_alert_snapshot(
        &config,
        &categories,
        &positions,
        &base_currency,
        evaluated_at,
    )? {
        PortfolioAlertCalculation::Empty => {
            if persist {
                before_persist();
                persist_empty_transition(db, config_id, evaluated_at, &guard)?;
            }
            Ok((
                PortfolioAlertEvaluation {
                    status: PortfolioAlertDataStatus::Empty,
                    snapshot: None,
                    stale: false,
                    missing_data: vec![],
                    active_breaches: vec![],
                    newly_triggered: vec![],
                },
                None,
            ))
        }
        PortfolioAlertCalculation::Ready(snapshot) => {
            let proposed = proposed_breaches(config_id, &snapshot, evaluated_at);
            let (active_breaches, newly_triggered, preview_calculation) = if persist {
                before_persist();
                let (active, newly_triggered) = persist_ready_transition(
                    db,
                    config_id,
                    &snapshot,
                    &proposed,
                    evaluated_at,
                    &guard,
                )?;
                (active, newly_triggered, None)
            } else {
                (
                    vec![],
                    vec![],
                    Some(PreviewCalculation {
                        guard,
                        account_market,
                        base_currency,
                        positions,
                        proposed_breaches: proposed.into_values().collect(),
                        initial_breaches: existing_breaches,
                    }),
                )
            };
            Ok((
                PortfolioAlertEvaluation {
                    status: PortfolioAlertDataStatus::Ready,
                    snapshot: Some(snapshot),
                    stale: false,
                    missing_data: vec![],
                    active_breaches,
                    newly_triggered,
                },
                preview_calculation,
            ))
        }
    }
}

#[cfg(test)]
async fn evaluate_portfolio_alert_with_before_persist_hook<F>(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    config_id: &str,
    evaluated_at: &str,
    before_persist: F,
) -> Result<PortfolioAlertEvaluation, String>
where
    F: Fn(),
{
    let read_model =
        PortfolioReadModel::load(db, quote_cache, None, QuoteReadMode::CacheOnly).await?;
    evaluate_portfolio_alert_inner(
        db,
        exchange_rates,
        config_id,
        evaluated_at,
        &read_model,
        true,
        before_persist,
    )
    .await
    .map(|(evaluation, _)| evaluation)
}

pub async fn save_and_evaluate_portfolio_alert_config(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    input: SavePortfolioAlertConfigInput,
    evaluated_at: &str,
) -> Result<PortfolioAlertView, String> {
    let config = save_portfolio_alert_config(db, input)?;
    let evaluation = if config.is_active {
        Some(
            evaluate_portfolio_alert(db, quote_cache, exchange_rates, &config.id, evaluated_at)
                .await?,
        )
    } else {
        None
    };
    Ok(PortfolioAlertView {
        config: Some(get_portfolio_alert_config_by_id(db, &config.id)?),
        evaluation,
    })
}

pub async fn set_portfolio_alert_active_and_evaluate(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    config_id: &str,
    is_active: bool,
    evaluated_at: &str,
) -> Result<PortfolioAlertView, String> {
    let config = set_portfolio_alert_active(db, config_id, is_active)?;
    let evaluation = if is_active {
        Some(
            evaluate_portfolio_alert(db, quote_cache, exchange_rates, config_id, evaluated_at)
                .await?,
        )
    } else {
        None
    };
    Ok(PortfolioAlertView {
        config: Some(get_portfolio_alert_config_by_id(db, &config.id)?),
        evaluation,
    })
}

fn validated_market(market: Option<&str>) -> Result<&str, String> {
    match market {
        Some("CN" | "US" | "HK") => Ok(market.unwrap()),
        _ => Err("invalid portfolio alert market scope".to_string()),
    }
}

fn native_currency(market: &str) -> &'static str {
    match market {
        "CN" => "CNY",
        "US" => "USD",
        "HK" => "HKD",
        _ => unreachable!("validated market"),
    }
}

fn scope_columns(
    scope: &PortfolioAlertScope,
) -> Result<(&'static str, Option<&str>, Option<&str>), String> {
    match scope.kind {
        PortfolioAlertScopeKind::Overall => Ok(("OVERALL", None, None)),
        PortfolioAlertScopeKind::Market => Ok((
            "MARKET",
            Some(validated_market(scope.market.as_deref())?),
            None,
        )),
        PortfolioAlertScopeKind::Account => Ok((
            "ACCOUNT",
            None,
            Some(
                scope
                    .account_id
                    .as_deref()
                    .ok_or_else(|| "account scope requires an account id".to_string())?,
            ),
        )),
    }
}

fn validate_input_basics(input: &SavePortfolioAlertConfigInput) -> Result<(), String> {
    if !matches!(input.base_currency.as_str(), "USD" | "CNY" | "HKD") {
        return Err("base currency must be USD, CNY, or HKD".to_string());
    }
    if !input.deviation_threshold.is_finite() || !(0.0..=100.0).contains(&input.deviation_threshold)
    {
        return Err("deviation threshold must be finite and between 0 and 100".to_string());
    }
    if !input.concentration_threshold.is_finite()
        || !(0.0 < input.concentration_threshold && input.concentration_threshold <= 100.0)
    {
        return Err(
            "concentration threshold must be finite and greater than 0 through 100".to_string(),
        );
    }
    let mut category_ids = HashSet::new();
    let mut total = 0.0;
    for target in &input.targets {
        if !category_ids.insert(&target.category_id) {
            return Err(format!("duplicate target category {}", target.category_id));
        }
        if !target.target_percent.is_finite() || !(0.0..=100.0).contains(&target.target_percent) {
            return Err(format!(
                "target for category {} must be finite and between 0 and 100",
                target.category_id
            ));
        }
        total += target.target_percent;
    }
    if !target_total_is_within_tolerance(total) {
        return Err("target percentages must total 100 within 0.01".to_string());
    }
    Ok(())
}

fn target_total_is_within_tolerance(total: f64) -> bool {
    if !total.is_finite() {
        return false;
    }
    let lower_bound = (100.0 - TOTAL_TOLERANCE).next_down();
    let upper_bound = (100.0 + TOTAL_TOLERANCE).next_up();
    total >= lower_bound && total <= upper_bound
}

fn validate_scope_currency(
    connection: &Connection,
    input: &SavePortfolioAlertConfigInput,
) -> Result<(), String> {
    match input.scope.kind {
        PortfolioAlertScopeKind::Overall => Ok(()),
        PortfolioAlertScopeKind::Market => {
            let market = validated_market(input.scope.market.as_deref())?;
            let expected = native_currency(market);
            if input.base_currency == expected {
                Ok(())
            } else {
                Err(format!("market {market} requires base currency {expected}"))
            }
        }
        PortfolioAlertScopeKind::Account => {
            let account_id = input
                .scope
                .account_id
                .as_deref()
                .ok_or_else(|| "account scope requires an account id".to_string())?;
            let market = connection
                .query_row(
                    "SELECT market FROM accounts WHERE id = ?1",
                    [account_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()
                .map_err(|error| error.to_string())?
                .ok_or_else(|| format!("account {account_id} not found"))?;
            let expected = native_currency(&market);
            if input.base_currency == expected {
                Ok(())
            } else {
                Err(format!(
                    "account {account_id} requires base currency {expected}"
                ))
            }
        }
    }
}

fn validate_target_categories(
    connection: &Connection,
    targets: &[PortfolioAlertTarget],
) -> Result<(), String> {
    for target in targets {
        let exists = connection
            .query_row(
                "SELECT 1 FROM categories WHERE id = ?1",
                [&target.category_id],
                |_| Ok(()),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .is_some();
        if !exists {
            return Err(format!("unknown target category {}", target.category_id));
        }
    }
    Ok(())
}

fn load_config_by_column(
    connection: &Connection,
    column: &str,
    value: &str,
) -> Result<Option<PortfolioAlertConfig>, String> {
    let query = match column {
        "id" => "SELECT id, scope_kind, market, account_id, base_currency, deviation_threshold, concentration_threshold, is_active, last_snapshot_json, last_evaluated_at FROM portfolio_alert_configs WHERE id = ?1",
        "scope_key" => "SELECT id, scope_kind, market, account_id, base_currency, deviation_threshold, concentration_threshold, is_active, last_snapshot_json, last_evaluated_at FROM portfolio_alert_configs WHERE scope_key = ?1",
        _ => return Err("invalid portfolio alert config lookup".to_string()),
    };
    let row = connection
        .query_row(query, [value], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, f64>(5)?,
                row.get::<_, f64>(6)?,
                row.get::<_, i32>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, Option<String>>(9)?,
            ))
        })
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((
        id,
        kind,
        market,
        account_id,
        base_currency,
        deviation_threshold,
        concentration_threshold,
        is_active,
        snapshot_json,
        last_evaluated_at,
    )) = row
    else {
        return Ok(None);
    };
    let kind = match kind.as_str() {
        "OVERALL" => PortfolioAlertScopeKind::Overall,
        "MARKET" => PortfolioAlertScopeKind::Market,
        "ACCOUNT" => PortfolioAlertScopeKind::Account,
        _ => return Err(format!("invalid stored portfolio alert scope kind {kind}")),
    };
    let mut statement = connection.prepare(
        "SELECT category_id, target_percent FROM portfolio_alert_targets WHERE config_id = ?1 ORDER BY rowid",
    ).map_err(|error| error.to_string())?;
    let targets = statement
        .query_map([&id], |row| {
            Ok(PortfolioAlertTarget {
                category_id: row.get(0)?,
                target_percent: row.get(1)?,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let last_snapshot = snapshot_json
        .map(|json| {
            serde_json::from_str::<PortfolioAlertSnapshot>(&json).map_err(|error| error.to_string())
        })
        .transpose()?;
    Ok(Some(PortfolioAlertConfig {
        id,
        scope: PortfolioAlertScope {
            kind,
            market,
            account_id,
        },
        base_currency,
        deviation_threshold,
        concentration_threshold,
        is_active: is_active != 0,
        targets,
        last_snapshot,
        last_evaluated_at,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        db::Database,
        models::portfolio_alert::{
            PortfolioAlertScope, PortfolioAlertScopeKind, PortfolioAlertTarget,
            SavePortfolioAlertConfigInput,
        },
    };
    use std::collections::HashSet;

    fn configured_db() -> Database {
        Database::new(":memory:").unwrap()
    }

    fn seed_categories<const N: usize>(db: &Database, ids: [&str; N]) {
        let conn = db.conn.lock().unwrap();
        for id in ids {
            conn.execute(
                "INSERT INTO categories (id, name, color, icon, created_at)
                 VALUES (?1, ?1, '#123456', 'icon', '2026-09-06')",
                [id],
            )
            .unwrap();
        }
    }

    fn seed_account(db: &Database, id: &str, market: &str) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO accounts (id, name, market, created_at, updated_at)
             VALUES (?1, ?1, ?2, '2026-09-06', '2026-09-06')",
                rusqlite::params![id, market],
            )
            .unwrap();
    }

    fn overall_scope() -> PortfolioAlertScope {
        PortfolioAlertScope {
            kind: PortfolioAlertScopeKind::Overall,
            market: None,
            account_id: None,
        }
    }

    fn market_scope(market: &str) -> PortfolioAlertScope {
        PortfolioAlertScope {
            kind: PortfolioAlertScopeKind::Market,
            market: Some(market.to_string()),
            account_id: None,
        }
    }

    fn account_scope(account_id: &str) -> PortfolioAlertScope {
        PortfolioAlertScope {
            kind: PortfolioAlertScopeKind::Account,
            market: None,
            account_id: Some(account_id.to_string()),
        }
    }

    fn targets<const N: usize>(items: [(&str, f64); N]) -> Vec<PortfolioAlertTarget> {
        items
            .into_iter()
            .map(|(category_id, target_percent)| PortfolioAlertTarget {
                category_id: category_id.to_string(),
                target_percent,
            })
            .collect()
    }

    fn native_currency(scope: &PortfolioAlertScope) -> &str {
        match scope.market.as_deref() {
            Some("CN") => "CNY",
            Some("HK") => "HKD",
            _ => "USD",
        }
    }

    fn input<const N: usize>(
        scope: PortfolioAlertScope,
        deviation_threshold: f64,
        concentration_threshold: f64,
        target_items: [(&str, f64); N],
    ) -> SavePortfolioAlertConfigInput {
        let base_currency = if scope.kind == PortfolioAlertScopeKind::Overall {
            "USD".to_string()
        } else {
            native_currency(&scope).to_string()
        };
        SavePortfolioAlertConfigInput {
            id: None,
            scope,
            base_currency,
            deviation_threshold,
            concentration_threshold,
            is_active: true,
            targets: targets(target_items),
        }
    }

    fn input_with_id<const N: usize>(
        id: String,
        scope: PortfolioAlertScope,
        deviation_threshold: f64,
        concentration_threshold: f64,
        target_items: [(&str, f64); N],
    ) -> SavePortfolioAlertConfigInput {
        let mut value = input(
            scope,
            deviation_threshold,
            concentration_threshold,
            target_items,
        );
        value.id = Some(id);
        value
    }

    fn save(
        db: &Database,
        scope: PortfolioAlertScope,
    ) -> crate::models::portfolio_alert::PortfolioAlertConfig {
        save_portfolio_alert_config(
            &db,
            input(scope, 20.0, 20.0, [("growth", 60.0), ("bonds", 40.0)]),
        )
        .unwrap()
    }

    fn config_count(db: &Database) -> i64 {
        db.conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM portfolio_alert_configs", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn insert_breach(db: &Database, config_id: &str) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO portfolio_alert_breaches
             (config_id, breach_key, breach_kind, direction, first_triggered_at, last_seen_at)
             VALUES (?1, 'category:growth', 'CATEGORY_DEVIATION', 'OVERWEIGHT', 'now', 'now')",
                [config_id],
            )
            .unwrap();
    }

    #[test]
    fn save_config_replaces_targets_atomically_and_preserves_scope_identity() {
        let db = configured_db();
        seed_categories(&db, ["growth", "bonds"]);
        let first = save(&db, overall_scope());
        insert_breach(&db, &first.id);

        let updated = save_portfolio_alert_config(
            &db,
            input_with_id(
                first.id.clone(),
                overall_scope(),
                15.0,
                25.0,
                [("growth", 50.0), ("bonds", 50.0)],
            ),
        )
        .unwrap();

        assert_eq!(updated.id, first.id);
        assert_eq!(
            updated.targets,
            targets([("growth", 50.0), ("bonds", 50.0)])
        );
        assert_eq!(config_count(&db), 1);
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row(
                    "SELECT COUNT(*) FROM portfolio_alert_breaches WHERE config_id = ?1",
                    [&updated.id],
                    |row| row.get::<_, i64>(0)
                )
                .unwrap(),
            0
        );
    }

    #[test]
    fn save_config_rejects_invalid_totals_and_mismatched_ids() {
        let db = configured_db();
        seed_categories(&db, ["growth", "bonds"]);
        assert!(save_portfolio_alert_config(
            &db,
            input(
                overall_scope(),
                20.0,
                20.0,
                [("growth", 70.0), ("bonds", 20.0)]
            )
        )
        .unwrap_err()
        .contains("100"));

        let saved = save_portfolio_alert_config(
            &db,
            input(
                market_scope("US"),
                20.0,
                20.0,
                [("growth", 60.0), ("bonds", 40.0)],
            ),
        )
        .unwrap();
        let error = save_portfolio_alert_config(
            &db,
            input_with_id(
                saved.id,
                market_scope("CN"),
                20.0,
                20.0,
                [("growth", 60.0), ("bonds", 40.0)],
            ),
        )
        .unwrap_err();
        assert!(error.contains("scope"));
    }

    #[test]
    fn overall_market_and_each_account_keep_independent_configs() {
        let db = configured_db();
        seed_categories(&db, ["growth", "bonds"]);
        seed_account(&db, "acct-us", "US");
        let configs = [
            save(&db, overall_scope()),
            save(&db, market_scope("US")),
            save(&db, account_scope("acct-us")),
        ];
        assert_eq!(
            configs
                .iter()
                .map(|config| &config.id)
                .collect::<HashSet<_>>()
                .len(),
            3
        );
    }

    #[test]
    fn failed_target_replacement_rolls_back_the_entire_configuration_save() {
        let db = configured_db();
        seed_categories(&db, ["growth", "bonds"]);
        let first = save(&db, overall_scope());
        insert_breach(&db, &first.id);
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER reject_bonds_target
             BEFORE INSERT ON portfolio_alert_targets
             WHEN NEW.category_id = 'bonds'
             BEGIN SELECT RAISE(ABORT, 'forced target insert failure'); END;",
            )
            .unwrap();

        let error = save_portfolio_alert_config(
            &db,
            input_with_id(
                first.id.clone(),
                overall_scope(),
                15.0,
                25.0,
                [("growth", 50.0), ("bonds", 50.0)],
            ),
        )
        .unwrap_err();
        assert!(error.contains("forced target insert failure"));
        let unchanged = get_portfolio_alert_config_by_id(&db, &first.id).unwrap();
        assert_eq!(
            unchanged.targets,
            targets([("growth", 60.0), ("bonds", 40.0)])
        );
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM portfolio_alert_breaches", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            1
        );
    }

    #[test]
    fn save_config_validates_categories_thresholds_and_total_tolerance() {
        let db = configured_db();
        seed_categories(&db, ["growth", "bonds"]);
        for items in [
            [("growth", 60.0), ("growth", 40.0)],
            [("growth", 60.0), ("unknown", 40.0)],
            [("growth", 60.0), ("bonds", 39.98)],
            [("growth", 60.0), ("bonds", 40.02)],
        ] {
            assert!(
                save_portfolio_alert_config(&db, input(overall_scope(), 20.0, 20.0, items))
                    .is_err()
            );
        }
        for total in [99.99, 100.01] {
            assert!(save_portfolio_alert_config(
                &db,
                input(
                    overall_scope(),
                    20.0,
                    20.0,
                    [("growth", 60.0), ("bonds", total - 60.0)]
                )
            )
            .is_ok());
        }
        for total in [99.989_999_999_999, 100.010_000_000_001] {
            assert!(save_portfolio_alert_config(
                &db,
                input(
                    overall_scope(),
                    20.0,
                    20.0,
                    [("growth", 60.0), ("bonds", total - 60.0)]
                )
            )
            .is_err());
        }
        for (deviation, concentration) in [
            (f64::NAN, 20.0),
            (20.0, f64::INFINITY),
            (-0.01, 20.0),
            (100.01, 20.0),
            (20.0, 0.0),
            (20.0, 100.01),
        ] {
            assert!(save_portfolio_alert_config(
                &db,
                input(
                    overall_scope(),
                    deviation,
                    concentration,
                    [("growth", 60.0), ("bonds", 40.0)]
                )
            )
            .is_err());
        }
    }

    #[test]
    fn save_config_enforces_scope_and_native_currency_rules() {
        let db = configured_db();
        seed_categories(&db, ["growth", "bonds"]);
        seed_account(&db, "acct-cn", "CN");
        let mut wrong_market_currency = input(
            market_scope("US"),
            20.0,
            20.0,
            [("growth", 60.0), ("bonds", 40.0)],
        );
        wrong_market_currency.base_currency = "CNY".to_string();
        assert!(save_portfolio_alert_config(&db, wrong_market_currency).is_err());
        for market in ["", "EU"] {
            assert!(save_portfolio_alert_config(
                &db,
                input(
                    market_scope(market),
                    20.0,
                    20.0,
                    [("growth", 60.0), ("bonds", 40.0)]
                )
            )
            .is_err());
        }
        let mut invalid_overall = input(
            overall_scope(),
            20.0,
            20.0,
            [("growth", 60.0), ("bonds", 40.0)],
        );
        invalid_overall.base_currency = "EUR".to_string();
        assert!(save_portfolio_alert_config(&db, invalid_overall).is_err());
        for currency in ["CNY", "HKD"] {
            let mut overall = input(
                overall_scope(),
                20.0,
                20.0,
                [("growth", 60.0), ("bonds", 40.0)],
            );
            overall.base_currency = currency.to_string();
            assert!(save_portfolio_alert_config(&db, overall).is_ok());
        }
        assert!(save_portfolio_alert_config(
            &db,
            input(
                market_scope("CN"),
                20.0,
                20.0,
                [("growth", 60.0), ("bonds", 40.0)]
            )
        )
        .is_ok());
        assert!(save_portfolio_alert_config(
            &db,
            input(
                market_scope("HK"),
                20.0,
                20.0,
                [("growth", 60.0), ("bonds", 40.0)]
            )
        )
        .is_ok());
        let mut wrong_account_currency = input(
            account_scope("acct-cn"),
            20.0,
            20.0,
            [("growth", 60.0), ("bonds", 40.0)],
        );
        wrong_account_currency.base_currency = "USD".to_string();
        assert!(save_portfolio_alert_config(&db, wrong_account_currency).is_err());
        assert!(save_portfolio_alert_config(
            &db,
            input(
                account_scope("missing"),
                20.0,
                20.0,
                [("growth", 60.0), ("bonds", 40.0)]
            )
        )
        .is_err());
    }

    #[test]
    fn scope_keys_and_scope_lookup_use_canonical_scope_identity() {
        let db = configured_db();
        seed_categories(&db, ["growth", "bonds"]);
        let config = save(&db, market_scope("US"));

        assert_eq!(scope_key(&overall_scope()).unwrap(), "overall");
        assert_eq!(scope_key(&market_scope("US")).unwrap(), "market:US");
        assert_eq!(
            scope_key(&account_scope("acct-us")).unwrap(),
            "account:acct-us"
        );
        assert_eq!(
            get_portfolio_alert_config_by_scope(&db, &market_scope("US"))
                .unwrap()
                .unwrap()
                .id,
            config.id
        );
        assert!(
            get_portfolio_alert_config_by_scope(&db, &market_scope("CN"))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn account_deletion_cascades_configuration_targets_and_breaches() {
        let db = configured_db();
        seed_categories(&db, ["growth", "bonds"]);
        seed_account(&db, "acct-us", "US");
        let config = save(&db, account_scope("acct-us"));
        insert_breach(&db, &config.id);
        db.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM accounts WHERE id = 'acct-us'", [])
            .unwrap();

        let conn = db.conn.lock().unwrap();
        for table in [
            "portfolio_alert_configs",
            "portfolio_alert_targets",
            "portfolio_alert_breaches",
        ] {
            assert_eq!(
                conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| row
                    .get::<_, i64>(0))
                    .unwrap(),
                0,
                "{table}"
            );
        }
    }

    #[test]
    fn disabling_config_clears_breaches_while_enabling_preserves_them() {
        let db = configured_db();
        seed_categories(&db, ["growth", "bonds"]);
        let config = save(&db, overall_scope());
        insert_breach(&db, &config.id);
        assert!(
            !set_portfolio_alert_active(&db, &config.id, false)
                .unwrap()
                .is_active
        );
        assert_eq!(
            db.conn
                .lock()
                .unwrap()
                .query_row("SELECT COUNT(*) FROM portfolio_alert_breaches", [], |row| {
                    row.get::<_, i64>(0)
                })
                .unwrap(),
            0
        );
        assert!(
            set_portfolio_alert_active(&db, &config.id, true)
                .unwrap()
                .is_active
        );
    }

    struct EvaluationFixture {
        db: Database,
        quote_cache: crate::services::quote_service::QuoteCache,
        rates: crate::models::ExchangeRates,
        config_id: String,
    }

    fn rates() -> crate::models::ExchangeRates {
        crate::models::ExchangeRates {
            usd_cny: 5.0,
            usd_hkd: 8.0,
            cny_hkd: 1.6,
            updated_at: "2026-09-06T09:00:00Z".to_string(),
        }
    }

    fn quote(market: &str, symbol: &str, price: f64) -> crate::models::StockQuote {
        crate::models::StockQuote {
            market: market.to_string(),
            symbol: symbol.to_string(),
            name: symbol.to_string(),
            current_price: price,
            previous_close: price,
            updated_at: "2026-09-06T09:00:00Z".to_string(),
            ..crate::models::StockQuote::default()
        }
    }

    fn seed_holding(
        db: &Database,
        id: &str,
        account_id: &str,
        symbol: &str,
        market: &str,
        category_id: Option<&str>,
        shares: f64,
        currency: &str,
    ) {
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO holdings
                 (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?3, ?4, ?5, ?6, 1, ?7, '2026-09-06', '2026-09-06')",
                rusqlite::params![id, account_id, symbol, market, category_id, shares, currency],
            )
            .unwrap();
    }

    fn evaluation_fixture() -> EvaluationFixture {
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        seed_account(&db, "acct-us", "US");
        seed_holding(
            &db,
            "holding-aapl",
            "acct-us",
            "AAPL",
            "US",
            Some("growth"),
            10.0,
            "USD",
        );
        seed_holding(
            &db,
            "holding-cash",
            "acct-us",
            "$CASH-CNY",
            "US",
            Some("growth"),
            500.0,
            "CNY",
        );
        let config = save_portfolio_alert_config(
            &db,
            input(overall_scope(), 20.0, 60.0, [("growth", 100.0)]),
        )
        .unwrap();
        let quote_cache = crate::services::quote_service::QuoteCache::new();
        quote_cache.set(quote("US", "AAPL", 100.0));
        EvaluationFixture {
            db,
            quote_cache,
            rates: rates(),
            config_id: config.id,
        }
    }

    fn breach_count(db: &Database) -> i64 {
        db.conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM portfolio_alert_breaches", [], |row| {
                row.get(0)
            })
            .unwrap()
    }

    fn breach_keys(db: &Database) -> Vec<String> {
        let conn = db.conn.lock().unwrap();
        let mut statement = conn
            .prepare("SELECT breach_key FROM portfolio_alert_breaches ORDER BY breach_key")
            .unwrap();
        statement
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    }

    fn alert_state(db: &Database, config_id: &str) -> (Option<String>, Option<String>, i64) {
        let conn = db.conn.lock().unwrap();
        let (snapshot, evaluated_at) = conn
            .query_row(
                "SELECT last_snapshot_json, last_evaluated_at
                 FROM portfolio_alert_configs WHERE id = ?1",
                [config_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        let breaches = conn
            .query_row(
                "SELECT COUNT(*) FROM portfolio_alert_breaches WHERE config_id = ?1",
                [config_id],
                |row| row.get(0),
            )
            .unwrap();
        (snapshot, evaluated_at, breaches)
    }

    async fn persist_fixture_breach(fixture: &EvaluationFixture) {
        let result = evaluate_portfolio_alert(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            "2026-09-06T09:00:00Z",
        )
        .await
        .unwrap();
        assert!(!result.active_breaches.is_empty());
    }

    #[test]
    fn rebalance_fix_intersection_requires_key_kind_and_direction_and_keeps_proposed_identity() {
        let proposed = PortfolioAlertBreach {
            config_id: "config-1".to_string(),
            breach_key: "category:growth".to_string(),
            breach_kind: PortfolioAlertBreachKind::CategoryDeviation,
            direction: PortfolioAlertBreachDirection::Underweight,
            first_triggered_at: "preview-first".to_string(),
            last_seen_at: "preview-last".to_string(),
        };
        let wrong_direction = PortfolioAlertBreach {
            direction: PortfolioAlertBreachDirection::Overweight,
            first_triggered_at: "persisted-first".to_string(),
            last_seen_at: "persisted-last".to_string(),
            ..proposed.clone()
        };
        assert!(intersect_preview_breaches(&[wrong_direction], &[proposed.clone()]).is_empty());
        let wrong_kind = PortfolioAlertBreach {
            breach_kind: PortfolioAlertBreachKind::Concentration,
            direction: PortfolioAlertBreachDirection::AboveLimit,
            first_triggered_at: "persisted-first".to_string(),
            last_seen_at: "persisted-last".to_string(),
            ..proposed.clone()
        };
        assert!(intersect_preview_breaches(&[wrong_kind], &[proposed.clone()]).is_empty());

        let persisted = PortfolioAlertBreach {
            first_triggered_at: "persisted-first".to_string(),
            last_seen_at: "persisted-last".to_string(),
            ..proposed.clone()
        };
        let matched = intersect_preview_breaches(&[persisted], &[proposed]);
        assert_eq!(matched.len(), 1);
        assert_eq!(
            matched[0].breach_kind,
            PortfolioAlertBreachKind::CategoryDeviation
        );
        assert_eq!(
            matched[0].direction,
            PortfolioAlertBreachDirection::Underweight
        );
        assert_eq!(matched[0].first_triggered_at, "persisted-first");
        assert_eq!(matched[0].last_seen_at, "persisted-last");
    }

    #[tokio::test]
    async fn rebalance_fix_preview_rejects_config_active_or_revision_changes_without_writes() {
        for mutation in ["inactive", "revision"] {
            let fixture = evaluation_fixture();
            persist_fixture_breach(&fixture).await;
            let before = alert_state(&fixture.db, &fixture.config_id);
            let result = preview_portfolio_alert_with_before_validate_hook(
                &fixture.db,
                &fixture.quote_cache,
                Some(&fixture.rates),
                &fixture.config_id,
                "2026-09-06T10:00:00Z",
                || {
                    let conn = fixture.db.conn.lock().unwrap();
                    if mutation == "inactive" {
                        conn.execute(
                            "UPDATE portfolio_alert_configs SET is_active = 0 WHERE id = ?1",
                            [&fixture.config_id],
                        )
                        .unwrap();
                    } else {
                        conn.execute(
                            "UPDATE portfolio_alert_configs
                             SET deviation_threshold = 25, updated_at = 'changed'
                             WHERE id = ?1",
                            [&fixture.config_id],
                        )
                        .unwrap();
                    }
                },
            )
            .await;
            assert!(result.is_err(), "{mutation} was accepted");
            let after = alert_state(&fixture.db, &fixture.config_id);
            assert_eq!(after.0, before.0, "{mutation} rewrote snapshot");
            assert_eq!(after.1, before.1, "{mutation} rewrote evaluated_at");
            assert_eq!(after.2, before.2, "{mutation} rewrote breaches");
        }
    }

    #[tokio::test]
    async fn rebalance_fix_preview_revalidates_account_market_and_active_breaches() {
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        seed_account(&db, "acct-us", "US");
        seed_holding(
            &db,
            "holding-aapl",
            "acct-us",
            "AAPL",
            "US",
            Some("growth"),
            10.0,
            "USD",
        );
        let config = save_portfolio_alert_config(
            &db,
            input(account_scope("acct-us"), 20.0, 60.0, [("growth", 100.0)]),
        )
        .unwrap();
        let quote_cache = QuoteCache::new();
        quote_cache.set(quote("US", "AAPL", 100.0));
        evaluate_portfolio_alert(&db, &quote_cache, None, &config.id, "2026-09-06T09:00:00Z")
            .await
            .unwrap();

        let before_market_change = alert_state(&db, &config.id);
        let changed_market = preview_portfolio_alert_with_before_validate_hook(
            &db,
            &quote_cache,
            None,
            &config.id,
            "2026-09-06T10:00:00Z",
            || {
                db.conn
                    .lock()
                    .unwrap()
                    .execute("UPDATE accounts SET market = 'CN' WHERE id = 'acct-us'", [])
                    .unwrap();
            },
        )
        .await;
        assert!(changed_market.is_err());
        assert_eq!(alert_state(&db, &config.id), before_market_change);

        db.conn
            .lock()
            .unwrap()
            .execute("UPDATE accounts SET market = 'US' WHERE id = 'acct-us'", [])
            .unwrap();
        let before_breach_resolution = alert_state(&db, &config.id);
        let resolved = preview_portfolio_alert_with_before_validate_hook(
            &db,
            &quote_cache,
            None,
            &config.id,
            "2026-09-06T10:00:00Z",
            || {
                db.conn
                    .lock()
                    .unwrap()
                    .execute(
                        "DELETE FROM portfolio_alert_breaches WHERE config_id = ?1",
                        [&config.id],
                    )
                    .unwrap();
            },
        )
        .await;
        assert!(resolved.is_err());
        let after_breach_resolution = alert_state(&db, &config.id);
        assert_eq!(after_breach_resolution.0, before_breach_resolution.0);
        assert_eq!(after_breach_resolution.1, before_breach_resolution.1);
        assert_eq!(after_breach_resolution.2, 0);
    }

    #[tokio::test]
    async fn rebalance_fix_preview_returns_frozen_positions_and_values_cash_at_one() {
        let fixture = evaluation_fixture();
        persist_fixture_breach(&fixture).await;
        let preview = preview_portfolio_alert_with_before_validate_hook(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            "2026-09-06T10:00:00Z",
            || {
                fixture
                    .db
                    .conn
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE holdings SET shares = 99 WHERE id = 'holding-aapl'",
                        [],
                    )
                    .unwrap();
                fixture.quote_cache.set(quote("US", "AAPL", 999.0));
            },
        )
        .await
        .unwrap();

        assert_eq!(
            preview
                .evaluation
                .snapshot
                .as_ref()
                .unwrap()
                .total_market_value,
            1100.0
        );
        let stock = preview
            .positions
            .iter()
            .find(|position| position.symbol == "AAPL")
            .unwrap();
        assert_eq!(stock.shares, 10.0);
        assert_eq!(stock.current_price, 100.0);
        assert_eq!(stock.native_market_value, 1000.0);
        assert_eq!(stock.base_market_value, 1000.0);
        assert_eq!(
            stock.quote_updated_at.as_deref(),
            Some("2026-09-06T09:00:00Z")
        );

        let cash = preview
            .positions
            .iter()
            .find(|position| position.symbol == "$CASH-CNY")
            .unwrap();
        assert_eq!(cash.current_price, 1.0);
        assert_eq!(cash.native_market_value, 500.0);
        assert_eq!(cash.base_market_value, 100.0);
        assert_eq!(cash.native_currency, "CNY");
        assert_eq!(cash.base_currency, "USD");
        assert_eq!(cash.conversion_rate, 0.2);
        assert_eq!(
            cash.exchange_rate_updated_at.as_deref(),
            Some("2026-09-06T09:00:00Z")
        );
        assert_eq!(cash.quote_updated_at, None);
    }

    #[tokio::test]
    async fn evaluate_all_active_skips_disabled_configs_and_collects_only_new_breaches() {
        // This catches a batch evaluator that includes disabled configurations
        // or reports an already-persisted breach as new.
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        seed_account(&db, "acct-us", "US");
        seed_holding(
            &db,
            "holding-aapl",
            "acct-us",
            "AAPL",
            "US",
            Some("growth"),
            10.0,
            "USD",
        );
        let active = save_portfolio_alert_config(
            &db,
            input_with_id(
                "config-us".to_string(),
                market_scope("US"),
                20.0,
                60.0,
                [("growth", 100.0)],
            ),
        )
        .unwrap();
        let disabled = save_portfolio_alert_config(
            &db,
            input_with_id(
                "config-hk".to_string(),
                market_scope("HK"),
                20.0,
                60.0,
                [("growth", 100.0)],
            ),
        )
        .unwrap();
        set_portfolio_alert_active(&db, &disabled.id, false).unwrap();
        let quote_cache = crate::services::quote_service::QuoteCache::new();
        quote_cache.set(quote("US", "AAPL", 100.0));

        let first = evaluate_all_active_portfolio_alerts(
            &db,
            &quote_cache,
            Some(&rates()),
            "2026-09-06T10:00:00Z",
        )
        .await
        .unwrap();

        assert_eq!(
            first
                .iter()
                .map(|notification| notification.config_id.as_str())
                .collect::<Vec<_>>(),
            vec!["config-us"]
        );
        assert_eq!(first[0].scope, market_scope("US"));
        assert_eq!(first[0].triggered_at, "2026-09-06T10:00:00Z");
        assert_eq!(first[0].breach.config_id, active.id);
        assert!(first[0].message.contains("预警"));
        assert!(get_portfolio_alert_config_by_id(&db, &disabled.id)
            .unwrap()
            .last_evaluated_at
            .is_none());

        let second = evaluate_all_active_portfolio_alerts(
            &db,
            &quote_cache,
            Some(&rates()),
            "2026-09-06T10:05:00Z",
        )
        .await
        .unwrap();
        assert!(second.is_empty());
    }

    #[tokio::test]
    async fn evaluate_all_active_continues_after_incomplete_conversion_scope() {
        // This catches a batch evaluator that stops when one active scope is
        // incomplete because its currency conversion rate is unavailable.
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        seed_account(&db, "acct-us", "US");
        seed_account(&db, "acct-hk", "HK");
        seed_holding(
            &db,
            "holding-aapl",
            "acct-us",
            "AAPL",
            "US",
            Some("growth"),
            10.0,
            "USD",
        );
        seed_holding(
            &db,
            "holding-0700",
            "acct-hk",
            "0700",
            "HK",
            Some("growth"),
            10.0,
            "HKD",
        );
        let incomplete = save_portfolio_alert_config(
            &db,
            input_with_id(
                "config-overall".to_string(),
                overall_scope(),
                20.0,
                60.0,
                [("growth", 100.0)],
            ),
        )
        .unwrap();
        let ready = save_portfolio_alert_config(
            &db,
            input_with_id(
                "config-us".to_string(),
                market_scope("US"),
                20.0,
                60.0,
                [("growth", 100.0)],
            ),
        )
        .unwrap();
        let quote_cache = crate::services::quote_service::QuoteCache::new();
        quote_cache.set(quote("US", "AAPL", 100.0));
        quote_cache.set(quote("HK", "0700", 100.0));

        let notifications =
            evaluate_all_active_portfolio_alerts(&db, &quote_cache, None, "2026-09-06T10:00:00Z")
                .await
                .unwrap();

        assert_eq!(
            notifications
                .iter()
                .map(|notification| notification.config_id.as_str())
                .collect::<Vec<_>>(),
            vec!["config-us"]
        );
        assert_eq!(
            get_portfolio_alert_config_by_id(&db, &incomplete.id)
                .unwrap()
                .last_evaluated_at,
            None
        );
        assert_eq!(
            get_portfolio_alert_config_by_id(&db, &ready.id)
                .unwrap()
                .last_evaluated_at
                .as_deref(),
            Some("2026-09-06T10:00:00Z")
        );
    }

    #[tokio::test]
    async fn evaluate_all_active_uses_one_frozen_quote_snapshot_for_every_scope() {
        // This catches later configurations rereading the live cache after a
        // concurrent quote update changed it during the batch.
        let db = configured_db();
        seed_categories(&db, ["growth", "cash"]);
        seed_account(&db, "acct-us", "US");
        seed_holding(
            &db,
            "holding-aapl",
            "acct-us",
            "AAPL",
            "US",
            Some("growth"),
            1.0,
            "USD",
        );
        seed_holding(
            &db,
            "holding-cash",
            "acct-us",
            "$CASH-USD",
            "US",
            Some("cash"),
            100.0,
            "USD",
        );
        save_portfolio_alert_config(
            &db,
            input_with_id(
                "config-a".to_string(),
                market_scope("US"),
                20.0,
                90.0,
                [("growth", 50.0), ("cash", 50.0)],
            ),
        )
        .unwrap();
        save_portfolio_alert_config(
            &db,
            input_with_id(
                "config-b".to_string(),
                overall_scope(),
                20.0,
                90.0,
                [("growth", 50.0), ("cash", 50.0)],
            ),
        )
        .unwrap();
        let quote_cache = crate::services::quote_service::QuoteCache::new();
        quote_cache.set(quote("US", "AAPL", 100.0));

        let notifications = evaluate_all_active_with_before_config_hook(
            &db,
            &quote_cache,
            Some(&rates()),
            "2026-09-06T10:00:00Z",
            |index| {
                if index == 1 {
                    quote_cache.set(quote("US", "AAPL", 300.0));
                }
            },
        )
        .await
        .unwrap();

        assert!(notifications.is_empty());
    }

    #[tokio::test]
    async fn ready_evaluation_persists_snapshot_and_notifies_only_on_new_transition() {
        let fixture = evaluation_fixture();
        let first = evaluate_portfolio_alert(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            "2026-09-06T10:00:00Z",
        )
        .await
        .unwrap();
        assert_eq!(
            first.status,
            crate::models::portfolio_alert::PortfolioAlertDataStatus::Ready
        );
        assert_eq!(first.newly_triggered.len(), 1);
        assert_eq!(breach_count(&fixture.db), 1);
        let persisted = get_portfolio_alert_config_by_id(&fixture.db, &fixture.config_id).unwrap();
        assert_eq!(
            persisted.last_evaluated_at.as_deref(),
            Some("2026-09-06T10:00:00Z")
        );
        assert_eq!(persisted.last_snapshot, first.snapshot);

        let second = evaluate_portfolio_alert(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            "2026-09-06T10:05:00Z",
        )
        .await
        .unwrap();
        assert!(second.newly_triggered.is_empty());
        assert_eq!(breach_count(&fixture.db), 1);
        assert_eq!(
            second.active_breaches[0].first_triggered_at,
            "2026-09-06T10:00:00Z"
        );
        assert_eq!(
            second.active_breaches[0].last_seen_at,
            "2026-09-06T10:05:00Z"
        );
    }

    #[tokio::test]
    async fn recovery_removes_active_row_and_later_breach_notifies_again() {
        let fixture = evaluation_fixture();
        evaluate_portfolio_alert(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            "2026-09-06T10:00:00Z",
        )
        .await
        .unwrap();

        fixture.quote_cache.set(quote("US", "AAPL", 10.0));
        let recovered = evaluate_portfolio_alert(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            "2026-09-06T10:05:00Z",
        )
        .await
        .unwrap();
        assert!(recovered.active_breaches.is_empty());
        assert_eq!(breach_count(&fixture.db), 0);

        fixture.quote_cache.set(quote("US", "AAPL", 100.0));
        let rebreach = evaluate_portfolio_alert(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            "2026-09-06T10:10:00Z",
        )
        .await
        .unwrap();
        assert_eq!(rebreach.newly_triggered.len(), 1);
        assert_eq!(
            rebreach.newly_triggered[0].first_triggered_at,
            "2026-09-06T10:10:00Z"
        );
    }

    #[tokio::test]
    async fn incomplete_quotes_keep_last_snapshot_and_do_not_change_breaches() {
        let fixture = evaluation_fixture();
        let prior = evaluate_portfolio_alert(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            "2026-09-06T10:00:00Z",
        )
        .await
        .unwrap()
        .snapshot
        .unwrap();
        let prior_breach_keys = breach_keys(&fixture.db);
        let empty_cache = crate::services::quote_service::QuoteCache::new();

        let result = evaluate_portfolio_alert(
            &fixture.db,
            &empty_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            "2026-09-06T10:05:00Z",
        )
        .await
        .unwrap();

        assert_eq!(
            result.status,
            crate::models::portfolio_alert::PortfolioAlertDataStatus::Incomplete
        );
        assert!(result.stale);
        assert_eq!(result.snapshot, Some(prior));
        assert_eq!(breach_keys(&fixture.db), prior_breach_keys);
        assert!(result.newly_triggered.is_empty());
        assert_eq!(result.missing_data.len(), 1);
        assert_eq!(result.missing_data[0].market.as_deref(), Some("US"));
        assert_eq!(result.missing_data[0].symbol.as_deref(), Some("AAPL"));
    }

    #[tokio::test]
    async fn incomplete_quotes_are_checked_only_after_scope_filtering() {
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        seed_account(&db, "acct-us", "US");
        seed_account(&db, "acct-cn", "CN");
        seed_holding(
            &db,
            "us",
            "acct-us",
            "SAME",
            "US",
            Some("growth"),
            10.0,
            "USD",
        );
        seed_holding(
            &db,
            "cn",
            "acct-cn",
            "SAME",
            "CN",
            Some("growth"),
            10.0,
            "CNY",
        );
        let config = save_portfolio_alert_config(
            &db,
            input(market_scope("US"), 20.0, 100.0, [("growth", 100.0)]),
        )
        .unwrap();
        let cache = crate::services::quote_service::QuoteCache::new();
        cache.set(quote("US", "SAME", 10.0));

        let result =
            evaluate_portfolio_alert(&db, &cache, None, &config.id, "2026-09-06T10:00:00Z")
                .await
                .unwrap();

        assert_eq!(
            result.status,
            crate::models::portfolio_alert::PortfolioAlertDataStatus::Ready
        );
        assert_eq!(result.snapshot.unwrap().total_market_value, 100.0);
    }

    #[tokio::test]
    async fn account_scope_isolated_native_holdings_need_no_fx_cache() {
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        seed_account(&db, "acct-us-1", "US");
        seed_account(&db, "acct-us-2", "US");
        seed_holding(
            &db,
            "one",
            "acct-us-1",
            "AAPL",
            "US",
            Some("growth"),
            2.0,
            "USD",
        );
        seed_holding(
            &db,
            "two",
            "acct-us-2",
            "MSFT",
            "US",
            Some("growth"),
            9.0,
            "USD",
        );
        let config = save_portfolio_alert_config(
            &db,
            input(account_scope("acct-us-1"), 20.0, 100.0, [("growth", 100.0)]),
        )
        .unwrap();
        let cache = crate::services::quote_service::QuoteCache::new();
        cache.set(quote("US", "AAPL", 25.0));

        let result =
            evaluate_portfolio_alert(&db, &cache, None, &config.id, "2026-09-06T10:00:00Z")
                .await
                .unwrap();

        assert_eq!(
            result.status,
            crate::models::portfolio_alert::PortfolioAlertDataStatus::Ready
        );
        assert_eq!(result.snapshot.unwrap().total_market_value, 50.0);
    }

    #[tokio::test]
    async fn overall_missing_fx_keeps_state_stale_and_reports_currency_pair() {
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        seed_account(&db, "acct-cn", "CN");
        seed_holding(
            &db,
            "cn",
            "acct-cn",
            "600000",
            "CN",
            Some("growth"),
            10.0,
            "CNY",
        );
        let config = save_portfolio_alert_config(
            &db,
            input(overall_scope(), 20.0, 100.0, [("growth", 100.0)]),
        )
        .unwrap();
        let cache = crate::services::quote_service::QuoteCache::new();
        cache.set(quote("CN", "600000", 10.0));

        let result =
            evaluate_portfolio_alert(&db, &cache, None, &config.id, "2026-09-06T10:00:00Z")
                .await
                .unwrap();

        assert_eq!(
            result.status,
            crate::models::portfolio_alert::PortfolioAlertDataStatus::Incomplete
        );
        assert!(result.stale);
        assert!(result
            .missing_data
            .iter()
            .any(|item| item.currency.as_deref() == Some("CNY")));
        assert!(result.snapshot.is_none());
    }

    #[tokio::test]
    async fn non_finite_or_non_positive_required_fx_is_incomplete() {
        for usd_cny in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let db = configured_db();
            seed_categories(&db, ["growth"]);
            seed_account(&db, "acct-cn", "CN");
            seed_holding(
                &db,
                "cn",
                "acct-cn",
                "600000",
                "CN",
                Some("growth"),
                1.0,
                "CNY",
            );
            let config = save_portfolio_alert_config(
                &db,
                input(overall_scope(), 20.0, 100.0, [("growth", 100.0)]),
            )
            .unwrap();
            let cache = crate::services::quote_service::QuoteCache::new();
            cache.set(quote("CN", "600000", 10.0));
            let invalid_rates = crate::models::ExchangeRates { usd_cny, ..rates() };

            let result = evaluate_portfolio_alert(
                &db,
                &cache,
                Some(&invalid_rates),
                &config.id,
                "2026-09-06T10:00:00Z",
            )
            .await
            .unwrap();
            assert_eq!(
                result.status,
                crate::models::portfolio_alert::PortfolioAlertDataStatus::Incomplete
            );
        }
    }

    #[tokio::test]
    async fn empty_and_non_positive_portfolios_return_empty_without_breaches() {
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        let config = save_portfolio_alert_config(
            &db,
            input(overall_scope(), 20.0, 20.0, [("growth", 100.0)]),
        )
        .unwrap();
        let cache = crate::services::quote_service::QuoteCache::new();
        let no_holdings =
            evaluate_portfolio_alert(&db, &cache, None, &config.id, "2026-09-06T10:00:00Z")
                .await
                .unwrap();
        assert_eq!(
            no_holdings.status,
            crate::models::portfolio_alert::PortfolioAlertDataStatus::Empty
        );

        seed_account(&db, "acct-us", "US");
        seed_holding(
            &db,
            "zero",
            "acct-us",
            "ZERO",
            "US",
            Some("growth"),
            1.0,
            "USD",
        );
        cache.set(quote("US", "ZERO", 0.0));
        let zero = evaluate_portfolio_alert(&db, &cache, None, &config.id, "2026-09-06T10:05:00Z")
            .await
            .unwrap();
        assert_eq!(
            zero.status,
            crate::models::portfolio_alert::PortfolioAlertDataStatus::Empty
        );
        assert_eq!(breach_count(&db), 0);
    }

    #[tokio::test]
    async fn deleted_target_category_returns_invalid_config_without_changing_state() {
        let fixture = evaluation_fixture();
        let prior = evaluate_portfolio_alert(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            "2026-09-06T10:00:00Z",
        )
        .await
        .unwrap()
        .snapshot;
        let prior_keys = breach_keys(&fixture.db);
        fixture
            .db
            .conn
            .lock()
            .unwrap()
            .execute("DELETE FROM categories WHERE id = 'growth'", [])
            .unwrap();

        let result = evaluate_portfolio_alert(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            "2026-09-06T10:05:00Z",
        )
        .await
        .unwrap();

        assert_eq!(
            result.status,
            crate::models::portfolio_alert::PortfolioAlertDataStatus::InvalidConfig
        );
        assert_eq!(result.snapshot, prior);
        assert!(result.stale);
        assert_eq!(breach_keys(&fixture.db), prior_keys);
    }

    #[tokio::test]
    async fn cash_uses_native_value_without_quote_and_is_excluded_from_concentration() {
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        seed_account(&db, "acct-us", "US");
        seed_holding(
            &db,
            "cash",
            "acct-us",
            "$CASH-USD",
            "US",
            Some("growth"),
            250.0,
            "USD",
        );
        let config = save_portfolio_alert_config(
            &db,
            input(market_scope("US"), 20.0, 1.0, [("growth", 100.0)]),
        )
        .unwrap();

        let result = evaluate_portfolio_alert(
            &db,
            &crate::services::quote_service::QuoteCache::new(),
            None,
            &config.id,
            "2026-09-06T10:00:00Z",
        )
        .await
        .unwrap();

        assert_eq!(
            result.status,
            crate::models::portfolio_alert::PortfolioAlertDataStatus::Ready
        );
        let snapshot = result.snapshot.unwrap();
        assert_eq!(snapshot.total_market_value, 250.0);
        assert!(snapshot.concentrations.is_empty());
        assert!(result.missing_data.is_empty());
    }

    #[tokio::test]
    async fn same_market_symbol_is_aggregated_across_accounts_for_concentration() {
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        seed_account(&db, "one", "US");
        seed_account(&db, "two", "US");
        seed_holding(
            &db,
            "one-aapl",
            "one",
            "aapl",
            "US",
            Some("growth"),
            2.0,
            "USD",
        );
        seed_holding(
            &db,
            "two-aapl",
            "two",
            "aapl",
            "US",
            Some("growth"),
            3.0,
            "USD",
        );
        seed_holding(
            &db,
            "cash",
            "one",
            "$CASH-USD",
            "US",
            Some("growth"),
            50.0,
            "USD",
        );
        let config = save_portfolio_alert_config(
            &db,
            input(overall_scope(), 20.0, 40.0, [("growth", 100.0)]),
        )
        .unwrap();
        let cache = crate::services::quote_service::QuoteCache::new();
        cache.set(quote("US", "aapl", 10.0));

        let result =
            evaluate_portfolio_alert(&db, &cache, None, &config.id, "2026-09-06T10:00:00Z")
                .await
                .unwrap();
        let concentrations = result.snapshot.unwrap().concentrations;
        assert_eq!(concentrations.len(), 1);
        assert_eq!(concentrations[0].market_value, 50.0);
        assert_eq!(concentrations[0].normalized_symbol, "AAPL");
    }

    #[tokio::test]
    async fn saving_changed_config_clears_old_breaches_then_immediately_evaluates() {
        let fixture = evaluation_fixture();
        evaluate_portfolio_alert(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            "2026-09-06T10:00:00Z",
        )
        .await
        .unwrap();
        let mut changed = input(overall_scope(), 20.0, 95.0, [("growth", 100.0)]);
        changed.id = Some(fixture.config_id.clone());

        let view = save_and_evaluate_portfolio_alert_config(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            changed,
            "2026-09-06T10:05:00Z",
        )
        .await
        .unwrap();

        assert!(view.evaluation.unwrap().active_breaches.is_empty());
        assert_eq!(breach_count(&fixture.db), 0);
        assert_eq!(
            view.config.unwrap().last_evaluated_at.as_deref(),
            Some("2026-09-06T10:05:00Z")
        );
    }

    #[tokio::test]
    async fn disable_clears_and_skips_evaluation_then_enable_evaluates_current_data() {
        let fixture = evaluation_fixture();
        let disabled = set_portfolio_alert_active_and_evaluate(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            false,
            "2026-09-06T10:00:00Z",
        )
        .await
        .unwrap();
        assert!(disabled.evaluation.is_none());
        assert_eq!(breach_count(&fixture.db), 0);
        assert!(
            get_portfolio_alert_config_by_id(&fixture.db, &fixture.config_id)
                .unwrap()
                .last_evaluated_at
                .is_none()
        );

        let enabled = set_portfolio_alert_active_and_evaluate(
            &fixture.db,
            &fixture.quote_cache,
            Some(&fixture.rates),
            &fixture.config_id,
            true,
            "2026-09-06T10:05:00Z",
        )
        .await
        .unwrap();
        assert_eq!(enabled.evaluation.unwrap().newly_triggered.len(), 1);
        assert_eq!(breach_count(&fixture.db), 1);
    }

    #[tokio::test]
    async fn breach_write_failure_rolls_back_snapshot_timestamp_insert_update_and_delete() {
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        seed_account(&db, "acct-us", "US");
        seed_holding(
            &db,
            "persist",
            "acct-us",
            "PERSIST",
            "US",
            Some("growth"),
            4.0,
            "USD",
        );
        seed_holding(
            &db,
            "new",
            "acct-us",
            "NEW",
            "US",
            Some("growth"),
            4.0,
            "USD",
        );
        seed_holding(
            &db,
            "cash",
            "acct-us",
            "$CASH-USD",
            "US",
            Some("growth"),
            2.0,
            "USD",
        );
        let config = save_portfolio_alert_config(
            &db,
            input(overall_scope(), 20.0, 30.0, [("growth", 100.0)]),
        )
        .unwrap();
        let previous_snapshot = crate::models::portfolio_alert::PortfolioAlertSnapshot {
            config_id: config.id.clone(),
            scope: overall_scope(),
            base_currency: "USD".to_string(),
            evaluated_at: "old".to_string(),
            total_market_value: 1.0,
            categories: vec![],
            concentrations: vec![],
        };
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "UPDATE portfolio_alert_configs SET last_snapshot_json = ?1, last_evaluated_at = 'old' WHERE id = ?2",
                rusqlite::params![serde_json::to_string(&previous_snapshot).unwrap(), config.id],
            )
            .unwrap();
            for key in ["security:US:PERSIST", "security:US:RECOVER"] {
                conn.execute(
                    "INSERT INTO portfolio_alert_breaches
                     (config_id, breach_key, breach_kind, direction, first_triggered_at, last_seen_at)
                     VALUES (?1, ?2, 'CONCENTRATION', 'ABOVE_LIMIT', 'first', 'old')",
                    rusqlite::params![config.id, key],
                )
                .unwrap();
            }
            conn.execute_batch(
                "CREATE TRIGGER fail_new_breach BEFORE INSERT ON portfolio_alert_breaches
                 WHEN NEW.breach_key = 'security:US:NEW'
                 BEGIN SELECT RAISE(ABORT, 'forced breach write failure'); END;",
            )
            .unwrap();
        }
        let cache = crate::services::quote_service::QuoteCache::new();
        cache.set(quote("US", "PERSIST", 10.0));
        cache.set(quote("US", "NEW", 10.0));

        let error = evaluate_portfolio_alert(&db, &cache, None, &config.id, "new")
            .await
            .unwrap_err();

        assert!(error.contains("forced breach write failure"));
        let unchanged = get_portfolio_alert_config_by_id(&db, &config.id).unwrap();
        assert_eq!(unchanged.last_snapshot, Some(previous_snapshot));
        assert_eq!(unchanged.last_evaluated_at.as_deref(), Some("old"));
        assert_eq!(
            breach_keys(&db),
            vec!["security:US:PERSIST", "security:US:RECOVER"]
        );
        let last_seen: String = db.conn.lock().unwrap().query_row(
            "SELECT last_seen_at FROM portfolio_alert_breaches WHERE breach_key = 'security:US:PERSIST'",
            [],
            |row| row.get(0),
        ).unwrap();
        assert_eq!(last_seen, "old");
    }

    #[tokio::test]
    async fn overall_evaluation_uses_same_symbol_quotes_from_two_markets() {
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        seed_account(&db, "acct-us", "US");
        seed_account(&db, "acct-cn", "CN");
        seed_holding(
            &db,
            "us",
            "acct-us",
            "SAME",
            "US",
            Some("growth"),
            10.0,
            "USD",
        );
        seed_holding(
            &db,
            "cn",
            "acct-cn",
            "SAME",
            "CN",
            Some("growth"),
            10.0,
            "CNY",
        );
        let config = save_portfolio_alert_config(
            &db,
            input(overall_scope(), 20.0, 100.0, [("growth", 100.0)]),
        )
        .unwrap();
        let cache = crate::services::quote_service::QuoteCache::new();
        cache.set(quote("US", "SAME", 10.0));
        cache.set(quote("CN", "SAME", 20.0));

        let result = evaluate_portfolio_alert(
            &db,
            &cache,
            Some(&rates()),
            &config.id,
            "2026-09-06T10:00:00Z",
        )
        .await
        .unwrap();

        assert_eq!(result.status, PortfolioAlertDataStatus::Ready);
        assert_eq!(result.snapshot.unwrap().total_market_value, 140.0);
    }

    #[tokio::test]
    async fn finite_negative_scoped_total_returns_empty_not_incomplete() {
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        seed_account(&db, "acct-us", "US");
        seed_holding(
            &db,
            "negative",
            "acct-us",
            "NEG",
            "US",
            Some("growth"),
            2.0,
            "USD",
        );
        let config = save_portfolio_alert_config(
            &db,
            input(overall_scope(), 20.0, 20.0, [("growth", 100.0)]),
        )
        .unwrap();
        let cache = crate::services::quote_service::QuoteCache::new();
        cache.set(quote("US", "NEG", -5.0));

        let result = evaluate_portfolio_alert(&db, &cache, None, &config.id, "negative")
            .await
            .unwrap();

        assert_eq!(result.status, PortfolioAlertDataStatus::Empty);
        assert!(result.missing_data.is_empty());
        assert!(result.active_breaches.is_empty());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn evaluation_loaded_before_save_cannot_commit_old_snapshot_or_breaches() {
        let fixture = evaluation_fixture();
        let db = std::sync::Arc::new(fixture.db);
        let cache = std::sync::Arc::new(fixture.quote_cache);
        let rates = std::sync::Arc::new(fixture.rates);
        let config_id = fixture.config_id;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let task = {
            let db = db.clone();
            let cache = cache.clone();
            let rates = rates.clone();
            let config_id = config_id.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                evaluate_portfolio_alert_with_before_persist_hook(
                    &db,
                    &cache,
                    Some(&rates),
                    &config_id,
                    "old-evaluation",
                    move || {
                        barrier.wait();
                        barrier.wait();
                    },
                )
                .await
            })
        };
        barrier.wait();
        let mut changed = input(overall_scope(), 20.0, 95.0, [("growth", 100.0)]);
        changed.id = Some(config_id.clone());
        save_portfolio_alert_config(&db, changed).unwrap();
        barrier.wait();

        let error = task.await.unwrap().unwrap_err();

        assert!(error.contains("changed during evaluation"));
        let current = get_portfolio_alert_config_by_id(&db, &config_id).unwrap();
        assert!(current.last_snapshot.is_none());
        assert!(current.last_evaluated_at.is_none());
        assert_eq!(breach_count(&db), 0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn evaluation_loaded_before_disable_cannot_recreate_state() {
        let fixture = evaluation_fixture();
        let db = std::sync::Arc::new(fixture.db);
        let cache = std::sync::Arc::new(fixture.quote_cache);
        let rates = std::sync::Arc::new(fixture.rates);
        let config_id = fixture.config_id;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let task = {
            let db = db.clone();
            let cache = cache.clone();
            let rates = rates.clone();
            let config_id = config_id.clone();
            let barrier = barrier.clone();
            tokio::spawn(async move {
                evaluate_portfolio_alert_with_before_persist_hook(
                    &db,
                    &cache,
                    Some(&rates),
                    &config_id,
                    "old-evaluation",
                    move || {
                        barrier.wait();
                        barrier.wait();
                    },
                )
                .await
            })
        };
        barrier.wait();
        set_portfolio_alert_active(&db, &config_id, false).unwrap();
        barrier.wait();

        let error = task.await.unwrap().unwrap_err();

        assert!(error.contains("changed during evaluation") || error.contains("inactive"));
        let current = get_portfolio_alert_config_by_id(&db, &config_id).unwrap();
        assert!(!current.is_active);
        assert!(current.last_snapshot.is_none());
        assert!(current.last_evaluated_at.is_none());
        assert_eq!(breach_count(&db), 0);
    }

    #[test]
    fn identical_save_advances_a_parseable_guard_revision() {
        let db = configured_db();
        seed_categories(&db, ["growth"]);
        let config = save_portfolio_alert_config(
            &db,
            input(overall_scope(), 20.0, 60.0, [("growth", 100.0)]),
        )
        .unwrap();
        let before: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT updated_at FROM portfolio_alert_configs WHERE id = ?1",
                [&config.id],
                |row| row.get(0),
            )
            .unwrap();
        let mut identical = input(overall_scope(), 20.0, 60.0, [("growth", 100.0)]);
        identical.id = Some(config.id.clone());
        save_portfolio_alert_config(&db, identical).unwrap();
        let after: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT updated_at FROM portfolio_alert_configs WHERE id = ?1",
                [&config.id],
                |row| row.get(0),
            )
            .unwrap();

        assert_ne!(before, after);
        assert!(chrono::DateTime::parse_from_rfc3339(&before).is_ok());
        assert!(chrono::DateTime::parse_from_rfc3339(&after).is_ok());
    }
}
