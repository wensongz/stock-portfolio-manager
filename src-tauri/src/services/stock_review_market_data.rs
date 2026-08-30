#![allow(dead_code)]

use crate::db::Database;
use crate::models::stock_review::{MetricAvailability, MetricStatus};
use crate::models::PriceCandle;
use crate::services::quote_service;
use chrono::{Duration, NaiveDate, Utc};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const STOCK_REVIEW_HORIZON_DAYS: i64 = 180;

/// A cached market observation.  The provider's current candle interface has
/// no total-return fields, so stock imports leave `adjusted_close` and
/// `dividend` as `None` until a reliable source supplies them.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DailyMarketPoint {
    pub date: NaiveDate,
    pub open: Option<f64>,
    pub high: Option<f64>,
    pub low: Option<f64>,
    pub close: f64,
    pub volume: Option<f64>,
    pub adjusted_close: Option<f64>,
    pub dividend: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MarketReturnMode {
    TotalReturn,
    PriceOnly,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MarketCalendar {
    pub sessions: Vec<NaiveDate>,
    pub complete_start: Option<NaiveDate>,
    pub complete_through: Option<NaiveDate>,
    pub availability: MetricAvailability,
}

impl MarketCalendar {
    pub fn covers(&self, start: NaiveDate, end: NaiveDate) -> bool {
        start <= end
            && self.availability.status == MetricStatus::Available
            && self.complete_start.is_some_and(|date| date <= start)
            && self.complete_through.is_some_and(|date| date >= end)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketDataCoverage {
    pub required_sessions: usize,
    pub present_sessions: usize,
    pub coverage_ratio: Option<f64>,
    pub availability: MetricAvailability,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MarketSeries {
    pub points: Vec<DailyMarketPoint>,
    pub coverage: MarketDataCoverage,
    pub return_mode: MarketReturnMode,
    pub return_mode_availability: MetricAvailability,
}

impl MarketSeries {
    pub fn from_points(points: Vec<DailyMarketPoint>, required_sessions: usize) -> Self {
        let coverage = classify_coverage(required_sessions, points.len());
        let (return_mode, return_mode_availability) = classify_return_mode(&points);
        Self {
            points,
            coverage,
            return_mode,
            return_mode_availability,
        }
    }
}

/// Classify point coverage against a caller-provided expected market-session
/// count.  Callers must not pass calendar-day counts here.
pub fn classify_coverage(required: usize, present: usize) -> MarketDataCoverage {
    if required == 0 {
        return MarketDataCoverage {
            required_sessions: required,
            present_sessions: present,
            coverage_ratio: None,
            availability: unavailable("No expected market sessions were supplied."),
        };
    }

    let ratio = present as f64 / required as f64;
    let availability = if ratio >= 0.95 {
        available()
    } else if ratio >= 0.80 {
        degraded("Market-session coverage is below 95%.")
    } else {
        unavailable("Market-session coverage is below 80%.")
    };
    MarketDataCoverage {
        required_sessions: required,
        present_sessions: present,
        coverage_ratio: Some(ratio),
        availability,
    }
}

/// Return total-return mode only when every relevant point exposes a reliable
/// adjusted close, or every point has an explicit dividend value (zero is a
/// valid explicit no-dividend observation).
pub fn classify_return_mode(points: &[DailyMarketPoint]) -> (MarketReturnMode, MetricAvailability) {
    let has_complete_adjusted_close =
        !points.is_empty() && points.iter().all(|point| point.adjusted_close.is_some());
    let has_complete_dividends =
        !points.is_empty() && points.iter().all(|point| point.dividend.is_some());

    if has_complete_adjusted_close || has_complete_dividends {
        (MarketReturnMode::TotalReturn, available())
    } else {
        (
            MarketReturnMode::PriceOnly,
            degraded(
                "Adjusted close or complete explicit dividend data is unavailable; using price-only returns.",
            ),
        )
    }
}

/// Store price candles idempotently.  `PriceCandle` does not carry adjusted
/// prices or dividends, so those values are intentionally stored as NULL.
pub fn upsert_stock_candles(
    db: &Database,
    symbol: &str,
    market: &str,
    source: &str,
    candles: &[PriceCandle],
) -> Result<(), String> {
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    let updated_at = Utc::now().to_rfc3339();

    for candle in candles {
        let date = NaiveDate::parse_from_str(&candle.date, "%Y-%m-%d")
            .map_err(|error| format!("Invalid stock candle date '{}': {}", candle.date, error))?;
        transaction
            .execute(
                "INSERT INTO stock_daily_prices
                    (symbol, market, date, open, high, low, close, volume, adjusted_close, dividend, source, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, NULL, ?9, ?10)
                 ON CONFLICT(symbol, market, date) DO UPDATE SET
                    open = excluded.open,
                    high = excluded.high,
                    low = excluded.low,
                    close = excluded.close,
                    volume = excluded.volume,
                    adjusted_close = excluded.adjusted_close,
                    dividend = excluded.dividend,
                    source = excluded.source,
                    updated_at = excluded.updated_at",
                params![
                    symbol,
                    market,
                    date.format("%Y-%m-%d").to_string(),
                    candle.open,
                    candle.high,
                    candle.low,
                    candle.close,
                    candle.volume,
                    source,
                    updated_at,
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

/// Store authoritative daily closes without fabricating OHLCV fields.
pub(crate) fn upsert_stock_closes(
    db: &Database,
    symbol: &str,
    market: &str,
    source: &str,
    prices: &[(NaiveDate, f64)],
) -> Result<(), String> {
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    let updated_at = Utc::now().to_rfc3339();

    for (date, close) in prices {
        if !close.is_finite() || *close <= 0.0 {
            continue;
        }
        transaction
            .execute(
                "INSERT INTO stock_daily_prices
                    (symbol, market, date, open, high, low, close, volume, adjusted_close, dividend, source, updated_at)
                 VALUES (?1, ?2, ?3, NULL, NULL, NULL, ?4, NULL, NULL, NULL, ?5, ?6)
                 ON CONFLICT(symbol, market, date) DO UPDATE SET
                    open = NULL,
                    high = NULL,
                    low = NULL,
                    close = excluded.close,
                    volume = NULL,
                    adjusted_close = NULL,
                    dividend = NULL,
                    source = excluded.source,
                    updated_at = excluded.updated_at",
                params![
                    symbol,
                    market,
                    date.format("%Y-%m-%d").to_string(),
                    close,
                    source,
                    updated_at,
                ],
            )
            .map_err(|error| error.to_string())?;
    }
    transaction.commit().map_err(|error| error.to_string())
}

/// Load cached stock points ordered by their actual market-session dates.
pub fn load_stock_price_series(
    db: &Database,
    symbol: &str,
    market: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<DailyMarketPoint>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT date, open, high, low, close, volume, adjusted_close, dividend
             FROM stock_daily_prices
             WHERE symbol = ?1 AND market = ?2 AND date BETWEEN ?3 AND ?4
               AND NOT (
                   ?2 IN ('CN', 'HK')
                   AND LOWER(source) = 'xueqiu'
                   AND (open IS NOT NULL OR high IS NOT NULL OR low IS NOT NULL OR volume IS NOT NULL)
               )
             ORDER BY date ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![
                symbol,
                market,
                start.format("%Y-%m-%d").to_string(),
                end.format("%Y-%m-%d").to_string(),
            ],
            |row| {
                let date: String = row.get(0)?;
                Ok((
                    date,
                    DailyMarketPoint {
                        date: NaiveDate::from_ymd_opt(1970, 1, 1).unwrap(),
                        open: row.get(1)?,
                        high: row.get(2)?,
                        low: row.get(3)?,
                        close: row.get(4)?,
                        volume: row.get(5)?,
                        adjusted_close: row.get(6)?,
                        dividend: row.get(7)?,
                    },
                ))
            },
        )
        .map_err(|error| error.to_string())?;

    rows.map(|row| {
        let (date, mut point) = row.map_err(|error| error.to_string())?;
        point.date = NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map_err(|error| format!("Invalid cached stock price date '{}': {}", date, error))?;
        Ok(point)
    })
    .collect()
}

/// Refresh leading/trailing cache bounds and any missing authoritative market
/// sessions, then return the reloaded cache. Prices are never forward-filled.
pub async fn ensure_stock_price_cache(
    db: &Database,
    symbol: &str,
    market: &str,
    start: NaiveDate,
    end: NaiveDate,
    expected_sessions: Option<&[NaiveDate]>,
    provider: &str,
) -> Result<Vec<DailyMarketPoint>, String> {
    let mut cached = load_stock_price_series(db, symbol, market, start, end)?;
    for (gap_start, gap_end) in
        cache_fill_ranges_for_sessions(&cached, start, end, expected_sessions)
    {
        if let Ok(prices) =
            quote_service::fetch_stock_history(symbol, market, gap_start, gap_end, provider).await
        {
            upsert_stock_closes(db, symbol, market, provider, &prices)?;
        }
    }
    cached = load_stock_price_series(db, symbol, market, start, end)?;
    if let Some(expected_sessions) = expected_sessions {
        if let Some((gap_start, gap_end)) =
            session_gap_fill_range(&cached, expected_sessions, start, end)
        {
            if let Ok(prices) =
                quote_service::fetch_stock_history(symbol, market, gap_start, gap_end, provider)
                    .await
            {
                upsert_stock_closes(db, symbol, market, provider, &prices)?;
            }
        }
    }
    load_stock_price_series(db, symbol, market, start, end)
}

/// Leading and trailing cache gaps to request from a provider.  Missing dates
/// between cached market points are intentionally not treated as fetchable
/// gaps, because they can be holidays, suspensions, or delistings.
fn cache_fill_ranges(
    cached: &[DailyMarketPoint],
    start: NaiveDate,
    end: NaiveDate,
) -> Vec<(NaiveDate, NaiveDate)> {
    if start > end {
        return Vec::new();
    }
    let first = cached.first().map(|point| point.date);
    let last = cached.last().map(|point| point.date);
    match (first, last) {
        (None, None) => vec![(start, end)],
        (Some(first), Some(last)) => {
            let mut gaps = Vec::new();
            if start < first {
                gaps.push((start, first - Duration::days(1)));
            }
            if last < end {
                gaps.push((last + Duration::days(1), end));
            }
            gaps
        }
        _ => Vec::new(),
    }
}

/// Bound provider requests to known market sessions when an authoritative
/// calendar is available. A closed trailing day must not become an empty
/// provider request that triggers fallback providers. `None` preserves the
/// natural-date fallback used when calendar coverage is unavailable.
fn cache_fill_ranges_for_sessions(
    cached: &[DailyMarketPoint],
    start: NaiveDate,
    end: NaiveDate,
    expected_sessions: Option<&[NaiveDate]>,
) -> Vec<(NaiveDate, NaiveDate)> {
    let Some(expected_sessions) = expected_sessions else {
        return cache_fill_ranges(cached, start, end);
    };
    let sessions = expected_sessions
        .iter()
        .copied()
        .filter(|date| *date >= start && *date <= end)
        .collect::<BTreeSet<_>>();
    let Some(first_session) = sessions.first().copied() else {
        return Vec::new();
    };
    let last_session = sessions.last().copied().unwrap_or(first_session);
    let authoritative_cached = cached
        .iter()
        .filter(|point| sessions.contains(&point.date))
        .cloned()
        .collect::<Vec<_>>();
    cache_fill_ranges(&authoritative_cached, first_session, last_session)
}

/// Return one bounded provider refresh covering every missing authoritative
/// market session. Provider rows on weekends or holidays do not satisfy a
/// missing exchange session.
fn session_gap_fill_range(
    cached: &[DailyMarketPoint],
    expected_sessions: &[NaiveDate],
    start: NaiveDate,
    end: NaiveDate,
) -> Option<(NaiveDate, NaiveDate)> {
    let present = cached
        .iter()
        .map(|point| point.date)
        .collect::<BTreeSet<_>>();
    let first_provider_observation = expected_sessions
        .iter()
        .copied()
        .find(|date| *date >= start && *date <= end && present.contains(date));
    let refresh_start = first_provider_observation.unwrap_or(start).max(start);
    let mut missing = expected_sessions
        .iter()
        .copied()
        // The leading range has already been requested by the bounds fill.
        // If a provider's first observation is later (for example an IPO),
        // only holes at or after that observation are retryable.
        .filter(|date| *date >= refresh_start && *date <= end)
        .filter(|date| !present.contains(date));
    let first = missing.next()?;
    let last = missing.next_back().unwrap_or(first);
    Some((first, last))
}

/// A complete authoritative calendar is the date authority for review
/// prices. Provider observations stamped on closed dates are ignored rather
/// than being allowed to distort shadow curves or satisfy coverage.
pub fn filter_price_points_to_authoritative_sessions(
    points: Vec<DailyMarketPoint>,
    calendar: &MarketCalendar,
) -> Vec<DailyMarketPoint> {
    if calendar.availability.status != MetricStatus::Available {
        return points;
    }
    let sessions = calendar.sessions.iter().copied().collect::<BTreeSet<_>>();
    points
        .into_iter()
        .filter(|point| sessions.contains(&point.date))
        .collect()
}

/// Read existing benchmark cache only.  Benchmark fetching stays with the
/// orchestration layer so this reader remains deterministic and testable.
pub fn load_benchmark_series(
    db: &Database,
    symbol: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<DailyMarketPoint>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let mut statement = conn
        .prepare(
            "SELECT date, close_price FROM benchmark_daily_prices
             WHERE symbol = ?1 AND date BETWEEN ?2 AND ?3
             ORDER BY date ASC",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(
            params![
                symbol,
                start.format("%Y-%m-%d").to_string(),
                end.format("%Y-%m-%d").to_string(),
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, f64>(1)?)),
        )
        .map_err(|error| error.to_string())?;

    rows.map(|row| {
        let (date, close) = row.map_err(|error| error.to_string())?;
        Ok(DailyMarketPoint {
            date: NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .map_err(|error| format!("Invalid cached benchmark date '{}': {}", date, error))?,
            open: None,
            high: None,
            low: None,
            close,
            volume: None,
            adjusted_close: None,
            dividend: None,
        })
    })
    .collect()
}

/// Load exchange sessions from the explicit calendar cache. Quote rows are
/// deliberately not accepted as calendar authority.
pub fn load_market_sessions(
    db: &Database,
    market: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<MarketCalendar, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let coverage = conn
        .query_row(
            "SELECT complete_start, complete_through, encodes_closed_dates
         FROM stock_market_calendar_coverage WHERE market = ?1",
            params![market],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .ok();
    let Some((complete_start, complete_through, encodes_closed_dates)) = coverage else {
        return Ok(MarketCalendar {
            sessions: Vec::new(),
            complete_start: None,
            complete_through: None,
            availability: MetricAvailability {
                status: MetricStatus::Unavailable,
                note: Some(
                    "Explicit market-calendar coverage metadata is unavailable.".to_string(),
                ),
            },
        });
    };
    let complete_start = NaiveDate::parse_from_str(&complete_start, "%Y-%m-%d")
        .map_err(|error| format!("Invalid calendar coverage start: {error}"))?;
    let complete_through = NaiveDate::parse_from_str(&complete_through, "%Y-%m-%d")
        .map_err(|error| format!("Invalid calendar coverage end: {error}"))?;
    if encodes_closed_dates != 1 || complete_start > complete_through {
        return Ok(MarketCalendar {
            sessions: Vec::new(),
            complete_start: Some(complete_start),
            complete_through: Some(complete_through),
            availability: MetricAvailability {
                status: MetricStatus::Unavailable,
                note: Some(
                    "Calendar coverage does not explicitly encode both open and closed dates."
                        .to_string(),
                ),
            },
        });
    }
    let range_start = start.max(complete_start);
    let range_end = end.min(complete_through);
    let mut statement = conn
        .prepare(
            "SELECT date, is_session FROM stock_market_sessions
             WHERE market = ?1 AND date BETWEEN ?2 AND ?3
             ORDER BY date ASC",
        )
        .map_err(|error| error.to_string())?;
    let sessions = statement
        .query_map(
            params![
                market,
                range_start.format("%Y-%m-%d").to_string(),
                range_end.format("%Y-%m-%d").to_string()
            ],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
        )
        .map_err(|error| error.to_string())?
        .map(|row| {
            let (date, is_session) = row.map_err(|error| error.to_string())?;
            NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .map(|date| (date, is_session))
                .map_err(|error| format!("Invalid cached market-session date '{date}': {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let expected_days = if range_start <= range_end {
        (range_end - range_start).num_days() as usize + 1
    } else {
        0
    };
    let complete_rows = sessions.len() == expected_days
        && sessions.iter().enumerate().all(|(offset, (date, flag))| {
            *date == range_start + Duration::days(offset as i64) && matches!(*flag, 0 | 1)
        });
    if !complete_rows {
        return Ok(MarketCalendar {
            sessions: Vec::new(),
            complete_start: Some(complete_start),
            complete_through: Some(complete_through),
            availability: MetricAvailability {
                status: MetricStatus::Unavailable,
                note: Some(
                    "Calendar coverage metadata claims a range with missing day rows.".to_string(),
                ),
            },
        });
    }
    Ok(MarketCalendar {
        sessions: sessions
            .into_iter()
            .filter_map(|(date, is_session)| (is_session == 1).then_some(date))
            .collect(),
        complete_start: Some(complete_start),
        complete_through: Some(complete_through),
        availability: MetricAvailability {
            status: MetricStatus::Available,
            note: None,
        },
    })
}

pub fn default_benchmark_symbol(market: &str) -> Option<&'static str> {
    match market {
        "US" => Some("^GSPC"),
        "CN" => Some("000300.SS"),
        "HK" => Some("^HSI"),
        _ => None,
    }
}

/// The maximum market-data end date needed for an action's 120-session review
/// window.  Callers should use this for both active and already-closed
/// positions before asking the cache to fill.
pub fn evaluation_cache_end(action_date: NaiveDate, today: NaiveDate) -> NaiveDate {
    std::cmp::min(
        action_date + Duration::days(STOCK_REVIEW_HORIZON_DAYS),
        today,
    )
}

/// Return the Nth expected market-session date strictly after the action date.
/// The selection uses the market or benchmark calendar, not observed stock
/// candles, so a halt cannot silently move an endpoint to a later quote.
pub fn nth_market_session_after(
    session_dates: &[NaiveDate],
    action_date: NaiveDate,
    session_number: usize,
) -> Option<NaiveDate> {
    if session_number == 0 {
        return None;
    }
    let mut ordered_dates = session_dates.to_vec();
    ordered_dates.sort_unstable();
    ordered_dates.dedup();
    ordered_dates
        .into_iter()
        .filter(|date| *date > action_date)
        .nth(session_number - 1)
}

/// Resolve a stock quote only when it exists on the chosen market-session
/// date.  There is deliberately no next-day fallback or forward fill.
pub fn market_point_on_session(
    points: &[DailyMarketPoint],
    session_date: NaiveDate,
) -> Option<&DailyMarketPoint> {
    points.iter().find(|point| point.date == session_date)
}

fn available() -> MetricAvailability {
    MetricAvailability {
        status: MetricStatus::Available,
        note: None,
    }
}

fn degraded(note: &str) -> MetricAvailability {
    MetricAvailability {
        status: MetricStatus::Degraded,
        note: Some(note.to_string()),
    }
}

fn unavailable(note: &str) -> MetricAvailability {
    MetricAvailability {
        status: MetricStatus::Unavailable,
        note: Some(note.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        cache_fill_ranges, cache_fill_ranges_for_sessions, classify_coverage, classify_return_mode,
        default_benchmark_symbol, evaluation_cache_end,
        filter_price_points_to_authoritative_sessions, load_benchmark_series,
        load_stock_price_series, market_point_on_session, nth_market_session_after,
        session_gap_fill_range, upsert_stock_candles, upsert_stock_closes, DailyMarketPoint,
        MarketCalendar, MarketReturnMode,
    };
    use crate::db::Database;
    use crate::models::stock_review::MetricStatus;
    use crate::models::PriceCandle;
    use chrono::{Datelike, Duration, NaiveDate};

    fn date(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    fn candle(day: &str, close: f64) -> PriceCandle {
        PriceCandle {
            date: day.to_string(),
            open: close - 1.0,
            high: close + 1.0,
            low: close - 2.0,
            close,
            volume: 100.0,
        }
    }

    fn point(day: NaiveDate) -> DailyMarketPoint {
        DailyMarketPoint {
            date: day,
            open: Some(99.0),
            high: Some(101.0),
            low: Some(98.0),
            close: 100.0,
            volume: Some(100.0),
            adjusted_close: None,
            dividend: None,
        }
    }

    #[test]
    fn market_calendar_never_covers_an_inverted_interval() {
        let calendar = MarketCalendar {
            sessions: vec![date("2026-08-27")],
            complete_start: Some(date("2026-08-01")),
            complete_through: Some(date("2026-08-27")),
            availability: super::available(),
        };

        assert!(!calendar.covers(date("2026-08-28"), date("2026-08-27")));
    }

    #[test]
    fn upsert_stock_candles_updates_same_primary_key_without_duplicates() {
        // Removing the conflict update would leave the old close or duplicate a day.
        let db = Database::new(":memory:").unwrap();
        let initial = vec![
            candle("2024-01-02", 100.0),
            candle("2024-01-03", 101.0),
            candle("2024-01-04", 102.0),
        ];
        upsert_stock_candles(&db, "AAPL", "US", "fixture", &initial).unwrap();
        upsert_stock_candles(
            &db,
            "AAPL",
            "US",
            "second-source",
            &[candle("2024-01-03", 111.0)],
        )
        .unwrap();

        let prices =
            load_stock_price_series(&db, "AAPL", "US", date("2024-01-01"), date("2024-01-05"))
                .unwrap();
        assert_eq!(prices.len(), 3);
        assert_eq!(prices[1].date, date("2024-01-03"));
        assert_eq!(prices[1].close, 111.0);
        assert_eq!(prices[1].adjusted_close, None);
        assert_eq!(prices[1].dividend, None);
        let source: String = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT source FROM stock_daily_prices WHERE symbol = ?1 AND market = ?2 AND date = ?3",
                rusqlite::params!["AAPL", "US", "2024-01-03"],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(source, "second-source");
    }

    #[test]
    fn cn_review_ignores_legacy_xueqiu_candles_stamped_with_utc_dates() {
        let db = Database::new(":memory:").unwrap();
        upsert_stock_candles(
            &db,
            "sz001248",
            "CN",
            "xueqiu",
            &[candle("2026-07-01", 23.95)],
        )
        .unwrap();
        upsert_stock_closes(
            &db,
            "sz001248",
            "CN",
            "xueqiu",
            &[(date("2026-07-02"), 23.95)],
        )
        .unwrap();

        let prices = load_stock_price_series(
            &db,
            "sz001248",
            "CN",
            date("2026-07-01"),
            date("2026-07-02"),
        )
        .unwrap();

        assert_eq!(
            prices.iter().map(|point| point.date).collect::<Vec<_>>(),
            vec![date("2026-07-02")]
        );
    }

    #[test]
    fn refreshed_xueqiu_close_clears_legacy_shifted_ohlcv_provenance() {
        let db = Database::new(":memory:").unwrap();
        upsert_stock_candles(
            &db,
            "sz001248",
            "CN",
            "xueqiu",
            &[candle("2026-07-02", 22.02)],
        )
        .unwrap();
        upsert_stock_closes(
            &db,
            "sz001248",
            "CN",
            "xueqiu",
            &[(date("2026-07-02"), 23.95)],
        )
        .unwrap();

        let prices = load_stock_price_series(
            &db,
            "sz001248",
            "CN",
            date("2026-07-02"),
            date("2026-07-02"),
        )
        .unwrap();

        assert_eq!(prices.len(), 1);
        assert_eq!(prices[0].close, 23.95);
        assert_eq!(prices[0].open, None);
        assert_eq!(prices[0].high, None);
        assert_eq!(prices[0].low, None);
        assert_eq!(prices[0].volume, None);
    }

    #[test]
    fn classifies_coverage_against_required_market_sessions() {
        // Changing a threshold or treating zero sessions as 100% must fail this table.
        assert_eq!(
            classify_coverage(100, 95).availability.status,
            MetricStatus::Available
        );
        assert_eq!(
            classify_coverage(100, 94).availability.status,
            MetricStatus::Degraded
        );
        assert_eq!(
            classify_coverage(100, 80).availability.status,
            MetricStatus::Degraded
        );
        assert_eq!(
            classify_coverage(100, 79).availability.status,
            MetricStatus::Unavailable
        );

        let zero = classify_coverage(0, 0);
        assert_eq!(zero.coverage_ratio, None);
        assert_eq!(zero.availability.status, MetricStatus::Unavailable);
    }

    #[test]
    fn market_session_lookup_counts_ordered_points_strictly_after_action_date() {
        // Counting calendar days or the action day would select a different endpoint.
        let action_date = date("2024-01-05");
        let holidays = [date("2024-01-15"), date("2024-02-19")];
        let mut points = Vec::new();
        let mut cursor = action_date;
        while points.len() < 120 {
            cursor += Duration::days(1);
            if cursor.weekday().num_days_from_monday() < 5 && !holidays.contains(&cursor) {
                points.push(point(cursor));
            }
        }

        let session_dates = points.iter().map(|item| item.date).collect::<Vec<_>>();
        let day_60 = nth_market_session_after(&session_dates, action_date, 60);
        let day_120 = nth_market_session_after(&session_dates, action_date, 120);
        assert_eq!(day_60, Some(points[59].date));
        assert_eq!(day_120, Some(points[119].date));
        assert_eq!(
            day_60.and_then(|session| market_point_on_session(&points, session)),
            Some(&points[59])
        );
        assert_eq!(
            day_120.and_then(|session| market_point_on_session(&points, session)),
            Some(&points[119])
        );
        assert_eq!(
            nth_market_session_after(&session_dates, action_date, 121),
            None
        );
    }

    #[test]
    fn missing_expected_session_quote_does_not_shift_to_a_later_stock_candle() {
        // Resolving the next observed quote would hide a halt and fabricate a forward endpoint.
        let action_date = date("2024-01-05");
        let session_dates = vec![date("2024-01-08"), date("2024-01-09"), date("2024-01-10")];
        let stock_points = vec![point(date("2024-01-08")), point(date("2024-01-11"))];

        let target = nth_market_session_after(&session_dates, action_date, 2).unwrap();
        assert_eq!(target, date("2024-01-09"));
        assert_eq!(market_point_on_session(&stock_points, target), None);
    }

    #[test]
    fn missing_total_return_fields_degrades_to_price_only() {
        // Treating an OHLC-only provider as total-return data would overstate precision.
        let points = vec![point(date("2024-01-02")), point(date("2024-01-03"))];
        let (mode, availability) = classify_return_mode(&points);

        assert_eq!(mode, MarketReturnMode::PriceOnly);
        assert_eq!(availability.status, MetricStatus::Degraded);
    }

    #[test]
    fn complete_adjusted_close_or_explicit_dividend_series_supports_total_return() {
        // Removing either complete-data path must make total-return attribution unavailable.
        let mut adjusted = vec![point(date("2024-01-02")), point(date("2024-01-03"))];
        adjusted
            .iter_mut()
            .for_each(|item| item.adjusted_close = Some(item.close));
        assert_eq!(
            classify_return_mode(&adjusted).0,
            MarketReturnMode::TotalReturn
        );

        let mut dividends = vec![point(date("2024-01-02")), point(date("2024-01-03"))];
        dividends
            .iter_mut()
            .for_each(|item| item.dividend = Some(0.0));
        assert_eq!(
            classify_return_mode(&dividends).0,
            MarketReturnMode::TotalReturn
        );
    }

    #[test]
    fn cache_fill_only_requests_leading_and_trailing_date_gaps() {
        // Fetching an interior absent date would turn a cache miss into an invented market session.
        let cached = vec![point(date("2024-01-03")), point(date("2024-01-05"))];
        assert_eq!(
            cache_fill_ranges(&cached, date("2024-01-01"), date("2024-01-08")),
            vec![
                (date("2024-01-01"), date("2024-01-02")),
                (date("2024-01-06"), date("2024-01-08")),
            ]
        );
    }

    #[test]
    fn authoritative_cache_fill_skips_a_closed_trailing_day() {
        let cached = vec![point(date("2026-08-27")), point(date("2026-08-28"))];
        let sessions = vec![date("2026-08-27"), date("2026-08-28")];

        assert_eq!(
            cache_fill_ranges_for_sessions(
                &cached,
                date("2026-08-27"),
                date("2026-08-29"),
                Some(&sessions),
            ),
            Vec::<(NaiveDate, NaiveDate)>::new()
        );
    }

    #[test]
    fn authoritative_cache_fill_requests_the_missing_last_session_only() {
        let cached = vec![point(date("2026-08-27"))];
        let sessions = vec![date("2026-08-27"), date("2026-08-28")];

        assert_eq!(
            cache_fill_ranges_for_sessions(
                &cached,
                date("2026-08-27"),
                date("2026-08-29"),
                Some(&sessions),
            ),
            vec![(date("2026-08-28"), date("2026-08-28"))]
        );
    }

    #[test]
    fn authoritative_cache_fill_skips_a_window_without_sessions() {
        assert_eq!(
            cache_fill_ranges_for_sessions(&[], date("2026-08-29"), date("2026-08-30"), Some(&[]),),
            Vec::<(NaiveDate, NaiveDate)>::new()
        );
    }

    #[test]
    fn cache_fill_keeps_natural_date_fallback_without_authoritative_calendar() {
        assert_eq!(
            cache_fill_ranges_for_sessions(&[], date("2026-08-29"), date("2026-08-30"), None,),
            vec![(date("2026-08-29"), date("2026-08-30"))]
        );
    }

    #[test]
    fn authoritative_session_gap_requests_an_interior_refresh() {
        let cached = vec![
            point(date("2026-07-02")),
            point(date("2026-07-05")),
            point(date("2026-07-06")),
        ];
        let sessions = vec![date("2026-07-02"), date("2026-07-03"), date("2026-07-06")];

        assert_eq!(
            session_gap_fill_range(&cached, &sessions, date("2026-07-02"), date("2026-07-06")),
            Some((date("2026-07-03"), date("2026-07-03")))
        );
    }

    #[test]
    fn authoritative_session_gap_does_not_retry_before_provider_first_observation() {
        // The provider was already asked for the full range and returned a
        // newly listed security beginning on July 2. Pre-listing sessions are
        // not an interior cache hole and must not trigger fallback requests.
        let cached = vec![point(date("2026-07-02")), point(date("2026-07-03"))];
        let sessions = vec![
            date("2026-06-30"),
            date("2026-07-01"),
            date("2026-07-02"),
            date("2026-07-03"),
        ];

        assert_eq!(
            session_gap_fill_range(&cached, &sessions, date("2026-06-30"), date("2026-07-03")),
            None
        );
    }

    #[test]
    fn authoritative_calendar_removes_provider_weekend_rows() {
        let calendar = MarketCalendar {
            sessions: vec![date("2026-07-02"), date("2026-07-03"), date("2026-07-06")],
            complete_start: Some(date("2026-07-02")),
            complete_through: Some(date("2026-07-06")),
            availability: super::available(),
        };
        let points = vec![
            point(date("2026-07-02")),
            point(date("2026-07-05")),
            point(date("2026-07-06")),
        ];

        assert_eq!(
            filter_price_points_to_authoritative_sessions(points, &calendar)
                .into_iter()
                .map(|point| point.date)
                .collect::<Vec<_>>(),
            vec![date("2026-07-02"), date("2026-07-06")]
        );
    }

    #[test]
    fn benchmark_reader_uses_cached_close_only_points_and_market_defaults() {
        // Fetching here or pretending benchmark OHLC/total-return fields exist would violate cache semantics.
        let db = Database::new(":memory:").unwrap();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO benchmark_daily_prices (symbol, date, close_price, change_percent) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params!["^GSPC", "2024-01-02", 4800.0, 0.0],
            )
            .unwrap();

        let points =
            load_benchmark_series(&db, "^GSPC", date("2024-01-01"), date("2024-01-03")).unwrap();
        assert_eq!(points.len(), 1);
        assert_eq!(points[0].close, 4800.0);
        assert_eq!(points[0].open, None);
        assert_eq!(classify_return_mode(&points).0, MarketReturnMode::PriceOnly);
        assert_eq!(default_benchmark_symbol("US"), Some("^GSPC"));
        assert_eq!(default_benchmark_symbol("CN"), Some("000300.SS"));
        assert_eq!(default_benchmark_symbol("HK"), Some("^HSI"));
        assert_eq!(default_benchmark_symbol("OTHER"), None);
    }

    #[test]
    fn evaluation_cache_horizon_is_180_calendar_days_capped_at_today() {
        // Using holdings or an uncapped horizon would skip closed positions or ask providers for future data.
        assert_eq!(
            evaluation_cache_end(date("2024-01-02"), date("2024-03-01")),
            date("2024-03-01")
        );
        assert_eq!(
            evaluation_cache_end(date("2024-01-02"), date("2024-12-31")),
            date("2024-06-30")
        );
    }
}
