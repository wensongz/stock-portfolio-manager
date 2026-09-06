use crate::{
    db::Database,
    models::portfolio_alert::{
        PortfolioAlertConfig, PortfolioAlertScope, PortfolioAlertScopeKind, PortfolioAlertSnapshot,
        PortfolioAlertTarget, SavePortfolioAlertConfigInput,
    },
};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;
use uuid::Uuid;

const TOTAL_TOLERANCE: f64 = 0.01;

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
                    now,
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
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?10)",
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
            rusqlite::params![is_active as i32, Utc::now().to_rfc3339(), config_id],
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
    if !total.is_finite() || (total - 100.0).abs() > TOTAL_TOLERANCE + 1e-9 {
        return Err("target percentages must total 100 within 0.01".to_string());
    }
    Ok(())
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
}
