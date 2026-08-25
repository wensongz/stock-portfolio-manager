use crate::db::Database;
use crate::models::{
    OptionCampaign, OptionReviewDataQuality, OptionReviewReport, OptionReviewSummary,
    OptionUnderlyingReview, OptionWorstCampaign,
};
use chrono::{NaiveDate, NaiveDateTime, Utc};
use std::collections::{BTreeMap, HashMap, HashSet};

#[derive(Debug, Clone)]
struct RawOptionRecord {
    id: String,
    option_symbol: String,
    underlying: String,
    expiry_date: String,
    strike_price: f64,
    option_type: String,
    action: String,
    code: String,
    quantity: i64,
    amount: f64,
    commission: f64,
    fee: f64,
    traded_at: Option<String>,
    trade_date: Option<NaiveDate>,
    trade_timestamp: Option<NaiveDateTime>,
}

#[derive(Debug, Clone)]
struct OpenLot {
    record: RawOptionRecord,
    original_quantity: i64,
    remaining_quantity: ContractQuantity,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ContractQuantity {
    numerator: i128,
    denominator: i128,
}

impl ContractQuantity {
    fn new(numerator: i128, denominator: i128) -> Option<Self> {
        if numerator < 0 || denominator <= 0 {
            return None;
        }
        if numerator == 0 {
            return Some(Self {
                numerator: 0,
                denominator: 1,
            });
        }
        let divisor = greatest_common_divisor(numerator, denominator);
        Some(Self {
            numerator: numerator / divisor,
            denominator: denominator / divisor,
        })
    }

    fn from_i64(value: i64) -> Self {
        Self::new(i128::from(value), 1).expect("positive option record quantity")
    }

    fn is_positive(self) -> bool {
        self.numerator > 0
    }

    fn checked_min(self, other: Self) -> Option<Self> {
        let left = self.numerator.checked_mul(other.denominator)?;
        let right = other.numerator.checked_mul(self.denominator)?;
        Some(if left <= right { self } else { other })
    }

    fn checked_sub(self, other: Self) -> Option<Self> {
        let divisor = greatest_common_divisor(self.denominator, other.denominator);
        let self_scale = other.denominator / divisor;
        let other_scale = self.denominator / divisor;
        let numerator = self
            .numerator
            .checked_mul(self_scale)?
            .checked_sub(other.numerator.checked_mul(other_scale)?)?;
        let denominator = self.denominator.checked_mul(self_scale)?;
        Self::new(numerator, denominator)
    }

    fn checked_mul_ratio(self, numerator: i64, denominator: i64) -> Option<Self> {
        if numerator <= 0 || denominator <= 0 {
            return None;
        }
        let mut left_numerator = self.numerator;
        let mut left_denominator = self.denominator;
        let mut right_numerator = i128::from(numerator);
        let mut right_denominator = i128::from(denominator);

        let first_divisor = greatest_common_divisor(left_numerator, right_denominator);
        left_numerator /= first_divisor;
        right_denominator /= first_divisor;
        let second_divisor = greatest_common_divisor(right_numerator, left_denominator);
        right_numerator /= second_divisor;
        left_denominator /= second_divisor;

        Self::new(
            left_numerator.checked_mul(right_numerator)?,
            left_denominator.checked_mul(right_denominator)?,
        )
    }

    fn as_f64(self) -> f64 {
        self.numerator as f64 / self.denominator as f64
    }
}

fn greatest_common_divisor(mut left: i128, mut right: i128) -> i128 {
    while right != 0 {
        let remainder = left % right;
        left = right;
        right = remainder;
    }
    left
}

#[derive(Debug, Clone)]
struct OptionCycle {
    underlying: String,
    option_type: String,
    opened_at: NaiveDate,
    ended_at: Option<NaiveDate>,
    status: String,
    gross_premium: f64,
    close_cost: f64,
    fees: f64,
    secured_notional: f64,
    capital_days: f64,
}

impl OptionCycle {
    fn effective_end(&self) -> NaiveDate {
        self.ended_at.unwrap_or(NaiveDate::MAX)
    }
}

#[derive(Debug, Clone)]
struct SplitInput {
    stock_code: String,
    split_date: NaiveDate,
    ratio_from: i64,
    ratio_to: i64,
}

#[derive(Debug, Clone, Copy)]
struct SplitRatio {
    from: i64,
    to: i64,
}

pub fn get_option_review(
    db: &Database,
    account_id: &str,
    period_days: Option<i64>,
) -> Result<OptionReviewReport, String> {
    get_option_review_at(db, account_id, period_days, Utc::now().date_naive())
}

fn get_option_review_at(
    db: &Database,
    account_id: &str,
    period_days: Option<i64>,
    today: NaiveDate,
) -> Result<OptionReviewReport, String> {
    let period_days = period_days.map(|days| days.clamp(1, 3650));
    let (market, records, share_lots, splits) = load_inputs(db, account_id)?;
    let currency = match market.as_str() {
        "CN" => "CNY",
        "HK" => "HKD",
        _ => "USD",
    }
    .to_string();
    let (cycles, mut quality) = pair_cycles_fifo(records, &share_lots, &splits, today);
    let campaigns = group_campaigns(account_id, cycles, today);
    let filtered = filter_campaigns(campaigns, period_days, today);
    quality.excluded_open_campaigns = filtered
        .iter()
        .filter(|campaign| campaign.status == "active")
        .count();
    quality.notes = data_quality_notes(&quality);
    Ok(build_report(
        account_id,
        currency,
        period_days,
        today,
        filtered,
        quality,
    ))
}

type LoadedInputs = (
    String,
    Vec<RawOptionRecord>,
    HashMap<String, i64>,
    Vec<SplitInput>,
);

fn load_inputs(db: &Database, account_id: &str) -> Result<LoadedInputs, String> {
    let conn = db.conn.lock().map_err(|error| error.to_string())?;
    let market = conn
        .query_row(
            "SELECT market FROM accounts WHERE id = ?1",
            [account_id],
            |row| row.get::<_, String>(0),
        )
        .map_err(|error| match error {
            rusqlite::Error::QueryReturnedNoRows => {
                format!("Account not found: {account_id}")
            }
            other => other.to_string(),
        })?;

    let mut statement = conn
        .prepare(
            "SELECT id, option_symbol, underlying, expiry_date, strike_price,
                    option_type, action, code, quantity, amount, commission, fee, traded_at
             FROM option_records WHERE account_id = ?1",
        )
        .map_err(|error| error.to_string())?;
    let records = statement
        .query_map([account_id], |row| {
            let traded_at = row.get::<_, Option<String>>(12)?;
            let trade_timestamp = traded_at.as_deref().and_then(parse_trade_timestamp);
            Ok(RawOptionRecord {
                id: row.get(0)?,
                option_symbol: row.get(1)?,
                underlying: row.get(2)?,
                expiry_date: row.get(3)?,
                strike_price: row.get(4)?,
                option_type: row.get(5)?,
                action: row.get(6)?,
                code: row.get(7)?,
                quantity: row.get::<_, i64>(8)?.abs(),
                amount: row.get(9)?,
                commission: row.get(10)?,
                fee: row.get(11)?,
                trade_date: traded_at.as_deref().and_then(parse_trade_date),
                trade_timestamp,
                traded_at,
            })
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    let mut share_statement = conn
        .prepare("SELECT stock_code, shares_per_contract FROM option_share_lots")
        .map_err(|error| error.to_string())?;
    let share_lots = share_statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| error.to_string())?
        .collect::<Result<HashMap<_, _>, _>>()
        .map_err(|error| error.to_string())?;

    let mut split_statement = conn
        .prepare("SELECT stock_code, split_date, ratio_from, ratio_to FROM stock_splits")
        .map_err(|error| error.to_string())?;
    let splits = split_statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
            ))
        })
        .map_err(|error| error.to_string())?
        .filter_map(|result| match result {
            Ok((stock_code, split_date, ratio_from, ratio_to)) => parse_trade_date(&split_date)
                .map(|split_date| {
                    Ok(SplitInput {
                        stock_code,
                        split_date,
                        ratio_from,
                        ratio_to,
                    })
                }),
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| error.to_string())?;

    Ok((market, records, share_lots, splits))
}

fn parse_trade_date(raw: &str) -> Option<NaiveDate> {
    let date = raw.trim().split([',', ' ']).next()?;
    ["%Y-%m-%d", "%Y/%m/%d", "%d%b%y"]
        .iter()
        .find_map(|format| NaiveDate::parse_from_str(date, format).ok())
}

fn parse_trade_timestamp(raw: &str) -> Option<NaiveDateTime> {
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

fn parse_expiry_date(raw: &str) -> Option<NaiveDate> {
    parse_trade_date(raw)
}

fn safe_ratio(numerator: f64, denominator: f64) -> Option<f64> {
    (denominator.abs() > f64::EPSILON).then_some(numerator / denominator)
}

fn is_open(record: &RawOptionRecord) -> bool {
    record.action == "SELL" && record.code.starts_with('O')
}

fn is_close(record: &RawOptionRecord) -> bool {
    record.action == "BUY" && matches!(record.code.as_str(), "C" | "C;Ep" | "A;C" | "C;P")
}

fn close_status(code: &str) -> &'static str {
    match code {
        "A;C" => "assigned",
        "C" | "C;P" => "closed",
        _ => "expired",
    }
}

fn split_ratio(
    open: &OpenLot,
    close: &RawOptionRecord,
    splits: &[SplitInput],
) -> Option<SplitRatio> {
    if open.record.underlying != close.underlying
        || open.record.expiry_date != close.expiry_date
        || open.record.option_type != close.option_type
    {
        return None;
    }
    let Some(expiry) = parse_expiry_date(&close.expiry_date) else {
        return None;
    };
    let (Some(opened_at), Some(closed_at)) = (open.record.trade_date, close.trade_date) else {
        return None;
    };
    splits.iter().find_map(|split| {
        if split.stock_code != open.record.underlying
            || split.split_date <= opened_at
            || split.split_date > closed_at
            || split.split_date > expiry
            || split.ratio_from <= 0
            || split.ratio_to <= 0
        {
            return None;
        }
        let ratio = split.ratio_to as f64 / split.ratio_from as f64;
        let expected_strike = open.record.strike_price / ratio;
        (expected_strike > 0.0
            && (close.strike_price - expected_strike).abs() / expected_strike <= 0.02)
            .then_some(SplitRatio {
                from: split.ratio_from,
                to: split.ratio_to,
            })
    })
}

fn matched_quantities(
    open_remaining: ContractQuantity,
    close_remaining: ContractQuantity,
    ratio: SplitRatio,
) -> Option<(ContractQuantity, ContractQuantity)> {
    let close_in_open_contracts = close_remaining.checked_mul_ratio(ratio.from, ratio.to)?;
    let matched_open = open_remaining.checked_min(close_in_open_contracts)?;
    let matched_close = matched_open.checked_mul_ratio(ratio.to, ratio.from)?;
    Some((matched_open, matched_close))
}

fn cycle_from_match(
    open: &OpenLot,
    close: Option<&RawOptionRecord>,
    matched_open: ContractQuantity,
    matched_close: ContractQuantity,
    share_lots: &HashMap<String, i64>,
    today: NaiveDate,
) -> OptionCycle {
    let matched_open = matched_open.as_f64();
    let open_fraction = matched_open / open.original_quantity as f64;
    let close_fraction = close
        .map(|record| matched_close.as_f64() / record.quantity as f64)
        .unwrap_or(0.0);
    let opened_at = open.record.trade_date.expect("validated open trade date");
    let ended_at = close.and_then(|record| record.trade_date);
    let effective_end = ended_at.unwrap_or(today);
    let holding_days = (effective_end - opened_at).num_days().max(1) as f64;
    let shares_per_contract = share_lots
        .get(&open.record.underlying)
        .copied()
        .unwrap_or(100)
        .abs() as f64;
    let secured_notional = matched_open * shares_per_contract * open.record.strike_price.abs();
    let close_cost = close
        .map(|record| record.amount * close_fraction)
        .unwrap_or(0.0);
    let fees = (open.record.commission.abs() + open.record.fee.abs()) * open_fraction
        + close
            .map(|record| (record.commission.abs() + record.fee.abs()) * close_fraction)
            .unwrap_or(0.0);
    OptionCycle {
        underlying: open.record.underlying.clone(),
        option_type: open.record.option_type.clone(),
        opened_at,
        ended_at,
        status: close
            .map(|record| close_status(&record.code).to_string())
            .unwrap_or_else(|| "active".to_string()),
        gross_premium: open.record.amount * open_fraction,
        close_cost,
        fees,
        secured_notional,
        capital_days: secured_notional * holding_days,
    }
}

fn pair_cycles_fifo(
    records: Vec<RawOptionRecord>,
    share_lots: &HashMap<String, i64>,
    splits: &[SplitInput],
    today: NaiveDate,
) -> (Vec<OptionCycle>, OptionReviewDataQuality) {
    let missing_trade_dates = records
        .iter()
        .filter(|record| record.traded_at.is_none() || record.trade_date.is_none())
        .count();
    let mut valid: Vec<_> = records
        .into_iter()
        .filter(|record| record.trade_date.is_some() && record.quantity > 0)
        .filter(|record| is_open(record) || is_close(record))
        .collect();
    valid.sort_by(|left, right| {
        left.trade_timestamp
            .cmp(&right.trade_timestamp)
            .then_with(|| match (left.action.as_str(), right.action.as_str()) {
                ("SELL", "BUY") => std::cmp::Ordering::Less,
                ("BUY", "SELL") => std::cmp::Ordering::Greater,
                _ => std::cmp::Ordering::Equal,
            })
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut opens: Vec<OpenLot> = Vec::new();
    let mut cycles = Vec::new();
    let mut unmatched_close_ids = HashSet::new();
    'record_loop: for record in valid {
        if is_open(&record) {
            opens.push(OpenLot {
                original_quantity: record.quantity,
                remaining_quantity: ContractQuantity::from_i64(record.quantity),
                record,
            });
            continue;
        }

        let mut remaining_quantity = ContractQuantity::from_i64(record.quantity);
        let exact_indices: Vec<_> = opens
            .iter()
            .enumerate()
            .filter(|(_, open)| {
                open.remaining_quantity.is_positive()
                    && open.record.option_symbol == record.option_symbol
            })
            .map(|(index, _)| index)
            .collect();
        for index in exact_indices {
            if !remaining_quantity.is_positive() {
                break;
            }
            let open = &mut opens[index];
            let Some((matched_open, matched_close)) = matched_quantities(
                open.remaining_quantity,
                remaining_quantity,
                SplitRatio { from: 1, to: 1 },
            ) else {
                unmatched_close_ids.insert(record.id.clone());
                continue 'record_loop;
            };
            let (Some(next_open_remaining), Some(next_close_remaining)) = (
                open.remaining_quantity.checked_sub(matched_open),
                remaining_quantity.checked_sub(matched_close),
            ) else {
                unmatched_close_ids.insert(record.id.clone());
                continue 'record_loop;
            };
            cycles.push(cycle_from_match(
                open,
                Some(&record),
                matched_open,
                matched_close,
                share_lots,
                today,
            ));
            open.remaining_quantity = next_open_remaining;
            remaining_quantity = next_close_remaining;
        }

        if remaining_quantity.is_positive() {
            let candidates: Vec<_> = opens
                .iter()
                .enumerate()
                .filter_map(|(index, open)| {
                    (open.remaining_quantity.is_positive())
                        .then(|| split_ratio(open, &record, splits))
                        .flatten()
                        .map(|ratio| (index, ratio))
                })
                .collect();
            if candidates.len() == 1 {
                let (index, ratio) = candidates[0];
                let open = &mut opens[index];
                let Some((matched_open, matched_close)) =
                    matched_quantities(open.remaining_quantity, remaining_quantity, ratio)
                else {
                    unmatched_close_ids.insert(record.id.clone());
                    continue 'record_loop;
                };
                let (Some(next_open_remaining), Some(next_close_remaining)) = (
                    open.remaining_quantity.checked_sub(matched_open),
                    remaining_quantity.checked_sub(matched_close),
                ) else {
                    unmatched_close_ids.insert(record.id.clone());
                    continue 'record_loop;
                };
                cycles.push(cycle_from_match(
                    open,
                    Some(&record),
                    matched_open,
                    matched_close,
                    share_lots,
                    today,
                ));
                open.remaining_quantity = next_open_remaining;
                remaining_quantity = next_close_remaining;
            }
        }

        if remaining_quantity.is_positive() {
            unmatched_close_ids.insert(record.id);
        }
    }

    for open in opens
        .iter()
        .filter(|open| open.remaining_quantity.is_positive())
    {
        cycles.push(cycle_from_match(
            open,
            None,
            open.remaining_quantity,
            ContractQuantity::from_i64(0),
            share_lots,
            today,
        ));
    }
    cycles.sort_by(|left, right| {
        left.underlying
            .cmp(&right.underlying)
            .then(left.opened_at.cmp(&right.opened_at))
            .then(left.option_type.cmp(&right.option_type))
    });
    (
        cycles,
        OptionReviewDataQuality {
            excluded_open_campaigns: 0,
            unmatched_records: unmatched_close_ids.len(),
            missing_trade_dates,
            notes: Vec::new(),
        },
    )
}

fn should_connect(previous: &OptionCycle, next: &OptionCycle) -> bool {
    if next.opened_at <= previous.effective_end() {
        return true;
    }
    let gap = (next.opened_at - previous.effective_end()).num_days();
    (previous.option_type == next.option_type && gap <= 7)
        || (previous.option_type == "P"
            && previous.status == "assigned"
            && next.option_type == "C"
            && gap <= 30)
}

fn campaign_from_cycles(
    account_id: &str,
    underlying: &str,
    ordinal: usize,
    cycles: &[OptionCycle],
) -> OptionCampaign {
    let started_at = cycles
        .iter()
        .map(|cycle| cycle.opened_at)
        .min()
        .expect("campaign contains a cycle");
    let active = cycles.iter().any(|cycle| cycle.status == "active");
    let ended_at = (!active)
        .then(|| cycles.iter().filter_map(|cycle| cycle.ended_at).max())
        .flatten();
    let gross_premium: f64 = cycles.iter().map(|cycle| cycle.gross_premium).sum();
    let close_cost: f64 = cycles.iter().map(|cycle| cycle.close_cost).sum();
    let fees: f64 = cycles.iter().map(|cycle| cycle.fees).sum();
    let secured_notional: f64 = cycles.iter().map(|cycle| cycle.secured_notional).sum();
    let capital_days: f64 = cycles.iter().map(|cycle| cycle.capital_days).sum();
    let net = (!active).then_some(gross_premium - close_cost - fees);
    let mut strategy_path = Vec::new();
    for cycle in cycles {
        let strategy = if cycle.option_type == "P" {
            "CSP"
        } else {
            "Covered Call"
        };
        if !strategy_path.iter().any(|existing| existing == strategy) {
            strategy_path.push(strategy.to_string());
        }
    }
    OptionCampaign {
        id: format!(
            "option-review:{account_id}:{underlying}:{}:{ordinal}",
            started_at.format("%Y-%m-%d")
        ),
        underlying: underlying.to_string(),
        started_at: started_at.format("%Y-%m-%d").to_string(),
        ended_at: ended_at.map(|date| date.format("%Y-%m-%d").to_string()),
        status: if active { "active" } else { "completed" }.to_string(),
        inferred: true,
        strategy_path,
        gross_premium,
        close_cost,
        fees,
        net_premium_pnl: net,
        secured_notional,
        capital_days,
        retention_rate: net.and_then(|value| safe_ratio(value, gross_premium)),
        annualized_yield_on_notional: net.and_then(|value| safe_ratio(value * 365.0, capital_days)),
    }
}

fn group_campaigns(
    account_id: &str,
    cycles: Vec<OptionCycle>,
    _today: NaiveDate,
) -> Vec<OptionCampaign> {
    let mut by_underlying: BTreeMap<String, Vec<OptionCycle>> = BTreeMap::new();
    for cycle in cycles {
        by_underlying
            .entry(cycle.underlying.clone())
            .or_default()
            .push(cycle);
    }
    let mut campaigns = Vec::new();
    for (underlying, mut underlying_cycles) in by_underlying {
        underlying_cycles.sort_by(|left, right| left.opened_at.cmp(&right.opened_at));
        let mut groups: Vec<Vec<OptionCycle>> = Vec::new();
        for next in underlying_cycles {
            if let Some(current_cycles) = groups.last_mut() {
                if current_cycles
                    .iter()
                    .any(|cycle| should_connect(cycle, &next))
                {
                    current_cycles.push(next);
                    continue;
                }
            }
            groups.push(vec![next]);
        }
        for (index, group) in groups.iter().enumerate() {
            campaigns.push(campaign_from_cycles(
                account_id,
                &underlying,
                index + 1,
                group,
            ));
        }
    }
    campaigns
}

fn filter_campaigns(
    campaigns: Vec<OptionCampaign>,
    period_days: Option<i64>,
    today: NaiveDate,
) -> Vec<OptionCampaign> {
    let Some(days) = period_days else {
        return campaigns;
    };
    let cutoff = today - chrono::Duration::days(days);
    campaigns
        .into_iter()
        .filter(|campaign| {
            campaign.status == "active"
                || campaign
                    .ended_at
                    .as_deref()
                    .and_then(parse_trade_date)
                    .is_some_and(|ended_at| ended_at >= cutoff)
        })
        .collect()
}

fn median(values: &mut [f64]) -> Option<f64> {
    if values.is_empty() {
        return None;
    }
    values.sort_by(f64::total_cmp);
    let middle = values.len() / 2;
    Some(if values.len() % 2 == 0 {
        (values[middle - 1] + values[middle]) / 2.0
    } else {
        values[middle]
    })
}

fn fact_flags(
    completed_campaigns: usize,
    active_campaigns: usize,
    net_premium_pnl: f64,
    retention_rate: Option<f64>,
    completed: &[&OptionCampaign],
) -> Vec<String> {
    let mut flags = Vec::new();
    if net_premium_pnl < 0.0 {
        flags.push("净亏损".to_string());
    }
    if retention_rate.is_some_and(|retention| retention < 0.4) {
        flags.push("低留存".to_string());
    }
    let worst = completed
        .iter()
        .filter_map(|campaign| campaign.net_premium_pnl)
        .min_by(f64::total_cmp);
    let mut positive: Vec<_> = completed
        .iter()
        .filter_map(|campaign| campaign.net_premium_pnl)
        .filter(|value| *value > 0.0)
        .collect();
    if let (Some(worst), Some(positive_median)) = (worst, median(&mut positive)) {
        if worst < 0.0 && worst.abs() > positive_median * 3.0 {
            flags.push("单次损失较大".to_string());
        }
    }
    if completed_campaigns >= 3 && retention_rate.is_some_and(|retention| retention >= 0.7) {
        flags.push("高留存".to_string());
    }
    if completed_campaigns < 3 {
        flags.push("样本不足".to_string());
    }
    if active_campaigns > 0 {
        flags.push("有进行中仓位".to_string());
    }
    flags
}

fn build_underlying(
    underlying: String,
    mut campaigns: Vec<OptionCampaign>,
) -> OptionUnderlyingReview {
    campaigns.sort_by(|left, right| {
        left.started_at
            .cmp(&right.started_at)
            .then(left.id.cmp(&right.id))
    });
    let completed: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.status == "completed")
        .collect();
    let completed_campaigns = completed.len();
    let active_campaigns = campaigns.len() - completed_campaigns;
    let gross_premium = completed
        .iter()
        .map(|campaign| campaign.gross_premium)
        .sum();
    let net_premium_pnl = completed
        .iter()
        .filter_map(|campaign| campaign.net_premium_pnl)
        .sum();
    let capital_days = completed.iter().map(|campaign| campaign.capital_days).sum();
    let retention_rate = safe_ratio(net_premium_pnl, gross_premium);
    let annualized_yield_on_notional = safe_ratio(net_premium_pnl * 365.0, capital_days);
    let worst_campaign_pnl = completed
        .iter()
        .filter_map(|campaign| campaign.net_premium_pnl)
        .min_by(f64::total_cmp);
    let flags = fact_flags(
        completed_campaigns,
        active_campaigns,
        net_premium_pnl,
        retention_rate,
        &completed,
    );
    OptionUnderlyingReview {
        underlying,
        completed_campaigns,
        active_campaigns,
        gross_premium,
        net_premium_pnl,
        retention_rate,
        annualized_yield_on_notional,
        worst_campaign_pnl,
        flags,
        campaigns,
    }
}

fn build_report(
    account_id: &str,
    currency: String,
    period_days: Option<i64>,
    today: NaiveDate,
    campaigns: Vec<OptionCampaign>,
    quality: OptionReviewDataQuality,
) -> OptionReviewReport {
    let completed: Vec<_> = campaigns
        .iter()
        .filter(|campaign| campaign.status == "completed")
        .collect();
    let completed_campaigns = completed.len();
    let active_campaigns = campaigns.len() - completed_campaigns;
    let gross_premium = completed
        .iter()
        .map(|campaign| campaign.gross_premium)
        .sum();
    let net_premium_pnl = completed
        .iter()
        .filter_map(|campaign| campaign.net_premium_pnl)
        .sum();
    let capital_days = completed.iter().map(|campaign| campaign.capital_days).sum();
    let worst_campaign = completed
        .iter()
        .min_by(|left, right| {
            left.net_premium_pnl
                .unwrap_or_default()
                .total_cmp(&right.net_premium_pnl.unwrap_or_default())
        })
        .map(|campaign| OptionWorstCampaign {
            campaign_id: campaign.id.clone(),
            underlying: campaign.underlying.clone(),
            started_at: campaign.started_at.clone(),
            ended_at: campaign.ended_at.clone().expect("completed campaign end"),
            strategy_path: campaign.strategy_path.clone(),
            net_premium_pnl: campaign.net_premium_pnl.expect("completed campaign P&L"),
        });
    let mut grouped: BTreeMap<String, Vec<OptionCampaign>> = BTreeMap::new();
    for campaign in campaigns {
        grouped
            .entry(campaign.underlying.clone())
            .or_default()
            .push(campaign);
    }
    let underlyings = grouped
        .into_iter()
        .map(|(underlying, campaigns)| build_underlying(underlying, campaigns))
        .collect();
    OptionReviewReport {
        account_id: account_id.to_string(),
        currency,
        period_days,
        generated_at: today.format("%Y-%m-%d").to_string(),
        summary: OptionReviewSummary {
            completed_campaigns,
            active_campaigns,
            gross_premium,
            net_premium_pnl,
            retention_rate: safe_ratio(net_premium_pnl, gross_premium),
            annualized_yield_on_notional: safe_ratio(net_premium_pnl * 365.0, capital_days),
            worst_campaign,
        },
        underlyings,
        data_quality: quality,
    }
}

fn data_quality_notes(quality: &OptionReviewDataQuality) -> Vec<String> {
    let mut notes = Vec::new();
    if quality.excluded_open_campaigns > 0 {
        notes.push("进行中Campaign未计入绩效指标".to_string());
    }
    if quality.missing_trade_dates > 0 {
        notes.push("缺少有效交易日期的记录已排除".to_string());
    }
    if quality.unmatched_records > 0 {
        notes.push("无法匹配开仓的结束记录已排除".to_string());
    }
    notes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn db_with_account() -> (Database, String) {
        let db = Database::new(":memory:").expect("in-memory db");
        let account_id = "acct-option-review".to_string();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at)
             VALUES (?1, 'Options', 'US', '2026-01-01', '2026-01-01')",
            rusqlite::params![account_id],
        )
        .unwrap();
        drop(conn);
        (db, account_id)
    }

    fn insert_record(
        db: &Database,
        id: &str,
        account_id: &str,
        symbol: &str,
        underlying: &str,
        expiry: &str,
        strike: f64,
        option_type: &str,
        action: &str,
        code: &str,
        quantity: i64,
        amount: f64,
        commission: f64,
        fee: f64,
        traded_at: Option<&str>,
    ) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO option_records
             (id, account_id, option_symbol, underlying, expiry_date, strike_price,
              option_type, action, code, quantity, price, amount, commission, fee,
              traded_at, settled_at, created_at, contract_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11, ?12, ?13, ?14, NULL, '2026-01-01', 'active')",
            rusqlite::params![
                id,
                account_id,
                symbol,
                underlying,
                expiry,
                strike,
                option_type,
                action,
                code,
                quantity,
                amount,
                commission,
                fee,
                traded_at
            ],
        )
        .unwrap();
    }

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 24).unwrap()
    }

    fn fixed_review(
        db: &Database,
        account_id: &str,
        period_days: Option<i64>,
    ) -> OptionReviewReport {
        get_option_review_at(db, account_id, period_days, today()).unwrap()
    }

    fn insert_cycle(
        db: &Database,
        account_id: &str,
        id: &str,
        underlying: &str,
        option_type: &str,
        opened_at: &str,
        ended_at: &str,
        gross: f64,
        close_cost: f64,
        close_code: &str,
    ) {
        let symbol = format!("{underlying} {id} {option_type}");
        insert_record(
            db,
            &format!("open-{id}"),
            account_id,
            &symbol,
            underlying,
            "31DEC26",
            100.0,
            option_type,
            "SELL",
            "O",
            1,
            gross,
            0.0,
            0.0,
            Some(opened_at),
        );
        insert_record(
            db,
            &format!("close-{id}"),
            account_id,
            &symbol,
            underlying,
            "31DEC26",
            100.0,
            option_type,
            "BUY",
            close_code,
            1,
            close_cost,
            0.0,
            0.0,
            Some(ended_at),
        );
    }

    #[test]
    fn expired_put_keeps_premium_net_of_fees() {
        let (db, account_id) = db_with_account();
        insert_record(
            &db,
            "o1",
            &account_id,
            "AAPL 20FEB26 100 P",
            "AAPL",
            "20FEB26",
            100.0,
            "P",
            "SELL",
            "O",
            1,
            200.0,
            1.0,
            0.2,
            Some("2026-01-15"),
        );
        insert_record(
            &db,
            "c1",
            &account_id,
            "AAPL 20FEB26 100 P",
            "AAPL",
            "20FEB26",
            100.0,
            "P",
            "BUY",
            "C;Ep",
            1,
            0.0,
            0.5,
            0.1,
            Some("2026-02-20"),
        );

        let report = fixed_review(&db, &account_id, None);
        assert_eq!(report.summary.completed_campaigns, 1);
        assert!((report.summary.gross_premium - 200.0).abs() < 1e-9);
        assert!((report.summary.net_premium_pnl - 198.2).abs() < 1e-9);
        assert_eq!(
            report.underlyings[0].campaigns[0].strategy_path,
            vec!["CSP"]
        );
    }

    #[test]
    fn losing_buyback_has_negative_retention() {
        let (db, account_id) = db_with_account();
        insert_record(
            &db,
            "loss-open",
            &account_id,
            "AAPL LOSS P",
            "AAPL",
            "31DEC26",
            100.0,
            "P",
            "SELL",
            "O",
            1,
            200.0,
            0.7,
            0.3,
            Some("2026-01-01"),
        );
        insert_record(
            &db,
            "loss-close",
            &account_id,
            "AAPL LOSS P",
            "AAPL",
            "31DEC26",
            100.0,
            "P",
            "BUY",
            "C;P",
            1,
            260.0,
            0.6,
            0.4,
            Some("2026-01-10"),
        );

        let campaign = &fixed_review(&db, &account_id, None).underlyings[0].campaigns[0];
        assert!((campaign.net_premium_pnl.unwrap() - -62.0).abs() < 1e-9);
        assert!((campaign.retention_rate.unwrap() - -0.31).abs() < 1e-9);
    }

    #[test]
    fn partial_close_allocates_amounts_and_fees_fifo() {
        let (db, account_id) = db_with_account();
        insert_record(
            &db,
            "partial-open",
            &account_id,
            "AAPL PARTIAL P",
            "AAPL",
            "31DEC26",
            100.0,
            "P",
            "SELL",
            "O",
            2,
            400.0,
            2.0,
            0.4,
            Some("2026-01-01"),
        );
        for (id, date, amount) in [
            ("partial-close-1", "2026-01-10", 50.0),
            ("partial-close-2", "2026-01-11", 70.0),
        ] {
            insert_record(
                &db,
                id,
                &account_id,
                "AAPL PARTIAL P",
                "AAPL",
                "31DEC26",
                100.0,
                "P",
                "BUY",
                "C;P",
                1,
                amount,
                0.5,
                0.1,
                Some(date),
            );
        }

        let (_, records, share_lots, splits) = load_inputs(&db, &account_id).unwrap();
        let (cycles, _) = pair_cycles_fifo(records, &share_lots, &splits, today());
        assert_eq!(cycles.len(), 2);
        assert!(cycles
            .iter()
            .all(|cycle| (cycle.gross_premium - 200.0).abs() < 1e-9));
        let campaign = &fixed_review(&db, &account_id, None).underlyings[0].campaigns[0];
        assert!((campaign.close_cost - 120.0).abs() < 1e-9);
        assert!((campaign.fees - 3.6).abs() < 1e-9);
        assert!((campaign.net_premium_pnl.unwrap() - 276.4).abs() < 1e-9);
    }

    #[test]
    fn rolls_within_seven_days_share_a_campaign() {
        let (db, account_id) = db_with_account();
        insert_cycle(
            &db,
            &account_id,
            "roll-1",
            "AAPL",
            "P",
            "2026-01-01",
            "2026-01-10",
            100.0,
            10.0,
            "C;P",
        );
        insert_cycle(
            &db,
            &account_id,
            "roll-2",
            "AAPL",
            "P",
            "2026-01-17",
            "2026-01-20",
            120.0,
            20.0,
            "C;P",
        );

        let underlying = &fixed_review(&db, &account_id, None).underlyings[0];
        assert_eq!(underlying.campaigns.len(), 1);
        assert!((underlying.campaigns[0].gross_premium - 220.0).abs() < 1e-9);
        assert!((underlying.campaigns[0].close_cost - 30.0).abs() < 1e-9);
    }

    #[test]
    fn cycles_after_eight_days_are_separate_campaigns() {
        let (db, account_id) = db_with_account();
        insert_cycle(
            &db,
            &account_id,
            "gap-1",
            "AAPL",
            "P",
            "2026-01-01",
            "2026-01-10",
            100.0,
            0.0,
            "C;Ep",
        );
        insert_cycle(
            &db,
            &account_id,
            "gap-2",
            "AAPL",
            "P",
            "2026-01-18",
            "2026-01-20",
            120.0,
            0.0,
            "C;Ep",
        );

        let campaigns = &fixed_review(&db, &account_id, None).underlyings[0].campaigns;
        assert_eq!(campaigns.len(), 2);
        assert_ne!(campaigns[0].id, campaigns[1].id);
        assert!(
            (campaigns
                .iter()
                .map(|campaign| campaign.gross_premium)
                .sum::<f64>()
                - 220.0)
                .abs()
                < 1e-9
        );
    }

    #[test]
    fn overlap_checks_every_cycle_in_current_campaign() {
        let (db, account_id) = db_with_account();
        for (id, opened, ended, gross) in [
            ("overlap-a", "2026-01-01", "2026-01-30", 100.0),
            ("overlap-b", "2026-01-05", "2026-01-06", 200.0),
            ("overlap-c", "2026-01-25", "2026-01-26", 300.0),
        ] {
            insert_cycle(
                &db,
                &account_id,
                id,
                "AAPL",
                "P",
                opened,
                ended,
                gross,
                0.0,
                "C;Ep",
            );
        }

        let campaigns = &fixed_review(&db, &account_id, None).underlyings[0].campaigns;
        assert_eq!(campaigns.len(), 1);
        assert!((campaigns[0].gross_premium - 600.0).abs() < 1e-9);
    }

    #[test]
    fn assigned_put_links_to_call_within_thirty_days() {
        let (db, account_id) = db_with_account();
        insert_cycle(
            &db,
            &account_id,
            "wheel-put",
            "AAPL",
            "P",
            "2026-02-01",
            "2026-03-01",
            100.0,
            0.0,
            "A;C",
        );
        insert_cycle(
            &db,
            &account_id,
            "wheel-call",
            "AAPL",
            "C",
            "2026-03-31",
            "2026-04-05",
            80.0,
            0.0,
            "C;Ep",
        );

        let campaigns = &fixed_review(&db, &account_id, None).underlyings[0].campaigns;
        assert_eq!(campaigns.len(), 1);
        assert_eq!(campaigns[0].strategy_path, vec!["CSP", "Covered Call"]);
        assert!((campaigns[0].gross_premium - 180.0).abs() < 1e-9);
    }

    #[test]
    fn active_campaign_is_excluded_from_summary() {
        let (db, account_id) = db_with_account();
        insert_cycle(
            &db,
            &account_id,
            "active-done",
            "AAPL",
            "P",
            "2026-01-01",
            "2026-01-10",
            100.0,
            0.0,
            "C;Ep",
        );
        insert_record(
            &db,
            "active-open",
            &account_id,
            "AAPL ACTIVE P",
            "AAPL",
            "31DEC26",
            100.0,
            "P",
            "SELL",
            "O",
            1,
            120.0,
            1.0,
            0.0,
            Some("2026-01-17"),
        );

        let report = fixed_review(&db, &account_id, None);
        assert_eq!(report.summary.completed_campaigns, 0);
        assert_eq!(report.summary.active_campaigns, 1);
        let campaign = &report.underlyings[0].campaigns[0];
        assert_eq!(campaign.net_premium_pnl, None);
        assert_eq!(campaign.retention_rate, None);
        assert_eq!(campaign.annualized_yield_on_notional, None);
        assert_eq!(report.data_quality.excluded_open_campaigns, 1);
    }

    #[test]
    fn completed_period_filter_uses_end_date_but_keeps_active() {
        let (db, account_id) = db_with_account();
        insert_cycle(
            &db,
            &account_id,
            "old-done",
            "AAPL",
            "P",
            "2024-12-01",
            "2025-01-01",
            100.0,
            0.0,
            "C;Ep",
        );
        insert_record(
            &db,
            "old-active",
            &account_id,
            "AAPL OLD ACTIVE P",
            "AAPL",
            "31DEC26",
            100.0,
            "P",
            "SELL",
            "O",
            1,
            75.0,
            0.0,
            0.0,
            Some("2025-01-20"),
        );

        let report = fixed_review(&db, &account_id, Some(365));
        assert_eq!(report.summary.completed_campaigns, 0);
        assert_eq!(report.summary.active_campaigns, 1);
        assert!((report.underlyings[0].campaigns[0].gross_premium - 75.0).abs() < 1e-9);
    }

    #[test]
    fn missing_dates_and_unmatched_closes_are_reported() {
        let (db, account_id) = db_with_account();
        insert_record(
            &db,
            "missing-date",
            &account_id,
            "AAPL MISSING P",
            "AAPL",
            "31DEC26",
            100.0,
            "P",
            "SELL",
            "O",
            1,
            100.0,
            0.0,
            0.0,
            None,
        );
        insert_record(
            &db,
            "orphan-close",
            &account_id,
            "AAPL ORPHAN P",
            "AAPL",
            "31DEC26",
            100.0,
            "P",
            "BUY",
            "C;P",
            2,
            40.0,
            0.0,
            0.0,
            Some("2026-01-10"),
        );

        let report = fixed_review(&db, &account_id, None);
        assert_eq!(report.data_quality.missing_trade_dates, 1);
        assert_eq!(report.data_quality.unmatched_records, 1);
        assert!(report.underlyings.is_empty());
        assert!((report.summary.net_premium_pnl - 0.0).abs() < 1e-9);
    }

    #[test]
    fn intraday_fifo_only_prioritizes_sell_at_the_same_timestamp() {
        let (db, account_id) = db_with_account();
        insert_record(
            &db,
            "earlier-close",
            &account_id,
            "AAPL INTRADAY P",
            "AAPL",
            "31DEC26",
            100.0,
            "P",
            "BUY",
            "C;P",
            1,
            50.0,
            0.0,
            0.0,
            Some("2026-01-01, 09:00:00"),
        );
        insert_record(
            &db,
            "later-open",
            &account_id,
            "AAPL INTRADAY P",
            "AAPL",
            "31DEC26",
            100.0,
            "P",
            "SELL",
            "O",
            1,
            100.0,
            0.0,
            0.0,
            Some("2026-01-01, 10:00:00"),
        );

        let report = fixed_review(&db, &account_id, None);
        assert_eq!(report.data_quality.unmatched_records, 1);
        assert_eq!(report.summary.active_campaigns, 1);
        assert!((report.underlyings[0].campaigns[0].gross_premium - 100.0).abs() < 1e-9);
    }

    #[test]
    fn mixed_supported_timestamp_formats_preserve_intraday_fifo() {
        let (db, account_id) = db_with_account();
        insert_record(
            &db,
            "mixed-earlier-close",
            &account_id,
            "AAPL MIXED P",
            "AAPL",
            "31DEC26",
            100.0,
            "P",
            "BUY",
            "C;P",
            1,
            50.0,
            0.0,
            0.0,
            Some("2026/01/01 09:00"),
        );
        insert_record(
            &db,
            "mixed-later-open",
            &account_id,
            "AAPL MIXED P",
            "AAPL",
            "31DEC26",
            100.0,
            "P",
            "SELL",
            "O",
            1,
            100.0,
            0.0,
            0.0,
            Some("2026-01-01 10:00"),
        );

        let report = fixed_review(&db, &account_id, None);
        assert_eq!(report.data_quality.unmatched_records, 1);
        assert_eq!(report.summary.active_campaigns, 1);
        assert!((report.underlyings[0].campaigns[0].gross_premium - 100.0).abs() < 1e-9);
    }

    #[test]
    fn forward_split_close_conserves_both_record_allocations() {
        let (db, account_id) = db_with_account();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO stock_splits
                 (stock_code, split_date, ratio_from, ratio_to, created_at)
                 VALUES ('BRK B', '2023-06-01', 1, 2, '2023-06-01')",
                [],
            )
            .unwrap();
        }
        insert_record(
            &db,
            "split-open",
            &account_id,
            "BRK B 16JUN23 330 C",
            "BRK B",
            "16JUN23",
            330.0,
            "C",
            "SELL",
            "O",
            1,
            250.0,
            1.0,
            0.2,
            Some("2023-05-01"),
        );
        insert_record(
            &db,
            "split-close",
            &account_id,
            "BRK B 16JUN23 165 C",
            "BRK B",
            "16JUN23",
            165.0,
            "C",
            "BUY",
            "C;Ep",
            2,
            80.0,
            0.8,
            0.2,
            Some("2023-06-16"),
        );

        let report = fixed_review(&db, &account_id, None);
        assert_eq!(report.summary.completed_campaigns, 1);
        assert_eq!(report.summary.active_campaigns, 0);
        assert_eq!(report.data_quality.unmatched_records, 0);
        let campaign = &report.underlyings[0].campaigns[0];
        assert!((campaign.gross_premium - 250.0).abs() < 1e-9);
        assert!((campaign.close_cost - 80.0).abs() < 1e-9);
        assert!((campaign.fees - 2.2).abs() < 1e-9);
        assert!((campaign.net_premium_pnl.unwrap() - 167.8).abs() < 1e-9);
        assert!(
            (campaign.net_premium_pnl.unwrap()
                - (campaign.gross_premium - campaign.close_cost - campaign.fees))
                .abs()
                < 1e-9
        );
        assert!((campaign.retention_rate.unwrap() - 0.6712).abs() < 1e-9);
    }

    #[test]
    fn reverse_split_close_conserves_both_record_allocations() {
        let (db, account_id) = db_with_account();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO stock_splits
                 (stock_code, split_date, ratio_from, ratio_to, created_at)
                 VALUES ('BRK B', '2026-06-01', 2, 1, '2026-06-01')",
                [],
            )
            .unwrap();
        }
        insert_record(
            &db,
            "reverse-split-open",
            &account_id,
            "BRK B 31DEC26 165 C",
            "BRK B",
            "31DEC26",
            165.0,
            "C",
            "SELL",
            "O",
            2,
            400.0,
            2.0,
            0.4,
            Some("2026-05-01"),
        );
        insert_record(
            &db,
            "reverse-split-close",
            &account_id,
            "BRK B 31DEC26 330 C",
            "BRK B",
            "31DEC26",
            330.0,
            "C",
            "BUY",
            "C;P",
            1,
            70.0,
            0.5,
            0.1,
            Some("2026-06-16"),
        );

        let report = fixed_review(&db, &account_id, None);
        assert_eq!(report.summary.completed_campaigns, 1);
        assert_eq!(report.summary.active_campaigns, 0);
        assert_eq!(report.data_quality.unmatched_records, 0);
        let campaign = &report.underlyings[0].campaigns[0];
        assert!((campaign.gross_premium - 400.0).abs() < 1e-9);
        assert!((campaign.close_cost - 70.0).abs() < 1e-9);
        assert!((campaign.fees - 3.0).abs() < 1e-9);
        assert!((campaign.net_premium_pnl.unwrap() - 327.0).abs() < 1e-9);
        assert!(
            (campaign.net_premium_pnl.unwrap()
                - (campaign.gross_premium - campaign.close_cost - campaign.fees))
                .abs()
                < 1e-9
        );
        assert!((campaign.retention_rate.unwrap() - 0.8175).abs() < 1e-9);
    }

    #[test]
    fn partial_adjusted_close_keeps_unclosed_open_exposure_active() {
        let (db, account_id) = db_with_account();
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO stock_splits
                 (stock_code, split_date, ratio_from, ratio_to, created_at)
                 VALUES ('BRK B', '2026-06-01', 1, 2, '2026-06-01')",
                [],
            )
            .unwrap();
        }
        insert_record(
            &db,
            "partial-split-open",
            &account_id,
            "BRK B 31DEC26 330 C",
            "BRK B",
            "31DEC26",
            330.0,
            "C",
            "SELL",
            "O",
            1,
            200.0,
            2.0,
            0.0,
            Some("2026-05-01"),
        );
        insert_record(
            &db,
            "partial-split-close",
            &account_id,
            "BRK B 31DEC26 165 C",
            "BRK B",
            "31DEC26",
            165.0,
            "C",
            "BUY",
            "C;P",
            1,
            30.0,
            0.5,
            0.0,
            Some("2026-06-16"),
        );

        let (_, records, share_lots, splits) = load_inputs(&db, &account_id).unwrap();
        let (cycles, quality) = pair_cycles_fifo(records, &share_lots, &splits, today());
        assert_eq!(quality.unmatched_records, 0);
        assert_eq!(cycles.len(), 2);
        let completed = cycles
            .iter()
            .find(|cycle| cycle.status == "closed")
            .expect("completed adjusted portion");
        let active = cycles
            .iter()
            .find(|cycle| cycle.status == "active")
            .expect("remaining open exposure");
        assert!((completed.gross_premium - 100.0).abs() < 1e-9);
        assert!((completed.close_cost - 30.0).abs() < 1e-9);
        assert!((completed.fees - 1.5).abs() < 1e-9);
        assert!((active.gross_premium - 100.0).abs() < 1e-9);
        assert!((active.close_cost - 0.0).abs() < 1e-9);
        assert!((active.fees - 1.0).abs() < 1e-9);
    }

    #[test]
    fn split_remainder_overflow_is_unmatched_and_leaves_open_state_atomic() {
        let (db, account_id) = db_with_account();
        let first_denominator = 1_000_003_i64;
        let second_factor = 1_000_033_i64;
        let second_denominator = first_denominator * second_factor;
        let third_denominator = 1_000_000_000_000_000_003_i64;
        let fourth_denominator = 2_000_000_000_000_000_033_i64;
        let open_strike = 1_000_000_000_000_000_000_f64;
        let split_closes = [
            (
                "2026-01-02",
                first_denominator - 1,
                first_denominator,
                "2026-01-03",
                10.0,
            ),
            (
                "2026-01-04",
                second_factor - 1,
                second_denominator,
                "2026-01-05",
                20.0,
            ),
            ("2026-01-06", 1, third_denominator, "2026-01-07", 30.0),
            ("2026-01-08", 1, fourth_denominator, "2026-01-09", 40.0),
        ];

        insert_record(
            &db,
            "overflow-open",
            &account_id,
            "OVERFLOW OPEN C",
            "OVERFLOW",
            "31DEC26",
            open_strike,
            "C",
            "SELL",
            "O",
            1,
            100.0,
            0.0,
            0.0,
            Some("2026-01-01"),
        );
        for (index, (split_date, ratio_from, ratio_to, close_date, amount)) in
            split_closes.into_iter().enumerate()
        {
            let conn = db.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO stock_splits
                 (stock_code, split_date, ratio_from, ratio_to, created_at)
                 VALUES ('OVERFLOW', ?1, ?2, ?3, ?1)",
                rusqlite::params![split_date, ratio_from, ratio_to],
            )
            .unwrap();
            drop(conn);
            insert_record(
                &db,
                &format!("overflow-close-{index}"),
                &account_id,
                &format!("OVERFLOW CLOSE {index} C"),
                "OVERFLOW",
                "31DEC26",
                open_strike * ratio_from as f64 / ratio_to as f64,
                "C",
                "BUY",
                "C;P",
                1,
                amount,
                0.0,
                0.0,
                Some(close_date),
            );
        }

        let (_, records, share_lots, splits) = load_inputs(&db, &account_id).unwrap();
        let (cycles, quality) = pair_cycles_fifo(records, &share_lots, &splits, today());

        assert_eq!(quality.unmatched_records, 1);
        assert_eq!(cycles.len(), 4);
        assert_eq!(
            cycles
                .iter()
                .filter(|cycle| cycle.status == "closed")
                .count(),
            3
        );
        assert_eq!(
            cycles
                .iter()
                .filter(|cycle| cycle.status == "active")
                .count(),
            1
        );
        assert!((cycles.iter().map(|cycle| cycle.close_cost).sum::<f64>() - 60.0).abs() < 1e-9);
        assert!((cycles.iter().map(|cycle| cycle.gross_premium).sum::<f64>() - 100.0).abs() < 1e-9);
    }

    #[test]
    fn split_outside_open_close_window_does_not_match() {
        for (case, split_date) in [("before-open", "2026-04-30"), ("after-close", "2026-06-02")] {
            let (db, account_id) = db_with_account();
            {
                let conn = db.conn.lock().unwrap();
                conn.execute(
                    "INSERT INTO stock_splits
                     (stock_code, split_date, ratio_from, ratio_to, created_at)
                     VALUES ('BRK B', ?1, 1, 2, ?1)",
                    [split_date],
                )
                .unwrap();
            }
            insert_record(
                &db,
                &format!("{case}-open"),
                &account_id,
                "BRK B 31DEC26 330 C",
                "BRK B",
                "31DEC26",
                330.0,
                "C",
                "SELL",
                "O",
                1,
                250.0,
                1.0,
                0.0,
                Some("2026-05-01"),
            );
            insert_record(
                &db,
                &format!("{case}-close"),
                &account_id,
                "BRK B 31DEC26 165 C",
                "BRK B",
                "31DEC26",
                165.0,
                "C",
                "BUY",
                "C;P",
                1,
                50.0,
                0.5,
                0.0,
                Some("2026-06-01"),
            );

            let report = fixed_review(&db, &account_id, None);
            assert_eq!(report.data_quality.unmatched_records, 1, "{case}");
            assert_eq!(report.summary.completed_campaigns, 0, "{case}");
            assert_eq!(report.summary.active_campaigns, 1, "{case}");
            assert!(
                (report.underlyings[0].campaigns[0].gross_premium - 250.0).abs() < 1e-9,
                "{case}"
            );
        }
    }

    #[test]
    fn underlying_and_account_totals_equal_campaign_sums() {
        let (db, account_id) = db_with_account();
        for (underlying, prefix) in [("AAPL", "aapl"), ("MSFT", "msft")] {
            insert_cycle(
                &db,
                &account_id,
                &format!("{prefix}-1"),
                underlying,
                "P",
                "2026-01-01",
                "2026-01-02",
                100.0,
                10.0,
                "C;P",
            );
            insert_cycle(
                &db,
                &account_id,
                &format!("{prefix}-2"),
                underlying,
                "P",
                "2026-01-20",
                "2026-01-21",
                200.0,
                20.0,
                "C;P",
            );
        }

        let report = fixed_review(&db, &account_id, None);
        let underlying_gross: f64 = report
            .underlyings
            .iter()
            .map(|item| item.gross_premium)
            .sum();
        let campaign_gross: f64 = report
            .underlyings
            .iter()
            .flat_map(|item| &item.campaigns)
            .map(|campaign| campaign.gross_premium)
            .sum();
        let underlying_net: f64 = report
            .underlyings
            .iter()
            .map(|item| item.net_premium_pnl)
            .sum();
        let campaign_net: f64 = report
            .underlyings
            .iter()
            .flat_map(|item| &item.campaigns)
            .filter_map(|campaign| campaign.net_premium_pnl)
            .sum();
        assert!((report.summary.gross_premium - underlying_gross).abs() < 1e-9);
        assert!((underlying_gross - campaign_gross).abs() < 1e-9);
        assert!((report.summary.net_premium_pnl - underlying_net).abs() < 1e-9);
        assert!((underlying_net - campaign_net).abs() < 1e-9);
    }

    fn completed_campaign(id: &str, net: f64) -> OptionCampaign {
        OptionCampaign {
            id: id.to_string(),
            underlying: "TEST".to_string(),
            started_at: "2026-01-01".to_string(),
            ended_at: Some("2026-01-02".to_string()),
            status: "completed".to_string(),
            inferred: true,
            strategy_path: vec!["CSP".to_string()],
            gross_premium: 100.0,
            close_cost: 100.0 - net,
            fees: 0.0,
            net_premium_pnl: Some(net),
            secured_notional: 10_000.0,
            capital_days: 10_000.0,
            retention_rate: Some(net / 100.0),
            annualized_yield_on_notional: Some(net * 365.0 / 10_000.0),
        }
    }

    #[test]
    fn fact_flags_follow_threshold_boundaries() {
        let high = vec![
            completed_campaign("h1", 70.0),
            completed_campaign("h2", 70.0),
            completed_campaign("h3", 70.0),
        ];
        let high_refs: Vec<_> = high.iter().collect();
        assert!(fact_flags(3, 0, 210.0, Some(0.7), &high_refs).contains(&"高留存".to_string()));

        let boundary = vec![
            completed_campaign("b1", 40.0),
            completed_campaign("b2", 40.0),
            completed_campaign("b3", 40.0),
        ];
        let boundary_refs: Vec<_> = boundary.iter().collect();
        assert!(!fact_flags(3, 0, 120.0, Some(0.4), &boundary_refs).contains(&"低留存".to_string()));

        let loss_at_boundary = vec![
            completed_campaign("p1", 100.0),
            completed_campaign("p2", 100.0),
            completed_campaign("l1", -300.0),
        ];
        let loss_at_boundary_refs: Vec<_> = loss_at_boundary.iter().collect();
        assert!(
            !fact_flags(3, 0, -100.0, Some(-1.0 / 3.0), &loss_at_boundary_refs)
                .contains(&"单次损失较大".to_string())
        );

        let large_loss = vec![
            completed_campaign("p1", 100.0),
            completed_campaign("p2", 100.0),
            completed_campaign("l1", -300.01),
        ];
        let large_loss_refs: Vec<_> = large_loss.iter().collect();
        assert!((large_loss[2].net_premium_pnl.unwrap() - -300.01).abs() < 1e-9);
        assert!(
            fact_flags(3, 0, -100.01, Some(-100.01 / 300.0), &large_loss_refs)
                .contains(&"单次损失较大".to_string())
        );
    }

    #[test]
    fn existing_account_without_options_returns_empty_report() {
        let (db, account_id) = db_with_account();
        let report = fixed_review(&db, &account_id, None);
        assert_eq!(report.currency, "USD");
        assert!(report.underlyings.is_empty());
        assert!((report.summary.gross_premium - 0.0).abs() < 1e-9);
    }

    #[test]
    fn unknown_account_returns_an_error() {
        let db = Database::new(":memory:").unwrap();
        let error = get_option_review_at(&db, "missing", None, today()).unwrap_err();
        assert!(error.contains("Account not found"));
    }
}
