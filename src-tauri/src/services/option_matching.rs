use chrono::{NaiveDate, NaiveDateTime};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone)]
pub(crate) struct MatchRecord {
    pub id: String,
    pub option_symbol: String,
    pub underlying: String,
    pub expiry_date: String,
    pub strike_price: f64,
    pub option_type: String,
    pub action: String,
    pub code: String,
    pub quantity: i64,
    pub traded_at: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct SplitRecord {
    pub stock_code: String,
    pub split_date: String,
    pub ratio_from: i64,
    pub ratio_to: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct MatchAllocation {
    pub open_id: String,
    pub close_id: String,
    pub quantity: i64,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct MatchResult {
    pub allocations: Vec<MatchAllocation>,
    pub remaining_open: HashMap<String, i64>,
    pub unmatched_close_ids: HashSet<String>,
}

pub(crate) fn parse_trade_date(raw: &str) -> Option<NaiveDate> {
    let date = raw.trim().split([',', ' ']).next()?;
    ["%Y-%m-%d", "%Y/%m/%d", "%d%b%y"]
        .iter()
        .find_map(|format| NaiveDate::parse_from_str(date, format).ok())
        .or_else(|| {
            let serial = date.parse::<i64>().ok()?;
            (serial > 0).then_some(())?;
            NaiveDate::from_ymd_opt(1899, 12, 30)?
                .checked_add_signed(chrono::Duration::days(serial))
        })
}

pub(crate) fn parse_trade_timestamp(raw: &str) -> Option<NaiveDateTime> {
    let raw = raw.trim();
    [
        "%Y-%m-%d, %H:%M:%S",
        "%Y-%m-%d, %H:%M",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y/%m/%d, %H:%M:%S",
        "%Y/%m/%d, %H:%M",
        "%Y/%m/%d %H:%M:%S",
        "%Y/%m/%d %H:%M",
        "%d%b%y, %H:%M:%S",
        "%d%b%y, %H:%M",
        "%d%b%y %H:%M:%S",
        "%d%b%y %H:%M",
    ]
    .iter()
    .find_map(|format| NaiveDateTime::parse_from_str(raw, format).ok())
    .or_else(|| parse_trade_date(raw)?.and_hms_opt(0, 0, 0))
}

pub(crate) fn match_options_fifo(records: &[MatchRecord], splits: &[SplitRecord]) -> MatchResult {
    #[derive(Clone)]
    struct OpenState {
        record: MatchRecord,
        remaining: i64,
    }

    fn is_open(record: &MatchRecord) -> bool {
        record.action == "SELL" && record.code.starts_with('O')
    }

    fn is_close(record: &MatchRecord) -> bool {
        record.action == "BUY" && matches!(record.code.as_str(), "C" | "C;Ep" | "A;C" | "C;P")
    }

    fn split_matches(open: &MatchRecord, close: &MatchRecord, splits: &[SplitRecord]) -> bool {
        if open.underlying != close.underlying
            || open.expiry_date != close.expiry_date
            || open.option_type != close.option_type
        {
            return false;
        }
        let Some(expiry) = parse_trade_date(&close.expiry_date) else {
            return false;
        };
        let (Some(opened_at), Some(closed_at)) = (
            open.traded_at.as_deref().and_then(parse_trade_date),
            close.traded_at.as_deref().and_then(parse_trade_date),
        ) else {
            return false;
        };

        splits.iter().any(|split| {
            let Some(split_date) = parse_trade_date(&split.split_date) else {
                return false;
            };
            if split.stock_code != open.underlying
                || split_date <= opened_at
                || split_date > closed_at
                || split_date > expiry
                || split.ratio_from <= 0
                || split.ratio_to <= 0
            {
                return false;
            }
            let ratio = split.ratio_to as f64 / split.ratio_from as f64;
            let expected_strike = open.strike_price / ratio;
            expected_strike > 0.0
                && (close.strike_price - expected_strike).abs() / expected_strike <= 0.02
        })
    }

    let mut ordered: Vec<_> = records
        .iter()
        .filter(|record| record.quantity.checked_abs().unwrap_or(0) > 0)
        .filter(|record| is_open(record) || is_close(record))
        .cloned()
        .collect();
    ordered.sort_by(|left, right| {
        left.traded_at
            .as_deref()
            .and_then(parse_trade_timestamp)
            .cmp(&right.traded_at.as_deref().and_then(parse_trade_timestamp))
            .then_with(|| match (left.action.as_str(), right.action.as_str()) {
                ("SELL", "BUY") => std::cmp::Ordering::Less,
                ("BUY", "SELL") => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut opens: Vec<OpenState> = Vec::new();
    let mut result = MatchResult::default();
    for record in ordered {
        let quantity = record.quantity.checked_abs().unwrap_or(0);
        if is_open(&record) {
            result.remaining_open.insert(record.id.clone(), quantity);
            opens.push(OpenState {
                record,
                remaining: quantity,
            });
            continue;
        }

        let mut remaining_close = quantity;
        for split_phase in [false, true] {
            for open in &mut opens {
                if remaining_close == 0 {
                    break;
                }
                if open.remaining == 0 {
                    continue;
                }
                let matches = if split_phase {
                    open.record.option_symbol != record.option_symbol
                        && split_matches(&open.record, &record, splits)
                } else {
                    open.record.option_symbol == record.option_symbol
                };
                if !matches {
                    continue;
                }

                let matched = open.remaining.min(remaining_close);
                open.remaining -= matched;
                remaining_close -= matched;
                result
                    .remaining_open
                    .insert(open.record.id.clone(), open.remaining);
                result.allocations.push(MatchAllocation {
                    open_id: open.record.id.clone(),
                    close_id: record.id.clone(),
                    quantity: matched,
                });
            }
        }
        if remaining_close > 0 {
            result.unmatched_close_ids.insert(record.id);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        id: &str,
        symbol: &str,
        action: &str,
        code: &str,
        quantity: i64,
        date: &str,
    ) -> MatchRecord {
        MatchRecord {
            id: id.to_string(),
            option_symbol: symbol.to_string(),
            underlying: "ACME".to_string(),
            expiry_date: "20FEB26".to_string(),
            strike_price: symbol.split_whitespace().nth(2).unwrap().parse().unwrap(),
            option_type: "P".to_string(),
            action: action.to_string(),
            code: code.to_string(),
            quantity,
            traded_at: Some(date.to_string()),
        }
    }

    #[test]
    fn exact_contract_matching_is_fifo_and_conserves_close_quantity() {
        let records = vec![
            record(
                "open-1",
                "ACME 20FEB26 100 P",
                "SELL",
                "O",
                2,
                "2026-01-01 09:00",
            ),
            record(
                "open-2",
                "ACME 20FEB26 100 P",
                "SELL",
                "O",
                2,
                "2026-01-02 09:00",
            ),
            record(
                "close",
                "ACME 20FEB26 100 P",
                "BUY",
                "C",
                3,
                "2026-01-03 09:00",
            ),
        ];

        let result = match_options_fifo(&records, &[]);

        assert_eq!(
            result.allocations,
            vec![
                MatchAllocation {
                    open_id: "open-1".into(),
                    close_id: "close".into(),
                    quantity: 2
                },
                MatchAllocation {
                    open_id: "open-2".into(),
                    close_id: "close".into(),
                    quantity: 1
                },
            ]
        );
        assert_eq!(result.remaining_open.get("open-1"), Some(&0));
        assert_eq!(result.remaining_open.get("open-2"), Some(&1));
        assert!(result.unmatched_close_ids.is_empty());
    }

    #[test]
    fn split_matching_requires_split_inside_open_close_window() {
        let mut open = record("open", "ACME 20FEB26 100 P", "SELL", "O", 1, "2026-01-10");
        let mut close = record("close", "ACME 20FEB26 50 P", "BUY", "C", 1, "2026-01-20");
        open.strike_price = 100.0;
        close.strike_price = 50.0;
        let split = |date: &str| SplitRecord {
            stock_code: "ACME".into(),
            split_date: date.into(),
            ratio_from: 1,
            ratio_to: 2,
        };

        assert_eq!(
            match_options_fifo(&[open.clone(), close.clone()], &[split("2026-01-15")])
                .allocations
                .len(),
            1
        );
        assert!(
            match_options_fifo(&[open.clone(), close.clone()], &[split("2026-01-10")])
                .allocations
                .is_empty()
        );
        assert!(match_options_fifo(&[open, close], &[split("2026-01-21")])
            .allocations
            .is_empty());
    }

    #[test]
    fn one_split_close_cannot_be_reused_for_multiple_opens() {
        let mut first = record("open-1", "ACME 20FEB26 100 P", "SELL", "O", 1, "2026-01-01");
        let mut second = record("open-2", "ACME 20FEB26 100 P", "SELL", "O", 1, "2026-01-02");
        let mut close = record("close", "ACME 20FEB26 50 P", "BUY", "C", 1, "2026-01-20");
        first.strike_price = 100.0;
        second.strike_price = 100.0;
        close.strike_price = 50.0;
        let result = match_options_fifo(
            &[first, second, close],
            &[SplitRecord {
                stock_code: "ACME".into(),
                split_date: "2026-01-15".into(),
                ratio_from: 1,
                ratio_to: 2,
            }],
        );

        assert_eq!(result.allocations.len(), 1);
        assert_eq!(result.allocations[0].open_id, "open-1");
        assert_eq!(result.remaining_open.get("open-2"), Some(&1));
    }
}
