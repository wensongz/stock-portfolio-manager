use crate::db::Database;
use crate::services::quote_service::{self, XueqiuHistoryOutcome};
use crate::services::stock_review_market_data::load_market_sessions;
use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, SecondsFormat, Utc, Weekday};
use chrono_tz::{America::New_York, Asia::Hong_Kong, Asia::Shanghai, Tz};
use rusqlite::{params, OptionalExtension, Transaction};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex as AsyncMutex;

const HOLIDAY_RESOURCE: &str = include_str!("../../resources/stock_review_market_holidays.v1.json");

#[derive(Debug, Clone, Deserialize)]
struct MarketHolidayBundle {
    revision: String,
    entries: Vec<MarketHolidayEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct MarketHolidayEntry {
    market: String,
    year: i32,
    source_urls: Vec<String>,
    notice_versions: Vec<String>,
    closed_weekdays: Vec<NaiveDate>,
    #[serde(default)]
    exceptional_closures: Vec<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketHolidayRules {
    pub market: String,
    pub resource_revision: String,
    pub complete_start: NaiveDate,
    pub complete_through: NaiveDate,
    pub weekday_closures: BTreeSet<NaiveDate>,
    pub source_urls: Vec<String>,
    pub notice_versions: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CalendarProvider {
    Xueqiu,
    EastMoney,
}

impl CalendarProvider {
    fn as_str(self) -> &'static str {
        match self {
            Self::Xueqiu => "xueqiu",
            Self::EastMoney => "eastmoney",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceIndex {
    pub logical_name: &'static str,
    pub xueqiu_symbol: &'static str,
    pub eastmoney_symbol: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexHistoryEvidence {
    pub provider: CalendarProvider,
    pub logical_index: String,
    pub request_start: NaiveDate,
    pub request_end: NaiveDate,
    pub session_dates: Vec<NaiveDate>,
    pub complete_response: bool,
}

#[derive(Debug, Clone)]
pub struct CalendarValidationRequest {
    pub market: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub rules: MarketHolidayRules,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarSyncWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarSyncStatus {
    Reused,
    Published,
    StaleCacheUsed,
    Unavailable,
}

impl CalendarSyncStatus {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Reused | Self::Published | Self::StaleCacheUsed)
    }
}

#[derive(Debug, Clone)]
pub struct CalendarSyncRequest {
    pub market: String,
    pub required_start: NaiveDate,
    pub required_through: NaiveDate,
}

#[derive(Debug, Clone)]
#[allow(dead_code)] // Read by the Task 4 report-integration consumer.
pub struct CalendarSyncOutcome {
    pub market: String,
    pub requested_start: NaiveDate,
    pub requested_through: NaiveDate,
    pub available_start: Option<NaiveDate>,
    pub available_through: Option<NaiveDate>,
    pub status: CalendarSyncStatus,
    pub issue_code: Option<String>,
    pub message: Option<String>,
    pub warnings: Vec<CalendarSyncWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedMarketCalendar {
    pub market: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub resource_revision: String,
    pub rows: Vec<(NaiveDate, bool)>,
    pub providers: Vec<String>,
    pub references: Vec<String>,
    pub warnings: Vec<CalendarSyncWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarValidationErrorKind {
    ResourceUnavailable,
    ProviderUnavailable,
    TruncatedResponse,
    IndexConflict,
    ProviderConflict,
    UnexpectedClosedDateBar,
    MissingExpectedSession,
    MissingAnchorSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarValidationError {
    pub kind: CalendarValidationErrorKind,
    pub message: String,
}

impl CalendarValidationError {
    fn new(kind: CalendarValidationErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
        }
    }
}

pub trait IndexHistorySource: Send + Sync {
    fn provider(&self) -> CalendarProvider;

    fn fetch<'a>(
        &'a self,
        market: &'a str,
        reference: &'a ReferenceIndex,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Pin<Box<dyn Future<Output = Result<IndexHistoryEvidence, String>> + Send + 'a>>;
}

#[allow(dead_code)] // Constructed by the Task 3 synchronizer.
pub struct LiveIndexHistorySource {
    provider: CalendarProvider,
}

impl IndexHistorySource for LiveIndexHistorySource {
    fn provider(&self) -> CalendarProvider {
        self.provider
    }

    fn fetch<'a>(
        &'a self,
        market: &'a str,
        reference: &'a ReferenceIndex,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Pin<Box<dyn Future<Output = Result<IndexHistoryEvidence, String>> + Send + 'a>> {
        Box::pin(async move {
            let prices = match self.provider {
                CalendarProvider::Xueqiu => {
                    match quote_service::fetch_index_history_xueqiu(
                        reference.xueqiu_symbol,
                        market,
                        start,
                        end,
                    )
                    .await?
                    {
                        XueqiuHistoryOutcome::Prices(prices) => prices,
                        XueqiuHistoryOutcome::StartsAfterRange {
                            first_available_date,
                        } => {
                            return Err(format!(
                                "Xueqiu index history starts after the requested range at {first_available_date}."
                            ));
                        }
                        XueqiuHistoryOutcome::Empty => {
                            return Err(
                                "Xueqiu returned no index sessions for the anchored request."
                                    .to_owned(),
                            );
                        }
                    }
                }
                CalendarProvider::EastMoney => {
                    quote_service::fetch_stock_history_eastmoney(
                        reference.eastmoney_symbol,
                        market,
                        start,
                        end,
                    )
                    .await?
                }
            };

            Ok(IndexHistoryEvidence {
                provider: self.provider,
                logical_index: reference.logical_name.to_owned(),
                request_start: start,
                request_end: end,
                session_dates: prices.into_iter().map(|(date, _)| date).collect(),
                complete_response: true,
            })
        })
    }
}

#[allow(dead_code)] // Public Task 3 entry point; Task 2 only establishes the adapter.
pub fn live_index_history_sources() -> Vec<Arc<dyn IndexHistorySource>> {
    vec![
        Arc::new(LiveIndexHistorySource {
            provider: CalendarProvider::Xueqiu,
        }),
        Arc::new(LiveIndexHistorySource {
            provider: CalendarProvider::EastMoney,
        }),
    ]
}

fn reference_indices(market: &str) -> Result<[ReferenceIndex; 2], String> {
    let values = match market {
        "CN" => [
            ("sse_composite", "SH000001", "^SSEC"),
            ("shenzhen_component", "SZ399001", "399001.SZ"),
        ],
        "HK" => [
            ("hang_seng", "HKHSI", "^HSI"),
            ("hang_seng_china_enterprises", "HKHSCEI", "^HSCEI"),
        ],
        "US" => [
            ("sp500", ".INX", "^GSPC"),
            ("nasdaq_composite", ".IXIC", "^IXIC"),
        ],
        _ => {
            return Err(format!("Unsupported stock-review market '{market}'."));
        }
    };
    Ok(values.map(
        |(logical_name, xueqiu_symbol, eastmoney_symbol)| ReferenceIndex {
            logical_name,
            xueqiu_symbol,
            eastmoney_symbol,
        },
    ))
}

fn is_expected_session(rules: &MarketHolidayRules, date: NaiveDate) -> bool {
    !matches!(date.weekday(), Weekday::Sat | Weekday::Sun)
        && !rules.weekday_closures.contains(&date)
}

fn dates_inclusive(start: NaiveDate, end: NaiveDate) -> Vec<NaiveDate> {
    let mut dates = Vec::new();
    let mut current = start;
    loop {
        dates.push(current);
        if current == end {
            break;
        }
        let Some(next) = current.succ_opt() else {
            break;
        };
        current = next;
    }
    dates
}

fn normalize_evidence(
    evidence: IndexHistoryEvidence,
    provider: CalendarProvider,
    reference: &ReferenceIndex,
    start: NaiveDate,
    end: NaiveDate,
    rules: &MarketHolidayRules,
) -> Result<BTreeSet<NaiveDate>, CalendarValidationError> {
    if evidence.provider != provider || evidence.logical_index != reference.logical_name {
        return Err(CalendarValidationError::new(
            CalendarValidationErrorKind::IndexConflict,
            format!(
                "{} returned evidence for the wrong provider or logical reference index.",
                provider.as_str()
            ),
        ));
    }
    if !evidence.complete_response || evidence.request_start != start || evidence.request_end != end
    {
        return Err(CalendarValidationError::new(
            CalendarValidationErrorKind::TruncatedResponse,
            format!(
                "{} returned incomplete {} evidence for {start} through {end}.",
                provider.as_str(),
                reference.logical_name
            ),
        ));
    }

    let dates: BTreeSet<_> = evidence
        .session_dates
        .into_iter()
        .filter(|date| *date >= start && *date <= end)
        .collect();
    if let Some(date) = dates
        .iter()
        .copied()
        .find(|date| !is_expected_session(rules, *date))
    {
        return Err(CalendarValidationError::new(
            CalendarValidationErrorKind::UnexpectedClosedDateBar,
            format!(
                "{} returned a regular {} bar on closed date {date}.",
                provider.as_str(),
                reference.logical_name
            ),
        ));
    }
    Ok(dates)
}

pub async fn validate_market_calendar(
    request: &CalendarValidationRequest,
    sources: &[Arc<dyn IndexHistorySource>],
) -> Result<ValidatedMarketCalendar, CalendarValidationError> {
    if request.start > request.end
        || request.rules.market != request.market
        || request.start < request.rules.complete_start
        || request.end > request.rules.complete_through
    {
        return Err(CalendarValidationError::new(
            CalendarValidationErrorKind::ResourceUnavailable,
            format!(
                "Holiday rules do not completely cover {} from {} through {}.",
                request.market, request.start, request.end
            ),
        ));
    }
    let references = reference_indices(&request.market).map_err(|message| {
        CalendarValidationError::new(CalendarValidationErrorKind::ResourceUnavailable, message)
    })?;

    let requested_dates = dates_inclusive(request.start, request.end);
    let expected_requested: BTreeSet<_> = requested_dates
        .iter()
        .copied()
        .filter(|date| is_expected_session(&request.rules, *date))
        .collect();
    let evidence_start = if expected_requested.is_empty() {
        let mut cursor = request.start.pred_opt();
        let mut anchor = None;
        while let Some(date) = cursor.filter(|date| *date >= request.rules.complete_start) {
            if is_expected_session(&request.rules, date) {
                anchor = Some(date);
                break;
            }
            cursor = date.pred_opt();
        }
        anchor.ok_or_else(|| {
            CalendarValidationError::new(
                CalendarValidationErrorKind::MissingAnchorSession,
                format!(
                    "No prior open session exists inside the complete {} holiday resource.",
                    request.market
                ),
            )
        })?
    } else {
        request.start
    };
    let expected_evidence: BTreeSet<_> = dates_inclusive(evidence_start, request.end)
        .into_iter()
        .filter(|date| is_expected_session(&request.rules, *date))
        .collect();

    let mut successful = BTreeMap::<CalendarProvider, BTreeSet<NaiveDate>>::new();
    let mut provider_errors = Vec::new();
    let mut structural_error = None;

    for source in sources {
        let provider = source.provider();
        let (left, right) = tokio::join!(
            source.fetch(&request.market, &references[0], evidence_start, request.end),
            source.fetch(&request.market, &references[1], evidence_start, request.end)
        );

        let left = left
            .map_err(|message| {
                provider_errors.push(format!(
                    "{} {}: {message}",
                    provider.as_str(),
                    references[0].logical_name
                ));
            })
            .and_then(|evidence| {
                normalize_evidence(
                    evidence,
                    provider,
                    &references[0],
                    evidence_start,
                    request.end,
                    &request.rules,
                )
                .map_err(|error| structural_error = Some(error))
            });
        let right = right
            .map_err(|message| {
                provider_errors.push(format!(
                    "{} {}: {message}",
                    provider.as_str(),
                    references[1].logical_name
                ));
            })
            .and_then(|evidence| {
                normalize_evidence(
                    evidence,
                    provider,
                    &references[1],
                    evidence_start,
                    request.end,
                    &request.rules,
                )
                .map_err(|error| structural_error = Some(error))
            });

        if let Some(error) = structural_error.take() {
            return Err(error);
        }
        if let (Ok(left), Ok(right)) = (left, right) {
            if left != right {
                return Err(CalendarValidationError::new(
                    CalendarValidationErrorKind::IndexConflict,
                    format!(
                        "{} reference indices returned different session sets.",
                        provider.as_str()
                    ),
                ));
            }
            if let Some(existing) = successful.insert(provider, left.clone()) {
                if existing != left {
                    return Err(CalendarValidationError::new(
                        CalendarValidationErrorKind::ProviderConflict,
                        format!(
                            "Duplicate {} sources returned different session sets.",
                            provider.as_str()
                        ),
                    ));
                }
            }
        }
    }

    if successful.is_empty() {
        let detail = if provider_errors.is_empty() {
            "no calendar evidence sources were configured".to_owned()
        } else {
            provider_errors.join("; ")
        };
        return Err(CalendarValidationError::new(
            CalendarValidationErrorKind::ProviderUnavailable,
            format!("No provider returned complete index evidence: {detail}."),
        ));
    }

    let mut sets = successful.values();
    let first = sets.next().expect("successful providers are non-empty");
    if sets.any(|dates| dates != first) {
        return Err(CalendarValidationError::new(
            CalendarValidationErrorKind::ProviderConflict,
            "Calendar providers returned different session sets.",
        ));
    }
    if first != &expected_evidence {
        let missing: Vec<_> = expected_evidence.difference(first).copied().collect();
        return Err(CalendarValidationError::new(
            CalendarValidationErrorKind::MissingExpectedSession,
            format!(
                "Index evidence is missing expected {} sessions: {}.",
                request.market,
                missing
                    .iter()
                    .map(|date| date.to_string())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        ));
    }

    let mut providers: Vec<_> = successful
        .keys()
        .map(|provider| provider.as_str().to_owned())
        .collect();
    providers.sort();
    providers.dedup();
    let mut references: Vec<_> = references
        .iter()
        .map(|reference| reference.logical_name.to_owned())
        .collect();
    references.sort();
    references.dedup();
    let warnings = provider_warnings(&request.market, &providers);
    let rows = requested_dates
        .into_iter()
        .map(|date| (date, expected_requested.contains(&date)))
        .collect();

    Ok(ValidatedMarketCalendar {
        market: request.market.clone(),
        start: request.start,
        end: request.end,
        resource_revision: request.rules.resource_revision.clone(),
        rows,
        providers,
        references,
        warnings,
    })
}

struct StableCalendarDigest(u64);

impl StableCalendarDigest {
    fn new(domain: &str) -> Self {
        let mut digest = Self(0xcbf29ce484222325);
        digest.write(domain);
        digest
    }

    fn write(&mut self, value: &str) {
        for byte in (value.len() as u64)
            .to_le_bytes()
            .into_iter()
            .chain(value.as_bytes().iter().copied())
        {
            self.0 ^= u64::from(byte);
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }

    fn finish(self) -> u64 {
        self.0
    }
}

pub fn stable_calendar_revision(calendar: &ValidatedMarketCalendar) -> String {
    let mut digest = StableCalendarDigest::new("stock-review-calendar-v1");
    digest.write(&calendar.resource_revision);
    digest.write(&calendar.market);
    let mut providers = calendar.providers.clone();
    providers.sort();
    providers.dedup();
    for provider in providers {
        digest.write(&provider);
    }
    let mut references = calendar.references.clone();
    references.sort();
    references.dedup();
    for reference in references {
        digest.write(&reference);
    }
    let mut rows = calendar.rows.clone();
    rows.sort();
    rows.dedup();
    for (date, is_session) in rows {
        digest.write(&date.format("%Y-%m-%d").to_string());
        digest.write(if is_session { "1" } else { "0" });
    }
    format!("{}:{:016x}", calendar.resource_revision, digest.finish())
}

fn market_clock(market: &str) -> Result<(Tz, NaiveTime), String> {
    match market {
        "CN" => Ok((Shanghai, NaiveTime::from_hms_opt(15, 30, 0).unwrap())),
        "HK" => Ok((Hong_Kong, NaiveTime::from_hms_opt(16, 30, 0).unwrap())),
        "US" => Ok((New_York, NaiveTime::from_hms_opt(16, 30, 0).unwrap())),
        _ => Err(format!("Unsupported stock-review market '{market}'.")),
    }
}

pub fn latest_fully_closed_date(market: &str, now: DateTime<Utc>) -> Result<NaiveDate, String> {
    let (timezone, cutoff) = market_clock(market)?;
    let local = now.with_timezone(&timezone);
    Ok(if local.time() >= cutoff {
        local.date_naive()
    } else {
        local
            .date_naive()
            .pred_opt()
            .ok_or_else(|| "Market-local date underflow".to_owned())?
    })
}

fn parse_market_holiday_bundle(source: &str) -> Result<MarketHolidayBundle, String> {
    let bundle: MarketHolidayBundle = serde_json::from_str(source)
        .map_err(|error| format!("Invalid holiday resource JSON: {error}"))?;
    let mut years_by_market = BTreeMap::<String, BTreeSet<i32>>::new();

    for entry in &bundle.entries {
        market_clock(&entry.market)?;
        if entry.source_urls.is_empty() || entry.source_urls.iter().any(|url| url.trim().is_empty())
        {
            return Err(format!(
                "Holiday entry {} {} has an empty source URL list.",
                entry.market, entry.year
            ));
        }

        let years = years_by_market.entry(entry.market.clone()).or_default();
        if !years.insert(entry.year) {
            return Err(format!(
                "Holiday resource has duplicate {} entry for {}.",
                entry.market, entry.year
            ));
        }

        let mut dates = BTreeSet::new();
        for date in entry
            .closed_weekdays
            .iter()
            .chain(&entry.exceptional_closures)
        {
            if !dates.insert(*date) {
                return Err(format!(
                    "Holiday entry {} {} has duplicate date {date}.",
                    entry.market, entry.year
                ));
            }
            if date.year() != entry.year {
                return Err(format!(
                    "Holiday entry {} {} has a date in the wrong year: {date}.",
                    entry.market, entry.year
                ));
            }
            if matches!(date.weekday(), Weekday::Sat | Weekday::Sun) {
                return Err(format!(
                    "Holiday entry {} {} includes weekend date {date}.",
                    entry.market, entry.year
                ));
            }
        }
    }

    for (market, years) in years_by_market {
        let mut iter = years.into_iter();
        let Some(mut previous) = iter.next() else {
            continue;
        };
        for year in iter {
            if year != previous + 1 {
                return Err(format!(
                    "Holiday resource years for {market} are not consecutive."
                ));
            }
            previous = year;
        }
    }

    Ok(bundle)
}

pub fn load_market_holiday_rules(
    market: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<MarketHolidayRules, String> {
    if start > end {
        return Err("Holiday request start must not be after end.".to_owned());
    }
    market_clock(market)?;
    let bundle = parse_market_holiday_bundle(HOLIDAY_RESOURCE)?;
    let entries: Vec<_> = bundle
        .entries
        .iter()
        .filter(|entry| entry.market == market)
        .collect();
    if entries.is_empty() {
        return Err(format!(
            "Holiday resource has no entries for market '{market}'."
        ));
    }

    let first_year = entries.iter().map(|entry| entry.year).min().unwrap();
    let last_year = entries.iter().map(|entry| entry.year).max().unwrap();
    let complete_start = NaiveDate::from_ymd_opt(first_year, 1, 1).unwrap();
    let complete_through = NaiveDate::from_ymd_opt(last_year, 12, 31).unwrap();
    if start < complete_start || end > complete_through {
        return Err(format!(
            "Holiday request {start} through {end} is outside the complete {market} resource range {complete_start} through {complete_through}."
        ));
    }

    let mut weekday_closures = BTreeSet::new();
    let mut source_urls = Vec::new();
    let mut notice_versions = Vec::new();
    for entry in entries {
        weekday_closures.extend(
            entry
                .closed_weekdays
                .iter()
                .chain(&entry.exceptional_closures)
                .copied(),
        );
        source_urls.extend(entry.source_urls.iter().cloned());
        notice_versions.extend(entry.notice_versions.iter().cloned());
    }

    Ok(MarketHolidayRules {
        market: market.to_owned(),
        resource_revision: bundle.revision,
        complete_start,
        complete_through,
        weekday_closures,
        source_urls,
        notice_versions,
    })
}

fn market_holiday_resource_bounds(market: &str) -> Result<(NaiveDate, NaiveDate), String> {
    market_clock(market)?;
    let bundle = parse_market_holiday_bundle(HOLIDAY_RESOURCE)?;
    let years = bundle
        .entries
        .iter()
        .filter(|entry| entry.market == market)
        .map(|entry| entry.year)
        .collect::<Vec<_>>();
    let first_year = years
        .iter()
        .min()
        .copied()
        .ok_or_else(|| format!("Holiday resource has no entries for market '{market}'."))?;
    let last_year = years
        .iter()
        .max()
        .copied()
        .ok_or_else(|| format!("Holiday resource has no entries for market '{market}'."))?;
    Ok((
        NaiveDate::from_ymd_opt(first_year, 1, 1).unwrap(),
        NaiveDate::from_ymd_opt(last_year, 12, 31).unwrap(),
    ))
}

const COVERAGE_REVISION_MISMATCH: &str = "calendar coverage revision changed";

#[derive(Debug, Clone)]
struct CalendarCoverageSnapshot {
    market: String,
    source: String,
    complete_start: NaiveDate,
    complete_through: NaiveDate,
    revision: String,
    rows: Vec<(NaiveDate, bool)>,
    structural_valid: bool,
    valid: bool,
}

fn market_sync_locks() -> &'static BTreeMap<&'static str, AsyncMutex<()>> {
    static LOCKS: OnceLock<BTreeMap<&'static str, AsyncMutex<()>>> = OnceLock::new();
    LOCKS.get_or_init(|| {
        BTreeMap::from([
            ("CN", AsyncMutex::new(())),
            ("HK", AsyncMutex::new(())),
            ("US", AsyncMutex::new(())),
        ])
    })
}

fn date_text(date: NaiveDate) -> String {
    date.format("%Y-%m-%d").to_string()
}

fn rows_are_complete(rows: &[(NaiveDate, bool)], start: NaiveDate, end: NaiveDate) -> bool {
    start <= end
        && rows.len() == (end - start).num_days() as usize + 1
        && rows
            .iter()
            .enumerate()
            .all(|(offset, (date, _))| *date == start + chrono::Duration::days(offset as i64))
}

fn read_calendar_coverage(
    db: &Database,
    market: &str,
) -> Result<Option<CalendarCoverageSnapshot>, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let coverage = conn
        .query_row(
            "SELECT source, complete_start, complete_through, revision, encodes_closed_dates
             FROM stock_market_calendar_coverage WHERE market = ?1",
            [market],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                ))
            },
        )
        .optional()
        .map_err(|error| error.to_string())?;
    let Some((source, complete_start, complete_through, revision, closed_flag)) = coverage else {
        return Ok(None);
    };
    let parsed_start = NaiveDate::parse_from_str(&complete_start, "%Y-%m-%d");
    let parsed_through = NaiveDate::parse_from_str(&complete_through, "%Y-%m-%d");
    let (complete_start, complete_through) = match (parsed_start, parsed_through) {
        (Ok(start), Ok(through)) => (start, through),
        _ => {
            return Ok(Some(CalendarCoverageSnapshot {
                market: market.to_owned(),
                source,
                complete_start: NaiveDate::MIN,
                complete_through: NaiveDate::MIN,
                revision,
                rows: Vec::new(),
                structural_valid: false,
                valid: false,
            }));
        }
    };

    let mut statement = conn
        .prepare(
            "SELECT date, is_session, source FROM stock_market_sessions
             WHERE market = ?1 AND date BETWEEN ?2 AND ?3 ORDER BY date",
        )
        .map_err(|error| error.to_string())?;
    let stored_rows = statement
        .query_map(
            params![
                market,
                date_text(complete_start),
                date_text(complete_through)
            ],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                ))
            },
        )
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    let flags_valid = stored_rows
        .iter()
        .all(|(_, is_session, _)| matches!(*is_session, 0 | 1));
    let row_sources_valid = stored_rows
        .iter()
        .all(|(_, _, row_source)| row_source == &source);
    let rows = stored_rows
        .into_iter()
        .map(|(date, is_session, _)| {
            NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .map(|date| (date, is_session == 1))
                .map_err(|error| format!("Invalid cached market-session date '{date}': {error}"))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let total_rows = conn
        .query_row(
            "SELECT COUNT(*) FROM stock_market_sessions WHERE market = ?1",
            [market],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;
    let providers = providers_from_source(&source);
    let source_valid = !providers.is_empty() && canonical_calendar_source(&providers) == source;
    let resource_revision = revision
        .rsplit_once(':')
        .map(|(resource_revision, _)| resource_revision.to_owned());
    let revision_valid = resource_revision.is_some_and(|resource_revision| {
        reference_indices(market).is_ok_and(|references| {
            let reconstructed = ValidatedMarketCalendar {
                market: market.to_owned(),
                start: complete_start,
                end: complete_through,
                resource_revision,
                rows: rows.clone(),
                providers: providers.clone(),
                references: references
                    .into_iter()
                    .map(|reference| reference.logical_name.to_owned())
                    .collect(),
                warnings: Vec::new(),
            };
            stable_calendar_revision(&reconstructed) == revision
        })
    });
    let structural_valid = closed_flag == 1
        && flags_valid
        && row_sources_valid
        && source_valid
        && rows_are_complete(&rows, complete_start, complete_through)
        && total_rows == rows.len() as i64;
    let valid = structural_valid && revision_valid;
    Ok(Some(CalendarCoverageSnapshot {
        market: market.to_owned(),
        source,
        complete_start,
        complete_through,
        revision,
        rows,
        structural_valid,
        valid,
    }))
}

fn same_calendar_content(
    left: &CalendarCoverageSnapshot,
    right: &CalendarCoverageSnapshot,
) -> bool {
    left.structural_valid
        && right.structural_valid
        && left.market == right.market
        && left.source == right.source
        && left.complete_start == right.complete_start
        && left.complete_through == right.complete_through
        && left.rows == right.rows
}

fn revision_uses_resource(revision: &str, resource_revision: &str) -> bool {
    revision
        .strip_prefix(resource_revision)
        .is_some_and(|suffix| suffix.starts_with(':'))
}

fn canonical_calendar_source(providers: &[String]) -> String {
    let mut providers = providers.to_vec();
    providers.sort();
    providers.dedup();
    if providers.is_empty() {
        "validated_index_history".to_owned()
    } else {
        providers.join("+")
    }
}

fn providers_from_source(source: &str) -> Vec<String> {
    source
        .split('+')
        .filter(|provider| matches!(*provider, "eastmoney" | "xueqiu"))
        .map(str::to_owned)
        .collect()
}

fn provider_warnings(market: &str, providers: &[String]) -> Vec<CalendarSyncWarning> {
    if providers.len() == 1 {
        vec![CalendarSyncWarning {
            code: "market_calendar_single_provider".to_owned(),
            message: format!(
                "{market} market calendar was validated with only the {} provider.",
                providers[0]
            ),
        }]
    } else {
        Vec::new()
    }
}

fn coverage_provider_warnings(coverage: &CalendarCoverageSnapshot) -> Vec<CalendarSyncWarning> {
    provider_warnings(&coverage.market, &providers_from_source(&coverage.source))
}

fn validation_failure_outcome(
    request: &CalendarSyncRequest,
    old_coverage: Option<&CalendarCoverageSnapshot>,
    message: String,
) -> CalendarSyncOutcome {
    let old_coverage = old_coverage.filter(|coverage| coverage.valid);
    let warnings = old_coverage
        .map(coverage_provider_warnings)
        .unwrap_or_default();
    CalendarSyncOutcome {
        market: request.market.clone(),
        requested_start: request.required_start,
        requested_through: request.required_through,
        available_start: old_coverage.map(|coverage| coverage.complete_start),
        available_through: old_coverage.map(|coverage| coverage.complete_through),
        status: if old_coverage.is_some() {
            CalendarSyncStatus::StaleCacheUsed
        } else {
            CalendarSyncStatus::Unavailable
        },
        issue_code: Some(
            if old_coverage.is_some() {
                "market_calendar_refresh_failed"
            } else {
                "market_calendar_sync_failed"
            }
            .to_owned(),
        ),
        message: Some(message),
        warnings,
    }
}

fn successful_outcome(
    request: &CalendarSyncRequest,
    status: CalendarSyncStatus,
    available_start: NaiveDate,
    available_through: NaiveDate,
    warnings: Vec<CalendarSyncWarning>,
) -> CalendarSyncOutcome {
    CalendarSyncOutcome {
        market: request.market.clone(),
        requested_start: request.required_start,
        requested_through: request.required_through,
        available_start: Some(available_start),
        available_through: Some(available_through),
        status,
        issue_code: None,
        message: None,
        warnings,
    }
}

fn transaction_calendar_rows(
    transaction: &Transaction<'_>,
    market: &str,
    start: NaiveDate,
    end: NaiveDate,
) -> Result<Vec<(NaiveDate, bool)>, String> {
    let mut statement = transaction
        .prepare(
            "SELECT date, is_session FROM stock_market_sessions
             WHERE market = ?1 AND date BETWEEN ?2 AND ?3 ORDER BY date",
        )
        .map_err(|error| error.to_string())?;
    let stored = statement
        .query_map(params![market, date_text(start), date_text(end)], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;
    stored
        .into_iter()
        .map(|(date, is_session)| {
            if !matches!(is_session, 0 | 1) {
                return Err(format!(
                    "Calendar row {market} {date} has invalid session flag {is_session}."
                ));
            }
            NaiveDate::parse_from_str(&date, "%Y-%m-%d")
                .map(|date| (date, is_session == 1))
                .map_err(|error| format!("Invalid cached market-session date '{date}': {error}"))
        })
        .collect()
}

fn publish_validated_segments(
    db: &Database,
    segments: &[ValidatedMarketCalendar],
    final_calendar: &ValidatedMarketCalendar,
    expected_coverage_revision: Option<&str>,
) -> Result<CalendarSyncStatus, String> {
    if !rows_are_complete(
        &final_calendar.rows,
        final_calendar.start,
        final_calendar.end,
    ) {
        return Err(
            "Final market calendar does not contain every natural day exactly once.".to_owned(),
        );
    }
    for segment in segments {
        if segment.market != final_calendar.market
            || segment.resource_revision != final_calendar.resource_revision
            || segment.start < final_calendar.start
            || segment.end > final_calendar.end
            || !rows_are_complete(&segment.rows, segment.start, segment.end)
        {
            return Err("Validated calendar segment is incomplete or inconsistent.".to_owned());
        }
    }

    let revision = stable_calendar_revision(final_calendar);
    let source = canonical_calendar_source(&final_calendar.providers);
    let updated_at = Utc::now().to_rfc3339_opts(SecondsFormat::Nanos, true);
    let mut conn = db.conn.lock().map_err(|error| error.to_string())?;
    let transaction = conn.transaction().map_err(|error| error.to_string())?;
    let current_revision = transaction
        .query_row(
            "SELECT revision FROM stock_market_calendar_coverage WHERE market = ?1",
            [&final_calendar.market],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| error.to_string())?;
    if current_revision.as_deref() != expected_coverage_revision {
        let message = format!(
            "{COVERAGE_REVISION_MISMATCH}: expected {:?}, found {:?}",
            expected_coverage_revision, current_revision
        );
        transaction.rollback().map_err(|error| error.to_string())?;
        return Err(message);
    }
    if current_revision.as_deref() == Some(revision.as_str()) {
        let coverage_matches = transaction
            .query_row(
                "SELECT source, complete_start, complete_through, encodes_closed_dates
                 FROM stock_market_calendar_coverage WHERE market = ?1",
                [&final_calendar.market],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, i64>(3)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?
            .is_some_and(|(stored_source, stored_start, stored_end, closed_flag)| {
                stored_source == source
                    && stored_start == date_text(final_calendar.start)
                    && stored_end == date_text(final_calendar.end)
                    && closed_flag == 1
            });
        let rows_match = if coverage_matches {
            let stored_rows = transaction_calendar_rows(
                &transaction,
                &final_calendar.market,
                final_calendar.start,
                final_calendar.end,
            )?;
            let total_rows = transaction
                .query_row(
                    "SELECT COUNT(*) FROM stock_market_sessions WHERE market = ?1",
                    [&final_calendar.market],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| error.to_string())?;
            let wrong_sources = transaction
                .query_row(
                    "SELECT COUNT(*) FROM stock_market_sessions
                     WHERE market = ?1 AND source <> ?2",
                    params![final_calendar.market, source],
                    |row| row.get::<_, i64>(0),
                )
                .map_err(|error| error.to_string())?;
            stored_rows == final_calendar.rows
                && total_rows == final_calendar.rows.len() as i64
                && wrong_sources == 0
        } else {
            false
        };
        if rows_match {
            return Ok(CalendarSyncStatus::Reused);
        }
    }

    let replaces_entire_calendar = segments.len() == 1
        && segments[0].start == final_calendar.start
        && segments[0].end == final_calendar.end
        && segments[0].rows == final_calendar.rows;
    if replaces_entire_calendar {
        transaction
            .execute(
                "DELETE FROM stock_market_sessions WHERE market = ?1",
                [&final_calendar.market],
            )
            .map_err(|error| error.to_string())?;
    }
    for segment in segments {
        if !replaces_entire_calendar {
            transaction
                .execute(
                    "DELETE FROM stock_market_sessions
                     WHERE market = ?1 AND date BETWEEN ?2 AND ?3",
                    params![
                        segment.market,
                        date_text(segment.start),
                        date_text(segment.end)
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
        for (date, is_session) in &segment.rows {
            transaction
                .execute(
                    "INSERT INTO stock_market_sessions
                        (market, date, is_session, source, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5)",
                    params![
                        segment.market,
                        date_text(*date),
                        i64::from(*is_session),
                        source,
                        updated_at
                    ],
                )
                .map_err(|error| error.to_string())?;
        }
    }

    let stored_rows = transaction_calendar_rows(
        &transaction,
        &final_calendar.market,
        final_calendar.start,
        final_calendar.end,
    )?;
    if !rows_are_complete(&stored_rows, final_calendar.start, final_calendar.end)
        || stored_rows != final_calendar.rows
    {
        return Err(
            "Published market-calendar rows do not form the validated continuous range.".to_owned(),
        );
    }
    let stored_row_count = transaction
        .query_row(
            "SELECT COUNT(*) FROM stock_market_sessions WHERE market = ?1",
            [&final_calendar.market],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;
    let inconsistent_sources = transaction
        .query_row(
            "SELECT COUNT(*) FROM stock_market_sessions
             WHERE market = ?1 AND source <> ?2",
            params![final_calendar.market, source],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| error.to_string())?;
    if stored_row_count != final_calendar.rows.len() as i64 || inconsistent_sources != 0 {
        return Err(
            "Published market-calendar rows contain out-of-range or inconsistent source data."
                .to_owned(),
        );
    }

    transaction
        .execute(
            "INSERT INTO stock_market_calendar_coverage
                (market, source, complete_start, complete_through, revision,
                 encodes_closed_dates, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 0, ?6)
             ON CONFLICT(market) DO UPDATE SET
               source = excluded.source,
               complete_start = excluded.complete_start,
               complete_through = excluded.complete_through,
               revision = excluded.revision,
               encodes_closed_dates = 0,
               updated_at = excluded.updated_at",
            params![
                final_calendar.market,
                source,
                date_text(final_calendar.start),
                date_text(final_calendar.end),
                revision,
                updated_at
            ],
        )
        .map_err(|error| error.to_string())?;
    let updated = transaction
        .execute(
            "UPDATE stock_market_calendar_coverage
             SET encodes_closed_dates = 1 WHERE market = ?1",
            [&final_calendar.market],
        )
        .map_err(|error| error.to_string())?;
    if updated != 1 {
        return Err("Calendar coverage finalization did not update exactly one row.".to_owned());
    }
    transaction.commit().map_err(|error| error.to_string())?;
    drop(conn);

    let reloaded = load_market_sessions(
        db,
        &final_calendar.market,
        final_calendar.start,
        final_calendar.end,
    )?;
    if !reloaded.covers(final_calendar.start, final_calendar.end) {
        return Err("Committed market calendar failed its coverage reload check.".to_owned());
    }
    Ok(CalendarSyncStatus::Published)
}

#[allow(dead_code)] // Kept as the single-calendar publication seam required by the sync contract.
fn publish_validated_calendar(
    db: &Database,
    calendar: &ValidatedMarketCalendar,
    expected_coverage_revision: Option<&str>,
) -> Result<CalendarSyncStatus, String> {
    publish_validated_segments(
        db,
        std::slice::from_ref(calendar),
        calendar,
        expected_coverage_revision,
    )
}

fn merge_validated_calendar(
    request: &CalendarSyncRequest,
    resource_revision: &str,
    old_coverage: Option<&CalendarCoverageSnapshot>,
    keep_old_rows: bool,
    segments: &[ValidatedMarketCalendar],
) -> Result<ValidatedMarketCalendar, String> {
    let final_start = segments
        .iter()
        .map(|segment| segment.start)
        .chain(
            old_coverage
                .filter(|_| keep_old_rows)
                .map(|coverage| coverage.complete_start),
        )
        .min()
        .unwrap_or(request.required_start)
        .min(request.required_start);
    let final_end = segments
        .iter()
        .map(|segment| segment.end)
        .chain(
            old_coverage
                .filter(|_| keep_old_rows)
                .map(|coverage| coverage.complete_through),
        )
        .max()
        .unwrap_or(request.required_through)
        .max(request.required_through);
    let mut rows = BTreeMap::new();
    let mut providers = Vec::new();
    let mut references = Vec::new();
    let mut warnings = Vec::new();
    if keep_old_rows {
        if let Some(coverage) = old_coverage {
            rows.extend(coverage.rows.iter().copied());
            providers.extend(providers_from_source(&coverage.source));
        }
    }
    for segment in segments {
        rows.extend(segment.rows.iter().copied());
        providers.extend(segment.providers.iter().cloned());
        references.extend(segment.references.iter().cloned());
        warnings.extend(segment.warnings.iter().cloned());
    }
    if references.is_empty() {
        references.extend(
            reference_indices(&request.market)?
                .into_iter()
                .map(|reference| reference.logical_name.to_owned()),
        );
    }
    providers.sort();
    providers.dedup();
    references.sort();
    references.dedup();
    warnings.sort_by(|left, right| (&left.code, &left.message).cmp(&(&right.code, &right.message)));
    warnings.dedup();
    let rows: Vec<_> = rows.into_iter().collect();
    if !rows_are_complete(&rows, final_start, final_end) {
        return Err(
            "Validated segments are not adjacent to the existing calendar coverage.".to_owned(),
        );
    }
    Ok(ValidatedMarketCalendar {
        market: request.market.clone(),
        start: final_start,
        end: final_end,
        resource_revision: resource_revision.to_owned(),
        rows,
        providers,
        references,
        warnings,
    })
}

pub async fn sync_market_calendar_with_sources(
    db: &Database,
    request: CalendarSyncRequest,
    sources: &[Arc<dyn IndexHistorySource>],
) -> CalendarSyncOutcome {
    let Some(market_lock) = market_sync_locks().get(request.market.as_str()) else {
        return validation_failure_outcome(
            &request,
            None,
            format!("Unsupported stock-review market '{}'.", request.market),
        );
    };
    let _market_guard = market_lock.lock().await;
    let mut last_valid_coverage = None;

    for attempt in 0..=1 {
        let coverage = match read_calendar_coverage(db, &request.market) {
            Ok(coverage) => coverage,
            Err(message) => return validation_failure_outcome(&request, None, message),
        };
        let old_coverage = coverage.as_ref().filter(|coverage| coverage.valid);
        if let Some(coverage) = old_coverage {
            last_valid_coverage = Some(coverage.clone());
        }
        let stale_coverage = old_coverage.or_else(|| {
            coverage.as_ref().and_then(|coverage| {
                last_valid_coverage
                    .as_ref()
                    .filter(|last_valid| same_calendar_content(coverage, last_valid))
            })
        });
        let request_rules = match load_market_holiday_rules(
            &request.market,
            request.required_start,
            request.required_through,
        ) {
            Ok(rules) => rules,
            Err(message) => {
                return validation_failure_outcome(&request, stale_coverage, message);
            }
        };
        let resource_matches = old_coverage.is_some_and(|coverage| {
            revision_uses_resource(&coverage.revision, &request_rules.resource_revision)
        });
        if let Some(coverage) = old_coverage.filter(|coverage| {
            resource_matches
                && coverage.complete_start <= request.required_start
                && coverage.complete_through >= request.required_through
        }) {
            return successful_outcome(
                &request,
                CalendarSyncStatus::Reused,
                coverage.complete_start,
                coverage.complete_through,
                coverage_provider_warnings(coverage),
            );
        }

        let union_start = old_coverage
            .map(|coverage| coverage.complete_start.min(request.required_start))
            .unwrap_or(request.required_start);
        let union_end = old_coverage
            .map(|coverage| coverage.complete_through.max(request.required_through))
            .unwrap_or(request.required_through);
        let ranges = if old_coverage.is_none() || !resource_matches {
            vec![(union_start, union_end)]
        } else {
            let coverage = old_coverage.expect("matching coverage exists");
            let mut ranges = Vec::new();
            if request.required_start < coverage.complete_start {
                let Some(end) = coverage.complete_start.pred_opt() else {
                    return validation_failure_outcome(
                        &request,
                        stale_coverage,
                        "Calendar front-extension date underflow.".to_owned(),
                    );
                };
                ranges.push((request.required_start, end));
            }
            if request.required_through > coverage.complete_through {
                let Some(start) = coverage.complete_through.succ_opt() else {
                    return validation_failure_outcome(
                        &request,
                        stale_coverage,
                        "Calendar back-extension date overflow.".to_owned(),
                    );
                };
                ranges.push((start, request.required_through));
            }
            ranges
        };

        let mut segments = Vec::new();
        let mut validation_error = None;
        for (start, end) in ranges {
            let rules = match load_market_holiday_rules(&request.market, start, end) {
                Ok(rules) => rules,
                Err(message) => {
                    validation_error = Some(message);
                    break;
                }
            };
            let validation_request = CalendarValidationRequest {
                market: request.market.clone(),
                start,
                end,
                rules,
            };
            match validate_market_calendar(&validation_request, sources).await {
                Ok(calendar) => segments.push(calendar),
                Err(error) => {
                    validation_error = Some(error.message);
                    break;
                }
            }
        }
        if let Some(message) = validation_error {
            return validation_failure_outcome(&request, stale_coverage, message);
        }

        let mut keep_old_rows = old_coverage.is_some() && resource_matches;
        if let Some(coverage) = old_coverage.filter(|_| keep_old_rows) {
            let durable_source =
                canonical_calendar_source(&providers_from_source(&coverage.source));
            let segment_sources_are_coherent = segments
                .iter()
                .all(|segment| canonical_calendar_source(&segment.providers) == durable_source);
            if !segment_sources_are_coherent {
                let rules = match load_market_holiday_rules(&request.market, union_start, union_end)
                {
                    Ok(rules) => rules,
                    Err(message) => {
                        return validation_failure_outcome(&request, stale_coverage, message);
                    }
                };
                let validation_request = CalendarValidationRequest {
                    market: request.market.clone(),
                    start: union_start,
                    end: union_end,
                    rules,
                };
                match validate_market_calendar(&validation_request, sources).await {
                    Ok(calendar) => {
                        segments = vec![calendar];
                        keep_old_rows = false;
                    }
                    Err(error) => {
                        return validation_failure_outcome(&request, stale_coverage, error.message);
                    }
                }
            }
        }
        let final_calendar = match merge_validated_calendar(
            &request,
            &request_rules.resource_revision,
            old_coverage,
            keep_old_rows,
            &segments,
        ) {
            Ok(calendar) => calendar,
            Err(message) => return validation_failure_outcome(&request, stale_coverage, message),
        };
        let expected_revision = coverage.as_ref().map(|coverage| coverage.revision.as_str());
        match publish_validated_segments(db, &segments, &final_calendar, expected_revision) {
            Ok(status) => {
                return successful_outcome(
                    &request,
                    status,
                    final_calendar.start,
                    final_calendar.end,
                    final_calendar.warnings,
                );
            }
            Err(message) if attempt == 0 && message.starts_with(COVERAGE_REVISION_MISMATCH) => {
                continue;
            }
            Err(message) => return validation_failure_outcome(&request, stale_coverage, message),
        }
    }
    unreachable!("calendar publication attempts are bounded by the loop")
}

#[allow(dead_code)] // Consumed by the Task 4 report-integration entry point.
pub async fn sync_market_calendars_with_sources(
    db: &Database,
    markets: &BTreeSet<String>,
    required_start: NaiveDate,
    now: DateTime<Utc>,
    sources: &[Arc<dyn IndexHistorySource>],
) -> Vec<CalendarSyncOutcome> {
    let mut outcomes = Vec::with_capacity(markets.len());
    for market in markets {
        let required_through = match latest_fully_closed_date(market, now) {
            Ok(date) => date,
            Err(message) => {
                let request = CalendarSyncRequest {
                    market: market.clone(),
                    required_start,
                    required_through: required_start,
                };
                outcomes.push(validation_failure_outcome(&request, None, message));
                continue;
            }
        };
        let requested = CalendarSyncRequest {
            market: market.clone(),
            required_start,
            required_through,
        };
        let (resource_start, resource_end) = match market_holiday_resource_bounds(market) {
            Ok(bounds) => bounds,
            Err(message) => {
                outcomes.push(validation_failure_outcome(&requested, None, message));
                continue;
            }
        };
        let supported_start = required_start.max(resource_start);
        let supported_through = required_through.min(resource_end);
        if supported_start > supported_through {
            let coverage = read_calendar_coverage(db, market).ok().flatten();
            outcomes.push(validation_failure_outcome(
                &requested,
                coverage.as_ref().filter(|coverage| coverage.valid),
                format!(
                    "Requested {market} calendar range {required_start} through {required_through} does not intersect the supported resource range {resource_start} through {resource_end}."
                ),
            ));
            continue;
        }
        let mut outcome = sync_market_calendar_with_sources(
            db,
            CalendarSyncRequest {
                market: market.clone(),
                required_start: supported_start,
                required_through: supported_through,
            },
            sources,
        )
        .await;
        outcome.requested_start = required_start;
        outcome.requested_through = required_through;
        outcomes.push(outcome);
    }
    outcomes
}

#[allow(dead_code)] // Production wrapper consumed by Task 4.
pub async fn sync_market_calendars(
    db: &Database,
    markets: &BTreeSet<String>,
    required_start: NaiveDate,
    now: DateTime<Utc>,
) -> Vec<CalendarSyncOutcome> {
    let sources = live_index_history_sources();
    sync_market_calendars_with_sources(db, markets, required_start, now, &sources).await
}

#[cfg(test)]
mod tests {
    use super::{
        latest_fully_closed_date, load_market_holiday_rules, parse_market_holiday_bundle,
        publish_validated_calendar, stable_calendar_revision, sync_market_calendar_with_sources,
        sync_market_calendars_with_sources, validate_market_calendar, CalendarProvider,
        CalendarSyncRequest, CalendarSyncStatus, CalendarValidationErrorKind,
        CalendarValidationRequest, IndexHistoryEvidence, IndexHistorySource, ReferenceIndex,
        ValidatedMarketCalendar,
    };
    use crate::db::Database;
    use chrono::{NaiveDate, TimeZone, Utc};
    use rusqlite::params;
    use std::collections::{BTreeMap, BTreeSet};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    fn day(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
    }

    #[derive(Clone)]
    enum FakeOutcome {
        Dates {
            values: Vec<NaiveDate>,
            complete_response: bool,
        },
        Error(&'static str),
    }

    struct FakeIndexHistorySource {
        provider: CalendarProvider,
        outcomes: BTreeMap<String, FakeOutcome>,
    }

    impl IndexHistorySource for FakeIndexHistorySource {
        fn provider(&self) -> CalendarProvider {
            self.provider
        }

        fn fetch<'a>(
            &'a self,
            _market: &'a str,
            reference: &'a ReferenceIndex,
            start: NaiveDate,
            end: NaiveDate,
        ) -> Pin<Box<dyn Future<Output = Result<IndexHistoryEvidence, String>> + Send + 'a>>
        {
            let provider = self.provider;
            let logical_index = reference.logical_name.to_owned();
            let outcome = self
                .outcomes
                .get(reference.logical_name)
                .cloned()
                .unwrap_or(FakeOutcome::Error("missing fake outcome"));
            Box::pin(async move {
                match outcome {
                    FakeOutcome::Dates {
                        values,
                        complete_response,
                    } => Ok(IndexHistoryEvidence {
                        provider,
                        logical_index,
                        request_start: start,
                        request_end: end,
                        session_dates: values,
                        complete_response,
                    }),
                    FakeOutcome::Error(message) => Err(message.to_owned()),
                }
            })
        }
    }

    fn dates(values: &[&str]) -> Vec<NaiveDate> {
        values.iter().map(|value| day(value)).collect()
    }

    fn success(values: &[&str]) -> FakeOutcome {
        FakeOutcome::Dates {
            values: dates(values),
            complete_response: true,
        }
    }

    fn provider_source(
        provider: CalendarProvider,
        left_name: &str,
        left: FakeOutcome,
        right_name: &str,
        right: FakeOutcome,
    ) -> Arc<dyn IndexHistorySource> {
        Arc::new(FakeIndexHistorySource {
            provider,
            outcomes: BTreeMap::from([
                (left_name.to_owned(), left),
                (right_name.to_owned(), right),
            ]),
        })
    }

    fn us_source(
        provider: CalendarProvider,
        sp500: FakeOutcome,
        nasdaq: FakeOutcome,
    ) -> Arc<dyn IndexHistorySource> {
        provider_source(provider, "sp500", sp500, "nasdaq_composite", nasdaq)
    }

    fn fixture_request(market: &str, start: &str, end: &str) -> CalendarValidationRequest {
        let start = day(start);
        let end = day(end);
        CalendarValidationRequest {
            market: market.to_owned(),
            start,
            end,
            rules: load_market_holiday_rules(market, start, end).unwrap(),
        }
    }

    fn fixture_sync_request(market: &str, start: &str, end: &str) -> CalendarSyncRequest {
        CalendarSyncRequest {
            market: market.to_owned(),
            required_start: day(start),
            required_through: day(end),
        }
    }

    fn fixture_request_a() -> CalendarSyncRequest {
        fixture_sync_request("US", "2026-01-01", "2026-01-09")
    }

    fn extended_request() -> CalendarSyncRequest {
        fixture_sync_request("US", "2026-01-01", "2026-01-09")
    }

    fn fixture_validated_range(
        start: &str,
        end: &str,
        resource_revision: &str,
    ) -> ValidatedMarketCalendar {
        let start = day(start);
        let end = day(end);
        let rules = load_market_holiday_rules("US", start, end).unwrap();
        ValidatedMarketCalendar {
            market: "US".to_owned(),
            start,
            end,
            resource_revision: resource_revision.to_owned(),
            rows: super::dates_inclusive(start, end)
                .into_iter()
                .map(|date| {
                    let is_session = super::is_expected_session(&rules, date);
                    (date, is_session)
                })
                .collect(),
            providers: vec!["eastmoney".to_owned()],
            references: vec!["nasdaq_composite".to_owned(), "sp500".to_owned()],
            warnings: Vec::new(),
        }
    }

    fn fixture_validated_a() -> ValidatedMarketCalendar {
        let rules = load_market_holiday_rules("US", day("2026-01-01"), day("2026-01-07")).unwrap();
        fixture_validated_range("2026-01-01", "2026-01-07", &rules.resource_revision)
    }

    struct CountingFakeSource {
        calls: Mutex<Vec<(String, NaiveDate, NaiveDate)>>,
        conflict: bool,
    }

    impl CountingFakeSource {
        fn complete() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                conflict: false,
            }
        }

        fn conflicting() -> Self {
            Self {
                calls: Mutex::new(Vec::new()),
                conflict: true,
            }
        }

        fn calls(&self) -> Vec<(String, NaiveDate, NaiveDate)> {
            self.calls.lock().unwrap().clone()
        }

        fn clear_calls(&self) {
            self.calls.lock().unwrap().clear();
        }
    }

    struct RangeBoundedFakeSource {
        provider: CalendarProvider,
        available_start: NaiveDate,
        available_end: NaiveDate,
        calls: Mutex<Vec<(String, NaiveDate, NaiveDate)>>,
    }

    impl RangeBoundedFakeSource {
        fn new(provider: CalendarProvider, available_start: &str, available_end: &str) -> Self {
            Self {
                provider,
                available_start: day(available_start),
                available_end: day(available_end),
                calls: Mutex::new(Vec::new()),
            }
        }

        fn calls(&self) -> Vec<(String, NaiveDate, NaiveDate)> {
            self.calls.lock().unwrap().clone()
        }
    }

    impl IndexHistorySource for RangeBoundedFakeSource {
        fn provider(&self) -> CalendarProvider {
            self.provider
        }

        fn fetch<'a>(
            &'a self,
            market: &'a str,
            reference: &'a ReferenceIndex,
            start: NaiveDate,
            end: NaiveDate,
        ) -> Pin<Box<dyn Future<Output = Result<IndexHistoryEvidence, String>> + Send + 'a>>
        {
            self.calls
                .lock()
                .unwrap()
                .push((reference.logical_name.to_owned(), start, end));
            let provider = self.provider;
            let logical_index = reference.logical_name.to_owned();
            let available_start = self.available_start;
            let available_end = self.available_end;
            Box::pin(async move {
                if start < available_start || end > available_end {
                    return Err(format!(
                        "{} evidence is unavailable outside {available_start} through {available_end}.",
                        provider.as_str()
                    ));
                }
                let rules = load_market_holiday_rules(market, start, end)?;
                let session_dates = super::dates_inclusive(start, end)
                    .into_iter()
                    .filter(|date| super::is_expected_session(&rules, *date))
                    .collect();
                Ok(IndexHistoryEvidence {
                    provider,
                    logical_index,
                    request_start: start,
                    request_end: end,
                    session_dates,
                    complete_response: true,
                })
            })
        }
    }

    impl IndexHistorySource for CountingFakeSource {
        fn provider(&self) -> CalendarProvider {
            CalendarProvider::EastMoney
        }

        fn fetch<'a>(
            &'a self,
            market: &'a str,
            reference: &'a ReferenceIndex,
            start: NaiveDate,
            end: NaiveDate,
        ) -> Pin<Box<dyn Future<Output = Result<IndexHistoryEvidence, String>> + Send + 'a>>
        {
            self.calls
                .lock()
                .unwrap()
                .push((reference.logical_name.to_owned(), start, end));
            let provider = self.provider();
            let logical_index = reference.logical_name.to_owned();
            let conflict = self.conflict && reference.logical_name == "nasdaq_composite";
            Box::pin(async move {
                let rules = load_market_holiday_rules(market, start, end)?;
                let mut session_dates: Vec<_> = super::dates_inclusive(start, end)
                    .into_iter()
                    .filter(|date| super::is_expected_session(&rules, *date))
                    .collect();
                if conflict {
                    session_dates.pop();
                }
                Ok(IndexHistoryEvidence {
                    provider,
                    logical_index,
                    request_start: start,
                    request_end: end,
                    session_dates,
                    complete_response: true,
                })
            })
        }
    }

    fn complete_fake_sources() -> Vec<Arc<dyn IndexHistorySource>> {
        vec![Arc::new(CountingFakeSource::complete())]
    }

    fn conflicting_sources() -> Vec<Arc<dyn IndexHistorySource>> {
        vec![Arc::new(CountingFakeSource::conflicting())]
    }

    struct RevisionThrashingSource {
        db: Arc<Database>,
        fetch_count: AtomicUsize,
    }

    impl IndexHistorySource for RevisionThrashingSource {
        fn provider(&self) -> CalendarProvider {
            CalendarProvider::EastMoney
        }

        fn fetch<'a>(
            &'a self,
            market: &'a str,
            reference: &'a ReferenceIndex,
            start: NaiveDate,
            end: NaiveDate,
        ) -> Pin<Box<dyn Future<Output = Result<IndexHistoryEvidence, String>> + Send + 'a>>
        {
            let fetch_number = self.fetch_count.fetch_add(1, Ordering::SeqCst) + 1;
            let db = self.db.clone();
            let provider = self.provider();
            let logical_index = reference.logical_name.to_owned();
            let market = market.to_owned();
            Box::pin(async move {
                db.conn
                    .lock()
                    .unwrap()
                    .execute(
                        "UPDATE stock_market_calendar_coverage SET revision = ?1 WHERE market = ?2",
                        rusqlite::params![format!("external-{fetch_number}"), market],
                    )
                    .unwrap();
                let rules = load_market_holiday_rules(&market, start, end)?;
                let session_dates = super::dates_inclusive(start, end)
                    .into_iter()
                    .filter(|date| super::is_expected_session(&rules, *date))
                    .collect();
                Ok(IndexHistoryEvidence {
                    provider,
                    logical_index,
                    request_start: start,
                    request_end: end,
                    session_dates,
                    complete_response: true,
                })
            })
        }
    }

    fn row_count(db: &Database, table: &str, market: &str) -> i64 {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                &format!("SELECT COUNT(*) FROM {table} WHERE market = ?1"),
                [market],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn coverage_flag(db: &Database, market: &str) -> i64 {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT encodes_closed_dates FROM stock_market_calendar_coverage WHERE market = ?1",
                [market],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn coverage_revision(db: &Database, market: &str) -> String {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT revision FROM stock_market_calendar_coverage WHERE market = ?1",
                [market],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn coverage_source(db: &Database, market: &str) -> String {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT source FROM stock_market_calendar_coverage WHERE market = ?1",
                [market],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn session_flag(db: &Database, market: &str, date: &str) -> i64 {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT is_session FROM stock_market_sessions WHERE market = ?1 AND date = ?2",
                params![market, date],
                |row| row.get(0),
            )
            .unwrap()
    }

    fn coverage_bounds(db: &Database, market: &str) -> (String, String) {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT complete_start, complete_through FROM stock_market_calendar_coverage WHERE market = ?1",
                [market],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap()
    }

    fn calendar_timestamps(db: &Database, market: &str) -> (String, String) {
        let conn = db.conn.lock().unwrap();
        let coverage_updated_at = conn
            .query_row(
                "SELECT updated_at FROM stock_market_calendar_coverage WHERE market = ?1",
                [market],
                |row| row.get(0),
            )
            .unwrap();
        let session_updated_at = conn
            .query_row(
                "SELECT GROUP_CONCAT(updated_at, '|') FROM (
                    SELECT updated_at FROM stock_market_sessions WHERE market = ?1 ORDER BY date
                 )",
                [market],
                |row| row.get(0),
            )
            .unwrap();
        (coverage_updated_at, session_updated_at)
    }

    fn install_coverage_abort_trigger(db: &Database) {
        db.conn
            .lock()
            .unwrap()
            .execute_batch(
                "CREATE TRIGGER abort_calendar_coverage_insert
                 BEFORE INSERT ON stock_market_calendar_coverage
                 BEGIN
                   SELECT RAISE(ABORT, 'fixture coverage failure');
                 END;",
            )
            .unwrap();
    }

    fn assert_calendar_rows_match_coverage(db: &Database, market: &str) {
        let (start, end) = coverage_bounds(db, market);
        let start = day(&start);
        let end = day(&end);
        assert_eq!(
            row_count(db, "stock_market_sessions", market),
            (end - start).num_days() + 1
        );
        let invalid_rows: i64 = db
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(*) FROM stock_market_sessions
                 WHERE market = ?1 AND is_session NOT IN (0, 1)",
                [market],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(invalid_rows, 0);
    }

    fn distinct_calendar_revisions(db: &Database, market: &str) -> i64 {
        db.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT COUNT(DISTINCT revision) FROM stock_market_calendar_coverage WHERE market = ?1",
                [market],
                |row| row.get(0),
            )
            .unwrap()
    }

    #[test]
    fn embedded_rules_cover_only_declared_2025_2026_range() {
        let rules = load_market_holiday_rules("CN", day("2025-01-01"), day("2026-12-31")).unwrap();
        assert_eq!(rules.complete_start, day("2025-01-01"));
        assert_eq!(rules.complete_through, day("2026-12-31"));
        assert!(rules.weekday_closures.contains(&day("2025-02-03")));
        assert!(rules.weekday_closures.contains(&day("2026-10-07")));
        assert!(load_market_holiday_rules("CN", day("2024-12-31"), day("2025-01-02")).is_err());
        assert!(load_market_holiday_rules("CN", day("2026-12-30"), day("2027-01-01")).is_err());
    }

    #[test]
    fn embedded_us_rules_include_the_2025_carter_mourning_closure_and_dated_notices() {
        let rules = load_market_holiday_rules("US", day("2025-01-01"), day("2026-12-31")).unwrap();

        assert_eq!(rules.resource_revision, "exchange-holidays-v2-2025-2026");
        assert!(rules.weekday_closures.contains(&day("2025-01-09")));
        assert!(rules
            .source_urls
            .contains(&"https://www.nasdaqtrader.com/TraderNews.aspx?id=ETA2024-87".to_string()));
        assert!(rules.source_urls.contains(&"https://www.nyse.com/publicdocs/nyse/markets/american-options/rule-interpretations/2025/National_Day_of_Mourning_20250102.pdf".to_string()));
        assert!(rules
            .notice_versions
            .contains(&"Nasdaq Equity Trader Alert #2024-86 (2024-12-30)".to_string()));
        assert!(rules
            .notice_versions
            .contains(&"NYSE American/Arca Options RM-25-01 (2025-01-02)".to_string()));
    }

    #[test]
    fn embedded_rules_reject_duplicates_weekends_and_wrong_years() {
        let duplicate = r#"{"revision":"bad","entries":[{"market":"US","year":2026,"source_urls":["https://www.nyse.com/trade/hours-calendars"],"notice_versions":["bad"],"closed_weekdays":["2026-01-01","2026-01-01","2026-01-03"]}]}"#;
        let error = parse_market_holiday_bundle(duplicate).unwrap_err();
        assert!(error.contains("duplicate"));

        let weekend = r#"{"revision":"bad","entries":[{"market":"US","year":2026,"source_urls":["https://www.nyse.com/trade/hours-calendars"],"notice_versions":["bad"],"closed_weekdays":["2026-01-03"]}]}"#;
        assert!(parse_market_holiday_bundle(weekend)
            .unwrap_err()
            .contains("weekend"));

        let wrong_year = r#"{"revision":"bad","entries":[{"market":"US","year":2026,"source_urls":["https://www.nyse.com/trade/hours-calendars"],"notice_versions":["bad"],"closed_weekdays":["2025-01-01"]}]}"#;
        assert!(parse_market_holiday_bundle(wrong_year)
            .unwrap_err()
            .contains("year"));
    }

    #[test]
    fn embedded_rules_reject_invalid_entry_metadata_and_year_sequences() {
        let unknown_market = r#"{"revision":"bad","entries":[{"market":"JP","year":2026,"source_urls":["https://example.com"],"notice_versions":["bad"],"closed_weekdays":[]}]}"#;
        assert!(parse_market_holiday_bundle(unknown_market)
            .unwrap_err()
            .contains("Unsupported"));

        let empty_sources = r#"{"revision":"bad","entries":[{"market":"US","year":2026,"source_urls":[],"notice_versions":["bad"],"closed_weekdays":[]}]}"#;
        assert!(parse_market_holiday_bundle(empty_sources)
            .unwrap_err()
            .contains("source"));

        let non_consecutive = r#"{"revision":"bad","entries":[{"market":"US","year":2025,"source_urls":["https://example.com"],"notice_versions":["bad"],"closed_weekdays":[]},{"market":"US","year":2027,"source_urls":["https://example.com"],"notice_versions":["bad"],"closed_weekdays":[]}]}"#;
        assert!(parse_market_holiday_bundle(non_consecutive)
            .unwrap_err()
            .contains("consecutive"));
    }

    #[test]
    fn latest_closed_date_uses_exchange_timezone_and_conservative_close_buffer() {
        let before_cn_close = Utc.with_ymd_and_hms(2026, 8, 28, 7, 20, 0).unwrap();
        let after_cn_close = Utc.with_ymd_and_hms(2026, 8, 28, 7, 40, 0).unwrap();
        assert_eq!(
            latest_fully_closed_date("CN", before_cn_close).unwrap(),
            day("2026-08-27")
        );
        assert_eq!(
            latest_fully_closed_date("CN", after_cn_close).unwrap(),
            day("2026-08-28")
        );

        let before_hk_close = Utc.with_ymd_and_hms(2026, 8, 28, 8, 20, 0).unwrap();
        let after_hk_close = Utc.with_ymd_and_hms(2026, 8, 28, 8, 40, 0).unwrap();
        assert_eq!(
            latest_fully_closed_date("HK", before_hk_close).unwrap(),
            day("2026-08-27")
        );
        assert_eq!(
            latest_fully_closed_date("HK", after_hk_close).unwrap(),
            day("2026-08-28")
        );

        let before_us_close_in_dst = Utc.with_ymd_and_hms(2026, 7, 6, 20, 20, 0).unwrap();
        let after_us_close_in_dst = Utc.with_ymd_and_hms(2026, 7, 6, 20, 40, 0).unwrap();
        assert_eq!(
            latest_fully_closed_date("US", before_us_close_in_dst).unwrap(),
            day("2026-07-05")
        );
        assert_eq!(
            latest_fully_closed_date("US", after_us_close_in_dst).unwrap(),
            day("2026-07-06")
        );
    }

    #[test]
    fn latest_closed_date_rejects_unknown_market() {
        let now = Utc.with_ymd_and_hms(2026, 7, 6, 20, 40, 0).unwrap();
        assert!(latest_fully_closed_date("JP", now)
            .unwrap_err()
            .contains("Unsupported"));
    }

    #[tokio::test]
    async fn matching_two_index_two_provider_evidence_validates_every_calendar_day() {
        let request = fixture_request("US", "2026-01-01", "2026-01-09");
        let expected = [
            "2026-01-02",
            "2026-01-05",
            "2026-01-06",
            "2026-01-07",
            "2026-01-08",
            "2026-01-09",
        ];
        let sources = vec![
            us_source(
                CalendarProvider::Xueqiu,
                success(&expected),
                success(&expected),
            ),
            us_source(
                CalendarProvider::EastMoney,
                success(&expected),
                success(&expected),
            ),
        ];

        let validated = validate_market_calendar(&request, &sources).await.unwrap();

        assert_eq!(validated.rows.len(), 9);
        assert_eq!(validated.rows[0], (day("2026-01-01"), false));
        assert_eq!(validated.rows[1], (day("2026-01-02"), true));
        assert_eq!(validated.providers, vec!["eastmoney", "xueqiu"]);
        assert_eq!(validated.references, vec!["nasdaq_composite", "sp500"]);

        let mut reversed = expected.iter().rev().copied().collect::<Vec<_>>();
        reversed.push("2026-01-09");
        let reordered_sources = vec![
            us_source(
                CalendarProvider::EastMoney,
                success(&reversed),
                success(&reversed),
            ),
            us_source(
                CalendarProvider::Xueqiu,
                success(&reversed),
                success(&reversed),
            ),
        ];
        let reordered = validate_market_calendar(&request, &reordered_sources)
            .await
            .unwrap();
        assert_eq!(
            stable_calendar_revision(&validated),
            stable_calendar_revision(&reordered)
        );
    }

    #[tokio::test]
    async fn carter_mourning_week_validates_without_a_january_ninth_bar() {
        let request = fixture_request("US", "2025-01-06", "2025-01-10");
        let expected = ["2025-01-06", "2025-01-07", "2025-01-08", "2025-01-10"];
        let sources = vec![us_source(
            CalendarProvider::EastMoney,
            success(&expected),
            success(&expected),
        )];

        let validated = validate_market_calendar(&request, &sources).await.unwrap();

        assert_eq!(validated.rows.len(), 5);
        assert_eq!(validated.rows[3], (day("2025-01-09"), false));
    }

    #[tokio::test]
    async fn one_missing_expected_weekday_or_provider_conflict_rejects_publish() {
        let request = fixture_request("US", "2026-01-01", "2026-01-09");
        let missing = [
            "2026-01-02",
            "2026-01-05",
            "2026-01-07",
            "2026-01-08",
            "2026-01-09",
        ];
        let error = validate_market_calendar(
            &request,
            &[us_source(
                CalendarProvider::EastMoney,
                success(&missing),
                success(&missing),
            )],
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error.kind,
            CalendarValidationErrorKind::MissingExpectedSession
        ));

        let complete = [
            "2026-01-02",
            "2026-01-05",
            "2026-01-06",
            "2026-01-07",
            "2026-01-08",
            "2026-01-09",
        ];
        let conflict = [
            "2026-01-02",
            "2026-01-05",
            "2026-01-06",
            "2026-01-07",
            "2026-01-09",
        ];
        let error = validate_market_calendar(
            &request,
            &[
                us_source(
                    CalendarProvider::Xueqiu,
                    success(&complete),
                    success(&complete),
                ),
                us_source(
                    CalendarProvider::EastMoney,
                    success(&conflict),
                    success(&conflict),
                ),
            ],
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error.kind,
            CalendarValidationErrorKind::ProviderConflict
        ));
    }

    #[tokio::test]
    async fn one_complete_provider_is_accepted_only_with_full_official_expectation() {
        let request = fixture_request("CN", "2026-02-16", "2026-02-27");
        let expected = ["2026-02-24", "2026-02-25", "2026-02-26", "2026-02-27"];
        let sources = vec![
            provider_source(
                CalendarProvider::Xueqiu,
                "sse_composite",
                FakeOutcome::Error("xueqiu unavailable"),
                "shenzhen_component",
                FakeOutcome::Error("xueqiu unavailable"),
            ),
            provider_source(
                CalendarProvider::EastMoney,
                "sse_composite",
                success(&expected),
                "shenzhen_component",
                success(&expected),
            ),
        ];

        let validated = validate_market_calendar(&request, &sources).await.unwrap();

        assert_eq!(validated.providers, vec!["eastmoney"]);
        assert!(validated
            .warnings
            .iter()
            .any(|item| item.code == "market_calendar_single_provider"));
    }

    #[tokio::test]
    async fn weekend_or_official_closed_date_bars_are_rejected() {
        let request = fixture_request("US", "2026-01-01", "2026-01-05");
        for unexpected in ["2026-01-01", "2026-01-03"] {
            let values = ["2026-01-02", unexpected, "2026-01-05"];
            let error = validate_market_calendar(
                &request,
                &[us_source(
                    CalendarProvider::EastMoney,
                    success(&values),
                    success(&values),
                )],
            )
            .await
            .unwrap_err();
            assert!(matches!(
                error.kind,
                CalendarValidationErrorKind::UnexpectedClosedDateBar
            ));
        }
    }

    #[tokio::test]
    async fn mismatched_indices_from_one_provider_are_rejected() {
        let request = fixture_request("US", "2026-01-01", "2026-01-09");
        let complete = [
            "2026-01-02",
            "2026-01-05",
            "2026-01-06",
            "2026-01-07",
            "2026-01-08",
            "2026-01-09",
        ];
        let missing = [
            "2026-01-02",
            "2026-01-05",
            "2026-01-06",
            "2026-01-07",
            "2026-01-09",
        ];
        let error = validate_market_calendar(
            &request,
            &[us_source(
                CalendarProvider::Xueqiu,
                success(&complete),
                success(&missing),
            )],
        )
        .await
        .unwrap_err();

        assert!(matches!(
            error.kind,
            CalendarValidationErrorKind::IndexConflict
        ));
    }

    #[tokio::test]
    async fn truncated_responses_and_all_failed_requests_are_rejected() {
        let request = fixture_request("US", "2026-01-01", "2026-01-09");
        let expected = dates(&[
            "2026-01-02",
            "2026-01-05",
            "2026-01-06",
            "2026-01-07",
            "2026-01-08",
            "2026-01-09",
        ]);
        let incomplete = FakeOutcome::Dates {
            values: expected,
            complete_response: false,
        };
        let error = validate_market_calendar(
            &request,
            &[us_source(
                CalendarProvider::Xueqiu,
                incomplete.clone(),
                incomplete,
            )],
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error.kind,
            CalendarValidationErrorKind::TruncatedResponse
        ));

        let error = validate_market_calendar(
            &request,
            &[us_source(
                CalendarProvider::Xueqiu,
                FakeOutcome::Error("left failed"),
                FakeOutcome::Error("right failed"),
            )],
        )
        .await
        .unwrap_err();
        assert!(matches!(
            error.kind,
            CalendarValidationErrorKind::ProviderUnavailable
        ));
    }

    #[tokio::test]
    async fn closed_only_range_uses_left_anchor_but_publishes_only_requested_days() {
        let request = fixture_request("US", "2026-01-01", "2026-01-01");
        let sources = vec![us_source(
            CalendarProvider::EastMoney,
            success(&["2025-12-31"]),
            success(&["2025-12-31"]),
        )];

        let validated = validate_market_calendar(&request, &sources).await.unwrap();

        assert_eq!(validated.rows, vec![(day("2026-01-01"), false)]);
    }

    #[tokio::test]
    async fn closed_only_range_without_resource_anchor_is_rejected() {
        let request = fixture_request("US", "2025-01-01", "2025-01-01");
        let error = validate_market_calendar(&request, &[]).await.unwrap_err();

        assert!(matches!(
            error.kind,
            CalendarValidationErrorKind::MissingAnchorSession
        ));
    }

    #[tokio::test]
    async fn first_sync_writes_every_natural_day_and_complete_coverage() {
        let db = Database::new(":memory:").unwrap();
        let sources = complete_fake_sources();
        let outcome = sync_market_calendar_with_sources(
            &db,
            fixture_sync_request("US", "2026-01-01", "2026-01-09"),
            &sources,
        )
        .await;
        assert_eq!(outcome.status, CalendarSyncStatus::Published);
        assert_eq!(row_count(&db, "stock_market_sessions", "US"), 9);
        assert_eq!(coverage_flag(&db, "US"), 1);
    }

    #[tokio::test]
    async fn batch_sync_publishes_only_the_resource_supported_intersection() {
        let db = Database::new(":memory:").unwrap();
        let sources = complete_fake_sources();
        let outcomes = sync_market_calendars_with_sources(
            &db,
            &BTreeSet::from(["US".to_owned()]),
            day("2024-01-01"),
            Utc.with_ymd_and_hms(2025, 1, 10, 22, 0, 0).unwrap(),
            &sources,
        )
        .await;

        assert_eq!(outcomes[0].status, CalendarSyncStatus::Published);
        assert_eq!(outcomes[0].requested_start, day("2024-01-01"));
        assert_eq!(outcomes[0].requested_through, day("2025-01-10"));
        assert_eq!(outcomes[0].available_start, Some(day("2025-01-01")));
        assert_eq!(outcomes[0].available_through, Some(day("2025-01-10")));
        assert_eq!(
            coverage_bounds(&db, "US"),
            ("2025-01-01".into(), "2025-01-10".into())
        );
        assert_eq!(row_count(&db, "stock_market_sessions", "US"), 10);
    }

    #[tokio::test]
    async fn coverage_write_failure_rolls_back_session_rows() {
        let db = Database::new(":memory:").unwrap();
        install_coverage_abort_trigger(&db);
        let result = publish_validated_calendar(&db, &fixture_validated_a(), None);
        assert!(result.is_err());
        assert_eq!(row_count(&db, "stock_market_sessions", "US"), 0);
        assert_eq!(row_count(&db, "stock_market_calendar_coverage", "US"), 0);
    }

    #[tokio::test]
    async fn refresh_failure_preserves_last_complete_revision() {
        let db = Database::new(":memory:").unwrap();
        publish_validated_calendar(&db, &fixture_validated_a(), None).unwrap();
        let before = coverage_revision(&db, "US");
        let sources = conflicting_sources();
        let outcome = sync_market_calendar_with_sources(&db, extended_request(), &sources).await;
        assert_eq!(outcome.status, CalendarSyncStatus::StaleCacheUsed);
        assert_eq!(
            outcome.issue_code.as_deref(),
            Some("market_calendar_refresh_failed")
        );
        assert_eq!(coverage_revision(&db, "US"), before);
        assert_eq!(row_count(&db, "stock_market_sessions", "US"), 7);
    }

    #[tokio::test]
    async fn same_market_concurrent_syncs_publish_one_coherent_revision() {
        let db = Arc::new(Database::new(":memory:").unwrap());
        let source = Arc::new(CountingFakeSource::complete());
        let sources: Vec<Arc<dyn IndexHistorySource>> = vec![source.clone()];
        let (left, right) = tokio::join!(
            sync_market_calendar_with_sources(&db, fixture_request_a(), &sources),
            sync_market_calendar_with_sources(&db, fixture_request_a(), &sources),
        );
        assert!(left.status.is_success());
        assert!(right.status.is_success());
        assert_calendar_rows_match_coverage(&db, "US");
        assert_eq!(distinct_calendar_revisions(&db, "US"), 1);
        assert_eq!(source.calls().len(), 2);
    }

    #[tokio::test]
    async fn matching_resource_revision_extends_only_missing_back_segment() {
        let db = Database::new(":memory:").unwrap();
        let source = Arc::new(CountingFakeSource::complete());
        let sources: Vec<Arc<dyn IndexHistorySource>> = vec![source.clone()];
        let first = sync_market_calendar_with_sources(
            &db,
            fixture_sync_request("US", "2026-01-05", "2026-01-07"),
            &sources,
        )
        .await;
        assert_eq!(first.status, CalendarSyncStatus::Published);
        source.clear_calls();

        let second = sync_market_calendar_with_sources(
            &db,
            fixture_sync_request("US", "2026-01-05", "2026-01-09"),
            &sources,
        )
        .await;

        assert_eq!(second.status, CalendarSyncStatus::Published);
        assert_eq!(
            coverage_bounds(&db, "US"),
            ("2026-01-05".into(), "2026-01-09".into())
        );
        assert_eq!(row_count(&db, "stock_market_sessions", "US"), 5);
        assert!(source
            .calls()
            .iter()
            .all(|(_, start, end)| { *start == day("2026-01-08") && *end == day("2026-01-09") }));
        let combined = fixture_validated_range(
            "2026-01-05",
            "2026-01-09",
            &load_market_holiday_rules("US", day("2026-01-05"), day("2026-01-09"))
                .unwrap()
                .resource_revision,
        );
        assert_eq!(
            coverage_revision(&db, "US"),
            stable_calendar_revision(&combined)
        );
    }

    #[tokio::test]
    async fn matching_resource_revision_extends_only_missing_front_segment() {
        let db = Database::new(":memory:").unwrap();
        let source = Arc::new(CountingFakeSource::complete());
        let sources: Vec<Arc<dyn IndexHistorySource>> = vec![source.clone()];
        sync_market_calendar_with_sources(
            &db,
            fixture_sync_request("US", "2026-01-05", "2026-01-09"),
            &sources,
        )
        .await;
        source.clear_calls();

        let outcome = sync_market_calendar_with_sources(
            &db,
            fixture_sync_request("US", "2026-01-01", "2026-01-09"),
            &sources,
        )
        .await;

        assert_eq!(outcome.status, CalendarSyncStatus::Published);
        assert_eq!(
            coverage_bounds(&db, "US"),
            ("2026-01-01".into(), "2026-01-09".into())
        );
        assert!(source
            .calls()
            .iter()
            .all(|(_, start, end)| { *start == day("2026-01-01") && *end == day("2026-01-04") }));
    }

    #[tokio::test]
    async fn resource_revision_change_revalidates_the_full_union() {
        let db = Database::new(":memory:").unwrap();
        let old = fixture_validated_range("2026-01-03", "2026-01-07", "old-rules");
        publish_validated_calendar(&db, &old, None).unwrap();
        let source = Arc::new(CountingFakeSource::complete());
        let sources: Vec<Arc<dyn IndexHistorySource>> = vec![source.clone()];

        let outcome = sync_market_calendar_with_sources(
            &db,
            fixture_sync_request("US", "2026-01-05", "2026-01-09"),
            &sources,
        )
        .await;

        assert_eq!(outcome.status, CalendarSyncStatus::Published);
        assert_eq!(
            coverage_bounds(&db, "US"),
            ("2026-01-03".into(), "2026-01-09".into())
        );
        assert!(source
            .calls()
            .iter()
            .all(|(_, start, end)| { *start == day("2026-01-03") && *end == day("2026-01-09") }));
    }

    #[tokio::test]
    async fn same_revision_reuses_without_updating_calendar_timestamps() {
        let db = Database::new(":memory:").unwrap();
        let source = Arc::new(CountingFakeSource::complete());
        let sources: Vec<Arc<dyn IndexHistorySource>> = vec![source.clone()];
        let first = sync_market_calendar_with_sources(&db, fixture_request_a(), &sources).await;
        assert_eq!(first.status, CalendarSyncStatus::Published);
        let before = calendar_timestamps(&db, "US");

        let second = sync_market_calendar_with_sources(&db, fixture_request_a(), &sources).await;

        assert_eq!(second.status, CalendarSyncStatus::Reused);
        assert_eq!(calendar_timestamps(&db, "US"), before);
        assert_eq!(source.calls().len(), 2);
        assert_eq!(second.warnings.len(), 1);
        assert_eq!(second.warnings[0].code, "market_calendar_single_provider");
    }

    #[tokio::test]
    async fn same_revision_missing_natural_day_is_revalidated_and_republished() {
        let db = Database::new(":memory:").unwrap();
        let source = Arc::new(CountingFakeSource::complete());
        let sources: Vec<Arc<dyn IndexHistorySource>> = vec![source.clone()];
        sync_market_calendar_with_sources(&db, fixture_request_a(), &sources).await;
        source.clear_calls();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "DELETE FROM stock_market_sessions WHERE market = 'US' AND date = '2026-01-06'",
                [],
            )
            .unwrap();

        let outcome = sync_market_calendar_with_sources(&db, fixture_request_a(), &sources).await;

        assert_eq!(outcome.status, CalendarSyncStatus::Published);
        assert_eq!(source.calls().len(), 2);
        assert_calendar_rows_match_coverage(&db, "US");
        assert_eq!(session_flag(&db, "US", "2026-01-06"), 1);
    }

    #[tokio::test]
    async fn same_revision_altered_session_flag_is_revalidated_and_republished() {
        let db = Database::new(":memory:").unwrap();
        let source = Arc::new(CountingFakeSource::complete());
        let sources: Vec<Arc<dyn IndexHistorySource>> = vec![source.clone()];
        sync_market_calendar_with_sources(&db, fixture_request_a(), &sources).await;
        source.clear_calls();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE stock_market_sessions SET is_session = 0 WHERE market = 'US' AND date = '2026-01-06'",
                [],
            )
            .unwrap();

        let outcome = sync_market_calendar_with_sources(&db, fixture_request_a(), &sources).await;

        assert_eq!(outcome.status, CalendarSyncStatus::Published);
        assert_eq!(source.calls().len(), 2);
        assert_eq!(session_flag(&db, "US", "2026-01-06"), 1);
    }

    #[tokio::test]
    async fn same_revision_non_finalized_coverage_is_revalidated_and_republished() {
        let db = Database::new(":memory:").unwrap();
        let source = Arc::new(CountingFakeSource::complete());
        let sources: Vec<Arc<dyn IndexHistorySource>> = vec![source.clone()];
        sync_market_calendar_with_sources(&db, fixture_request_a(), &sources).await;
        source.clear_calls();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE stock_market_calendar_coverage SET encodes_closed_dates = 0 WHERE market = 'US'",
                [],
            )
            .unwrap();

        let outcome = sync_market_calendar_with_sources(&db, fixture_request_a(), &sources).await;

        assert_eq!(outcome.status, CalendarSyncStatus::Published);
        assert_eq!(source.calls().len(), 2);
        assert_eq!(coverage_flag(&db, "US"), 1);
    }

    #[tokio::test]
    async fn same_revision_inconsistent_source_and_bounds_are_revalidated_and_republished() {
        let db = Database::new(":memory:").unwrap();
        let source = Arc::new(CountingFakeSource::complete());
        let sources: Vec<Arc<dyn IndexHistorySource>> = vec![source.clone()];
        sync_market_calendar_with_sources(&db, fixture_request_a(), &sources).await;
        source.clear_calls();
        db.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE stock_market_calendar_coverage
                 SET source = 'xueqiu', complete_start = '2026-01-02'
                 WHERE market = 'US'",
                [],
            )
            .unwrap();

        let outcome = sync_market_calendar_with_sources(&db, fixture_request_a(), &sources).await;

        assert_eq!(outcome.status, CalendarSyncStatus::Published);
        assert_eq!(source.calls().len(), 2);
        assert_eq!(coverage_source(&db, "US"), "eastmoney");
        assert_eq!(
            coverage_bounds(&db, "US"),
            ("2026-01-01".into(), "2026-01-09".into())
        );
        assert_calendar_rows_match_coverage(&db, "US");
    }

    #[tokio::test]
    async fn disjoint_single_provider_segments_do_not_publish_false_combined_provenance() {
        let db = Database::new(":memory:").unwrap();
        let initial = Arc::new(RangeBoundedFakeSource::new(
            CalendarProvider::Xueqiu,
            "2026-01-05",
            "2026-01-07",
        ));
        let initial_sources: Vec<Arc<dyn IndexHistorySource>> = vec![initial];
        let first = sync_market_calendar_with_sources(
            &db,
            fixture_sync_request("US", "2026-01-05", "2026-01-07"),
            &initial_sources,
        )
        .await;
        assert_eq!(first.status, CalendarSyncStatus::Published);
        assert_eq!(coverage_source(&db, "US"), "xueqiu");
        let before_revision = coverage_revision(&db, "US");

        let xueqiu = Arc::new(RangeBoundedFakeSource::new(
            CalendarProvider::Xueqiu,
            "2026-01-05",
            "2026-01-07",
        ));
        let eastmoney = Arc::new(RangeBoundedFakeSource::new(
            CalendarProvider::EastMoney,
            "2026-01-08",
            "2026-01-09",
        ));
        let extension_sources: Vec<Arc<dyn IndexHistorySource>> =
            vec![xueqiu.clone(), eastmoney.clone()];

        let outcome = sync_market_calendar_with_sources(
            &db,
            fixture_sync_request("US", "2026-01-05", "2026-01-09"),
            &extension_sources,
        )
        .await;

        assert_eq!(outcome.status, CalendarSyncStatus::StaleCacheUsed);
        assert_eq!(coverage_source(&db, "US"), "xueqiu");
        assert_eq!(coverage_revision(&db, "US"), before_revision);
        assert_eq!(
            coverage_bounds(&db, "US"),
            ("2026-01-05".into(), "2026-01-07".into())
        );
        assert!(xueqiu
            .calls()
            .iter()
            .any(|(_, start, end)| *start == day("2026-01-05") && *end == day("2026-01-09")));
        assert!(eastmoney
            .calls()
            .iter()
            .any(|(_, start, end)| *start == day("2026-01-05") && *end == day("2026-01-09")));
    }

    #[tokio::test]
    async fn request_outside_resource_range_preserves_old_cache_with_structured_reason() {
        let db = Database::new(":memory:").unwrap();
        publish_validated_calendar(&db, &fixture_validated_a(), None).unwrap();
        let revision = coverage_revision(&db, "US");
        let sources = complete_fake_sources();

        let stale = sync_market_calendar_with_sources(
            &db,
            fixture_sync_request("US", "2026-01-01", "2027-01-02"),
            &sources,
        )
        .await;

        assert_eq!(stale.status, CalendarSyncStatus::StaleCacheUsed);
        assert_eq!(
            stale.issue_code.as_deref(),
            Some("market_calendar_refresh_failed")
        );
        assert!(stale.message.as_deref().unwrap().contains("outside"));
        assert_eq!(coverage_revision(&db, "US"), revision);

        let empty = Database::new(":memory:").unwrap();
        let unavailable = sync_market_calendar_with_sources(
            &empty,
            fixture_sync_request("US", "2026-01-01", "2027-01-02"),
            &sources,
        )
        .await;
        assert_eq!(unavailable.status, CalendarSyncStatus::Unavailable);
        assert_eq!(
            unavailable.issue_code.as_deref(),
            Some("market_calendar_sync_failed")
        );
    }

    #[tokio::test]
    async fn optimistic_revision_mismatch_retries_at_most_once_and_preserves_old_rows() {
        let db = Arc::new(Database::new(":memory:").unwrap());
        publish_validated_calendar(&db, &fixture_validated_a(), None).unwrap();
        let source = Arc::new(RevisionThrashingSource {
            db: db.clone(),
            fetch_count: AtomicUsize::new(0),
        });
        let sources: Vec<Arc<dyn IndexHistorySource>> = vec![source.clone()];

        let outcome = sync_market_calendar_with_sources(&db, extended_request(), &sources).await;

        assert_eq!(outcome.status, CalendarSyncStatus::StaleCacheUsed);
        assert_eq!(
            outcome.issue_code.as_deref(),
            Some("market_calendar_refresh_failed")
        );
        assert_eq!(source.fetch_count.load(Ordering::SeqCst), 4);
        assert_eq!(row_count(&db, "stock_market_sessions", "US"), 7);
    }

    #[test]
    fn revision_is_independent_of_provider_and_date_input_order() {
        let left = ValidatedMarketCalendar {
            market: "US".to_owned(),
            start: day("2026-01-01"),
            end: day("2026-01-03"),
            resource_revision: "holidays-v1".to_owned(),
            rows: vec![
                (day("2026-01-01"), false),
                (day("2026-01-02"), true),
                (day("2026-01-03"), false),
            ],
            providers: vec!["xueqiu".to_owned(), "eastmoney".to_owned()],
            references: vec!["sp500".to_owned(), "nasdaq_composite".to_owned()],
            warnings: Vec::new(),
        };
        let mut reordered = left.clone();
        reordered.providers = vec![
            "eastmoney".to_owned(),
            "xueqiu".to_owned(),
            "eastmoney".to_owned(),
        ];
        reordered.references = vec![
            "nasdaq_composite".to_owned(),
            "sp500".to_owned(),
            "nasdaq_composite".to_owned(),
        ];
        reordered.rows = vec![
            (day("2026-01-03"), false),
            (day("2026-01-02"), true),
            (day("2026-01-01"), false),
            (day("2026-01-02"), true),
        ];

        assert_eq!(
            stable_calendar_revision(&left),
            stable_calendar_revision(&reordered)
        );

        let mut single_provider = left.clone();
        single_provider.providers = vec!["eastmoney".to_owned()];
        assert_ne!(
            stable_calendar_revision(&left),
            stable_calendar_revision(&single_provider)
        );
    }
}
