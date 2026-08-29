use crate::services::quote_service::{self, XueqiuHistoryOutcome};
use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, Utc, Weekday};
use chrono_tz::{America::New_York, Asia::Hong_Kong, Asia::Shanghai, Tz};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

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
    let warnings = if providers.len() == 1 {
        vec![CalendarSyncWarning {
            code: "market_calendar_single_provider".to_owned(),
            message: format!(
                "{} market calendar was validated with only the {} provider.",
                request.market, providers[0]
            ),
        }]
    } else {
        Vec::new()
    };
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
    for (date, is_session) in &calendar.rows {
        digest.write(&date.format("%Y-%m-%d").to_string());
        digest.write(if *is_session { "1" } else { "0" });
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

#[cfg(test)]
mod tests {
    use super::{
        latest_fully_closed_date, load_market_holiday_rules, parse_market_holiday_bundle,
        stable_calendar_revision, validate_market_calendar, CalendarProvider,
        CalendarValidationErrorKind, CalendarValidationRequest, IndexHistoryEvidence,
        IndexHistorySource, ReferenceIndex, ValidatedMarketCalendar,
    };
    use chrono::{NaiveDate, TimeZone, Utc};
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

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
