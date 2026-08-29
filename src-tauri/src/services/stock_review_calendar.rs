use chrono::{DateTime, Datelike, NaiveDate, NaiveTime, Utc, Weekday};
use chrono_tz::{America::New_York, Asia::Hong_Kong, Asia::Shanghai, Tz};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

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
    use super::{latest_fully_closed_date, load_market_holiday_rules, parse_market_holiday_bundle};
    use chrono::{NaiveDate, TimeZone, Utc};

    fn day(value: &str) -> NaiveDate {
        NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
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
}
