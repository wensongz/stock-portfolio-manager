use crate::db::Database;
use crate::models::ExchangeRates;
use chrono::NaiveDate;
use rusqlite::{Connection, OptionalExtension};

pub(crate) fn parse_rates(json: &str, label: &str) -> Result<ExchangeRates, String> {
    let rates: ExchangeRates = serde_json::from_str(json)
        .map_err(|error| format!("invalid {label} exchange rates: {error}"))?;
    if [rates.usd_cny, rates.usd_hkd, rates.cny_hkd]
        .iter()
        .any(|rate| !rate.is_finite() || *rate <= 0.0)
    {
        return Err(format!(
            "invalid {label} exchange rates: expected positive finite values"
        ));
    }
    Ok(rates)
}

pub(crate) fn record_rates_in(
    conn: &Connection,
    rates: &ExchangeRates,
    source: &str,
) -> Result<(), String> {
    let json = serde_json::to_string(rates).map_err(|error| error.to_string())?;
    parse_rates(&json, source)?;
    let observed_at: Option<String> = conn
        .query_row(
            "SELECT STRFTIME('%Y-%m-%dT%H:%M:%fZ',?1)",
            [&rates.updated_at],
            |row| row.get(0),
        )
        .map_err(|error| error.to_string())?;
    let observed_at =
        observed_at.ok_or_else(|| format!("invalid exchange rate date: {}", rates.updated_at))?;
    let rate_date = &observed_at[..10];
    conn.execute(
        "INSERT INTO historical_exchange_rates (rate_date,observed_at,source,exchange_rates,recorded_at)
         VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(observed_at,source) DO NOTHING",
        rusqlite::params![rate_date,observed_at,source,json,chrono::Utc::now().to_rfc3339()],
    ).map_err(|error| error.to_string())?;
    Ok(())
}

fn archived_rates_in(conn: &Connection, date: &str) -> Result<Option<String>, String> {
    conn.query_row(
        "SELECT exchange_rates FROM historical_exchange_rates
         WHERE rate_date <= ?1 ORDER BY rate_date DESC, observed_at DESC,
           CASE source WHEN 'observed' THEN 0 WHEN 'quarterly_snapshot' THEN 1
             WHEN 'legacy_quarterly_snapshot' THEN 2 ELSE 3 END
         LIMIT 1",
        [date],
        |row| row.get(0),
    )
    .optional()
    .map_err(|error| error.to_string())
}

pub(crate) fn load_for_snapshot(
    db: &Database,
    cutoff: NaiveDate,
    saved: Option<&str>,
) -> Result<ExchangeRates, String> {
    // A historical report's saved valuation basis wins over newer observations.
    // Empty legacy placeholders have no rates to preserve.
    if let Some(json) = saved.filter(|json| !matches!(json.trim(), "" | "{}" | "null")) {
        return parse_rates(json, "saved quarterly");
    }
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let date = cutoff.to_string();
    let archived = archived_rates_in(&conn, &date)?;
    if let Some(json) = archived {
        return parse_rates(&json, "historical");
    }
    // Compatibility for older imports containing only daily valuation records.
    // Validate the observation date too: backfilled current FX is not past FX.
    let mut stmt = conn
        .prepare(
            "SELECT exchange_rates FROM daily_portfolio_values WHERE date <= ?1 ORDER BY date DESC",
        )
        .map_err(|error| error.to_string())?;
    let legacy = stmt
        .query_map([&date], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?
        .collect::<rusqlite::Result<Vec<_>>>()
        .map_err(|error| error.to_string())?;
    let mut invalid = None;
    for json in legacy {
        let rates = match parse_rates(&json, "historical") {
            Ok(rates) => rates,
            Err(error) => {
                invalid.get_or_insert(error);
                continue;
            }
        };
        let observed: Option<String> = conn
            .query_row("SELECT DATE(?1)", [&rates.updated_at], |row| row.get(0))
            .map_err(|error| error.to_string())?;
        if observed.is_some() {
            record_rates_in(&conn, &rates, "legacy_daily_snapshot")?;
        }
    }
    if let Some(json) = archived_rates_in(&conn, &date)? {
        return parse_rates(&json, "historical");
    }
    Err(invalid
        .unwrap_or_else(|| format!("missing historical exchange rates on or before {cutoff}")))
}

pub(crate) fn record_snapshot_rates_in(
    conn: &Connection,
    _valuation_date: &str,
    rates: &ExchangeRates,
) -> Result<(), String> {
    let json = serde_json::to_string(rates).map_err(|error| error.to_string())?;
    parse_rates(&json, "quarterly")?;
    let date: Option<String> = conn
        .query_row("SELECT DATE(?1)", [&rates.updated_at], |row| row.get(0))
        .map_err(|error| error.to_string())?;
    // Preserve a legacy report's valid saved basis even if its observation date
    // was omitted. It cannot become an independently dated historical record.
    if date.is_none() {
        return Ok(());
    }
    record_rates_in(conn, rates, "quarterly_snapshot")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::exchange_rate_service::save_exchange_rates_to_db;

    fn rates(date: &str, usd_cny: f64) -> ExchangeRates {
        ExchangeRates {
            usd_cny,
            usd_hkd: 7.8,
            cny_hkd: 7.8 / usd_cny,
            updated_at: date.into(),
        }
    }

    fn cutoff() -> NaiveDate {
        NaiveDate::from_ymd_opt(2025, 3, 31).unwrap()
    }

    #[test]
    fn existing_quarter_keeps_its_rates_after_daily_cache_is_cleared() {
        let db = Database::new(":memory:").unwrap();
        save_exchange_rates_to_db(&db, &rates("2025-03-31", 8.0)).unwrap();
        let original = serde_json::to_string(&rates("2025-03-28", 7.0)).unwrap();
        let loaded = load_for_snapshot(&db, cutoff(), Some(&original)).unwrap();
        assert_eq!(loaded.usd_cny, 7.0);
        assert_eq!(loaded.updated_at, "2025-03-28");
    }

    #[test]
    fn observed_rates_survive_daily_cache_deletion_and_select_only_past_dates() {
        let db = Database::new(":memory:").unwrap();
        save_exchange_rates_to_db(&db, &rates("2025-03-28", 7.0)).unwrap();
        save_exchange_rates_to_db(&db, &rates("2025-04-01", 8.0)).unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "DELETE FROM daily_portfolio_values; DELETE FROM daily_holding_snapshots;",
            )
            .unwrap();
        let loaded = load_for_snapshot(&db, cutoff(), None).unwrap();
        assert_eq!(loaded.usd_cny, 7.0);
        assert_eq!(loaded.updated_at, "2025-03-28");
    }

    #[test]
    fn saving_a_snapshot_does_not_relabel_future_rates_as_historical() {
        let db = Database::new(":memory:").unwrap();
        record_snapshot_rates_in(
            &db.conn.lock().unwrap(),
            "2025-03-31",
            &rates("2025-04-01", 8.0),
        )
        .unwrap();
        assert!(load_for_snapshot(&db, cutoff(), None).is_err());
        let conn = db.conn.lock().unwrap();
        let (date, source): (String, String) = conn
            .query_row(
                "SELECT rate_date, source FROM historical_exchange_rates",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(date, "2025-04-01");
        assert_eq!(source, "quarterly_snapshot");
    }

    #[test]
    fn invalid_saved_quarter_rates_do_not_fall_back_to_new_rates() {
        let db = Database::new(":memory:").unwrap();
        save_exchange_rates_to_db(&db, &rates("2025-03-28", 7.0)).unwrap();
        let invalid = serde_json::to_string(&rates("2025-03-28", 0.0)).unwrap();
        assert!(load_for_snapshot(&db, cutoff(), Some(&invalid)).is_err());
    }

    #[test]
    fn legacy_daily_lookup_skips_future_backfilled_rates_for_older_valid_observations() {
        let db = Database::new(":memory:").unwrap();
        {
            let conn = db.conn.lock().unwrap();
            for (date, rates) in [
                ("2025-03-28", rates("2025-03-28", 7.0)),
                ("2025-03-31", rates("2025-06-01", 8.0)),
            ] {
                conn.execute(
                    "INSERT INTO daily_portfolio_values(date,exchange_rates) VALUES(?1,?2)",
                    rusqlite::params![date, serde_json::to_string(&rates).unwrap()],
                )
                .unwrap();
            }
        }
        assert_eq!(load_for_snapshot(&db, cutoff(), None).unwrap().usd_cny, 7.0);
    }

    #[test]
    fn latest_intraday_observation_wins_without_discarding_earlier_observations() {
        let db = Database::new(":memory:").unwrap();
        save_exchange_rates_to_db(&db, &rates("2025-03-31T15:00:00+08:00", 8.0)).unwrap();
        save_exchange_rates_to_db(&db, &rates("2025-03-31T09:00:00+08:00", 7.0)).unwrap();
        assert_eq!(load_for_snapshot(&db, cutoff(), None).unwrap().usd_cny, 8.0);
        let count: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM historical_exchange_rates WHERE source='observed'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(count, 2);
    }

    #[test]
    fn undated_legacy_quarter_keeps_its_basis_without_inventing_an_observation_date() {
        let db = Database::new(":memory:").unwrap();
        let original = rates("", 7.0);
        let json = serde_json::to_string(&original).unwrap();
        assert_eq!(
            load_for_snapshot(&db, cutoff(), Some(&json))
                .unwrap()
                .usd_cny,
            7.0
        );
        record_snapshot_rates_in(&db.conn.lock().unwrap(), "2025-03-31", &original).unwrap();
        assert!(load_for_snapshot(&db, cutoff(), None).is_err());
    }
}
