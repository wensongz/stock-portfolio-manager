use crate::models::stock_review::{
    normalized_stock_market, normalized_stock_symbol, stock_securities_equal, ForwardEffectWindow,
    MetricStatus, StockActionReview, StockReviewIssue, StockReviewIssueSeverity,
    StockReviewOverride,
};
use crate::models::Transaction;
use crate::services::quote_service::is_cash_symbol;
use chrono::NaiveDate;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

const EPSILON: f64 = 1e-9;

/// A single stock-position replay step retained for campaign construction.
#[derive(Debug, Clone, PartialEq)]
pub struct PositionEvent {
    pub transaction_id: String,
    pub account_id: String,
    pub symbol: String,
    pub market: String,
    pub transaction_type: String,
    pub traded_at: String,
    pub trade_date: NaiveDate,
    pub shares_delta: f64,
    pub shares_before: f64,
    pub shares_after: f64,
    pub is_date_precision: bool,
    pub is_transfer: bool,
}

/// Pure derivation output used by review orchestration and campaign building.
#[derive(Debug, Clone)]
pub struct ActionBuildResult {
    pub actions: Vec<StockActionReview>,
    pub position_events: Vec<PositionEvent>,
    pub corrected_transactions: Vec<CorrectedTransaction>,
    pub issues: Vec<StockReviewIssue>,
}

/// Canonical ordered source row after active override disposition. Every
/// stock-review cash/flow consumer must use this materialization rather than
/// iterating the raw transaction table independently.
#[derive(Debug, Clone)]
pub struct CorrectedTransaction {
    pub transaction: Transaction,
    pub is_transfer: bool,
    pub has_cash_effect: bool,
    pub action_id: Option<String>,
}

#[derive(Clone)]
struct ReplayRecord<'a> {
    transaction: &'a Transaction,
    is_transfer: bool,
}

#[derive(Clone)]
struct ActionFill<'a> {
    record: ReplayRecord<'a>,
    trade_date: NaiveDate,
    side: &'static str,
    action_type: String,
    shares_before: f64,
    shares_after: f64,
}

pub fn build_stock_actions(
    transactions: &[Transaction],
    overrides: &[StockReviewOverride],
) -> ActionBuildResult {
    let override_state = OverrideState::from_overrides(overrides, transactions);
    let mut records: Vec<ReplayRecord<'_>> = transactions
        .iter()
        .filter(|transaction| !override_state.excluded_ids.contains(&transaction.id))
        .map(|transaction| ReplayRecord {
            transaction,
            is_transfer: override_state.transfer_ids.contains(&transaction.id),
        })
        .collect();
    records.sort_by(|left, right| compare_records(left, right, &override_state.same_day_order));
    let mut corrected_transactions = records
        .iter()
        .map(|record| CorrectedTransaction {
            transaction: record.transaction.clone(),
            is_transfer: record.is_transfer,
            has_cash_effect: !record.is_transfer,
            action_id: None,
        })
        .collect::<Vec<_>>();

    let mut shares_by_position: HashMap<(String, String, String), f64> = HashMap::new();
    let mut actions = Vec::new();
    let mut position_events = Vec::new();
    let mut issues = duplicate_issues(&override_state.duplicate_ids, transactions);
    let mut fills: Vec<ActionFill<'_>> = Vec::new();
    let mut date_only_reversals: HashSet<(String, String, String, NaiveDate)> = HashSet::new();

    for record in records {
        let transaction = record.transaction;
        if is_cash_symbol(&transaction.symbol) || transaction.transaction_type == "PAY" {
            flush_action_fills(&mut fills, &mut actions);
            continue;
        }

        let trade_date = trade_date(&transaction.traded_at);
        let Some(trade_date) = trade_date else {
            flush_action_fills(&mut fills, &mut actions);
            issues.push(issue(
                "invalid_trade_date",
                StockReviewIssueSeverity::Error,
                "Transaction date cannot be parsed for stock review replay.",
                transaction,
                None,
            ));
            continue;
        };

        let key = (
            transaction.account_id.clone(),
            symbol_key(&transaction.symbol),
            market_key(&transaction.market),
        );
        let shares_before = *shares_by_position.get(&key).unwrap_or(&0.0);
        let delta = match transaction.transaction_type.as_str() {
            "BUY" | "OPEN" => transaction.shares,
            "SELL" => -transaction.shares,
            _ => {
                flush_action_fills(&mut fills, &mut actions);
                issues.push(issue(
                    "unsupported_stock_transaction",
                    StockReviewIssueSeverity::Warning,
                    "Transaction type cannot be classified as a long-stock position change.",
                    transaction,
                    Some(trade_date),
                ));
                continue;
            }
        };
        let shares_after = shares_before + delta;
        shares_by_position.insert(key, shares_after);

        let is_date_precision = is_date_only(&transaction.traded_at);
        position_events.push(PositionEvent {
            transaction_id: transaction.id.clone(),
            account_id: transaction.account_id.clone(),
            symbol: transaction.symbol.clone(),
            market: transaction.market.clone(),
            transaction_type: transaction.transaction_type.clone(),
            traded_at: transaction.traded_at.clone(),
            trade_date,
            shares_delta: delta,
            shares_before,
            shares_after,
            is_date_precision,
            is_transfer: record.is_transfer,
        });

        if transaction.transaction_type == "OPEN" {
            flush_action_fills(&mut fills, &mut actions);
            continue;
        }

        if shares_before < -EPSILON || shares_after < -EPSILON {
            flush_action_fills(&mut fills, &mut actions);
            issues.push(issue(
                "negative_position",
                StockReviewIssueSeverity::Error,
                "Position replay produced a negative share balance; no action is inferred.",
                transaction,
                Some(trade_date),
            ));
            continue;
        }

        let action_type = match transaction.transaction_type.as_str() {
            "BUY" if shares_before.abs() <= EPSILON => "open",
            "BUY" if shares_before > EPSILON => "add",
            "SELL" if shares_before <= EPSILON => {
                flush_action_fills(&mut fills, &mut actions);
                issues.push(issue(
                    "unexplained_position_path",
                    StockReviewIssueSeverity::Error,
                    "Sell transaction has no preceding long position; no action is inferred.",
                    transaction,
                    Some(trade_date),
                ));
                continue;
            }
            "SELL" if shares_after.abs() <= EPSILON => "close",
            "SELL" => "reduce",
            _ => unreachable!("only BUY and SELL reach action classification"),
        }
        .to_string();

        let side = if transaction.transaction_type == "BUY" {
            "buy"
        } else {
            "sell"
        };
        if let Some(previous) = fills.last() {
            if !same_action_group(previous, transaction, trade_date, side, record.is_transfer) {
                flush_action_fills(&mut fills, &mut actions);
            }
        }
        fills.push(ActionFill {
            record,
            trade_date,
            side,
            action_type,
            shares_before,
            shares_after,
        });

        if is_date_precision {
            date_only_reversals.insert((
                transaction.account_id.clone(),
                symbol_key(&transaction.symbol),
                market_key(&transaction.market),
                trade_date,
            ));
        }
    }
    flush_action_fills(&mut fills, &mut actions);
    let action_ids_by_transaction = actions
        .iter()
        .flat_map(|action| {
            action
                .transaction_ids
                .iter()
                .map(move |transaction_id| (transaction_id.as_str(), action.action_id.as_str()))
        })
        .collect::<HashMap<_, _>>();
    for corrected in &mut corrected_transactions {
        corrected.action_id = action_ids_by_transaction
            .get(corrected.transaction.id.as_str())
            .map(|value| (*value).to_string());
    }

    for (account_id, normalized_symbol, normalized_market, date) in date_only_reversals {
        let relevant_events = position_events
            .iter()
            .filter(|event| {
                event.account_id == account_id
                    && symbol_key(&event.symbol) == normalized_symbol
                    && market_key(&event.market) == normalized_market
                    && event.trade_date == date
                    && event.is_date_precision
                    && (event.transaction_type == "BUY" || event.transaction_type == "SELL")
            })
            .collect::<Vec<_>>();
        let sides: HashSet<&str> = relevant_events
            .iter()
            .map(|event| event.transaction_type.as_str())
            .collect();
        let order_is_complete = !relevant_events.is_empty()
            && relevant_events.iter().all(|event| {
                override_state
                    .same_day_order
                    .contains_key(&event.transaction_id)
            });
        if sides.len() > 1 && !order_is_complete {
            issues.push(StockReviewIssue {
                code: "same_day_order_uncertain".to_string(),
                severity: StockReviewIssueSeverity::Warning,
                message: "Same-day reversal has date-only precision; confirm transaction order before using derived metrics.".to_string(),
                affected_symbol: relevant_events.first().map(|event| event.symbol.clone()),
                affected_date: Some(date),
            });
        }
    }

    ActionBuildResult {
        actions,
        position_events,
        corrected_transactions,
        issues,
    }
}

struct OverrideState {
    excluded_ids: HashSet<String>,
    duplicate_ids: HashSet<String>,
    transfer_ids: HashSet<String>,
    same_day_order: HashMap<String, usize>,
}

impl OverrideState {
    fn from_overrides(overrides: &[StockReviewOverride], transactions: &[Transaction]) -> Self {
        let mut state = Self {
            excluded_ids: HashSet::new(),
            duplicate_ids: HashSet::new(),
            transfer_ids: HashSet::new(),
            same_day_order: HashMap::new(),
        };
        let transactions_by_id = transactions
            .iter()
            .map(|transaction| (transaction.id.as_str(), transaction))
            .collect::<HashMap<_, _>>();
        for override_record in overrides {
            let ids = parse_ids(&override_record.transaction_ids_json);
            let references = override_references(&ids, &transactions_by_id);
            match override_record.override_type.as_str() {
                "non_trade" => state.excluded_ids.extend(ids),
                "duplicate" => {
                    if references.as_ref().is_some_and(|references| {
                        references.len() >= 2 && !same_account_security(references)
                    }) {
                        continue;
                    }
                    state.duplicate_ids.extend(ids.iter().cloned());
                    if ids.len() < 2 {
                        state.excluded_ids.extend(ids);
                    } else {
                        // The confirmed first reference is the canonical source
                        // row; later equivalent references are excluded.
                        state.excluded_ids.extend(ids.into_iter().skip(1));
                    }
                }
                "transfer" => {
                    if references.as_ref().is_some_and(|references| {
                        references.len() == 2 && !same_security(references)
                    }) {
                        continue;
                    }
                    state.transfer_ids.extend(ids);
                }
                "same_day_order" => {
                    if references
                        .as_ref()
                        .is_some_and(|references| !same_position_day(references))
                    {
                        continue;
                    }
                    let ordered_ids = parse_ids(&override_record.value_json);
                    for (index, id) in ordered_ids.iter().enumerate() {
                        state.same_day_order.insert(id.clone(), index);
                    }
                }
                _ => {}
            }
        }
        state
    }
}

fn override_references<'a>(
    ids: &[String],
    transactions_by_id: &HashMap<&str, &'a Transaction>,
) -> Option<Vec<&'a Transaction>> {
    let references = ids
        .iter()
        .filter_map(|id| transactions_by_id.get(id.as_str()).copied())
        .collect::<Vec<_>>();
    (references.len() == ids.len()).then_some(references)
}

fn same_security(references: &[&Transaction]) -> bool {
    references.first().is_none_or(|first| {
        references.iter().skip(1).all(|transaction| {
            stock_securities_equal(
                &first.symbol,
                &first.market,
                &transaction.symbol,
                &transaction.market,
            )
        })
    })
}

fn same_account_security(references: &[&Transaction]) -> bool {
    references.first().is_none_or(|first| {
        references
            .iter()
            .skip(1)
            .all(|transaction| transaction.account_id == first.account_id)
    }) && same_security(references)
}

fn same_position_day(references: &[&Transaction]) -> bool {
    references.first().is_none_or(|first| {
        let first_date = trade_date(&first.traded_at);
        references.iter().skip(1).all(|transaction| {
            transaction.account_id == first.account_id
                && stock_securities_equal(
                    &first.symbol,
                    &first.market,
                    &transaction.symbol,
                    &transaction.market,
                )
                && trade_date(&transaction.traded_at) == first_date
        })
    })
}

fn compare_records(
    left: &ReplayRecord<'_>,
    right: &ReplayRecord<'_>,
    same_day_order: &HashMap<String, usize>,
) -> Ordering {
    let left_date = trade_date(&left.transaction.traded_at);
    let right_date = trade_date(&right.transaction.traded_at);
    if left.transaction.account_id == right.transaction.account_id
        && stock_securities_equal(
            &left.transaction.symbol,
            &left.transaction.market,
            &right.transaction.symbol,
            &right.transaction.market,
        )
        && left_date == right_date
    {
        if let (Some(left_rank), Some(right_rank)) = (
            same_day_order.get(&left.transaction.id),
            same_day_order.get(&right.transaction.id),
        ) {
            return left_rank.cmp(right_rank);
        }
    }
    left.transaction
        .traded_at
        .cmp(&right.transaction.traded_at)
        .then_with(|| {
            left.transaction
                .created_at
                .cmp(&right.transaction.created_at)
        })
        .then_with(|| left.transaction.id.cmp(&right.transaction.id))
}

fn same_action_group(
    previous: &ActionFill<'_>,
    transaction: &Transaction,
    trade_date: NaiveDate,
    side: &str,
    is_transfer: bool,
) -> bool {
    previous.record.transaction.account_id == transaction.account_id
        && stock_securities_equal(
            &previous.record.transaction.symbol,
            &previous.record.transaction.market,
            &transaction.symbol,
            &transaction.market,
        )
        && previous.trade_date == trade_date
        && previous.side == side
        && previous.record.is_transfer == is_transfer
}

fn flush_action_fills(fills: &mut Vec<ActionFill<'_>>, actions: &mut Vec<StockActionReview>) {
    if fills.is_empty() {
        return;
    }
    let first = &fills[0];
    let shares = fills
        .iter()
        .map(|fill| fill.record.transaction.shares)
        .sum::<f64>();
    let weighted_total = fills
        .iter()
        .map(|fill| fill.record.transaction.shares * fill.record.transaction.price)
        .sum::<f64>();
    let gross_amount = fills
        .iter()
        .map(|fill| fill.record.transaction.total_amount)
        .sum::<f64>();
    let fees = fills
        .iter()
        .map(|fill| fill.record.transaction.commission)
        .sum::<f64>();
    let transaction_ids = fills
        .iter()
        .map(|fill| fill.record.transaction.id.clone())
        .collect::<Vec<_>>();
    let is_transfer = fills.iter().any(|fill| fill.record.is_transfer);
    actions.push(StockActionReview {
        action_id: action_id(first.record.transaction, first.trade_date, first.side),
        transaction_ids,
        account_id: first.record.transaction.account_id.clone(),
        symbol: first.record.transaction.symbol.clone(),
        market: first.record.transaction.market.clone(),
        action_type: first.action_type.clone(),
        traded_at: first.record.transaction.traded_at.clone(),
        weighted_average_price: (shares.abs() > EPSILON).then_some(weighted_total / shares),
        gross_amount: Some(gross_amount),
        currency: Some(first.record.transaction.currency.clone()),
        shares_before: Some(first.shares_before),
        shares_after: fills.last().map(|fill| fill.shares_after),
        portfolio_weight_before: None,
        portfolio_weight_after: None,
        fees: Some(fees),
        contribution: None,
        observation_windows: Vec::<ForwardEffectWindow>::new(),
        status: MetricStatus::Pending,
        fact_labels: if is_transfer {
            vec!["transfer".to_string()]
        } else {
            Vec::new()
        },
    });
    fills.clear();
}

fn duplicate_issues(ids: &HashSet<String>, transactions: &[Transaction]) -> Vec<StockReviewIssue> {
    transactions
        .iter()
        .filter(|transaction| ids.contains(&transaction.id))
        .map(|transaction| {
            issue(
                "source_ledger_conflict",
                StockReviewIssueSeverity::Error,
                "A duplicate override excludes this transaction; attribution and shadow metrics require ledger review.",
                transaction,
                trade_date(&transaction.traded_at),
            )
        })
        .collect()
}

fn issue(
    code: &str,
    severity: StockReviewIssueSeverity,
    message: &str,
    transaction: &Transaction,
    date: Option<NaiveDate>,
) -> StockReviewIssue {
    StockReviewIssue {
        code: code.to_string(),
        severity,
        message: message.to_string(),
        affected_symbol: Some(transaction.symbol.clone()),
        affected_date: date,
    }
}

fn parse_ids(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

fn symbol_key(symbol: &str) -> String {
    normalized_stock_symbol(symbol).unwrap_or_else(|| symbol.trim().to_string())
}

fn market_key(market: &str) -> String {
    normalized_stock_market(market).unwrap_or_else(|| market.trim().to_string())
}

fn trade_date(traded_at: &str) -> Option<NaiveDate> {
    traded_at
        .get(..10)
        .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
}

fn is_date_only(traded_at: &str) -> bool {
    traded_at.len() == 10 && trade_date(traded_at).is_some()
}

fn action_id(transaction: &Transaction, date: NaiveDate, side: &str) -> String {
    format!(
        "action:{}:{}:{}:{}:{}",
        escape_action_component(&transaction.account_id),
        escape_action_component(&transaction.symbol),
        date.format("%Y-%m-%d"),
        side,
        escape_action_component(&transaction.id),
    )
}

fn escape_action_component(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::build_stock_actions;
    use crate::models::stock_review::StockReviewOverride;
    use crate::models::Transaction;

    fn transaction(
        id: &str,
        account_id: &str,
        symbol: &str,
        transaction_type: &str,
        shares: f64,
        price: f64,
        traded_at: &str,
    ) -> Transaction {
        transaction_in_market(
            id,
            account_id,
            symbol,
            "US",
            transaction_type,
            shares,
            price,
            traded_at,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn transaction_in_market(
        id: &str,
        account_id: &str,
        symbol: &str,
        market: &str,
        transaction_type: &str,
        shares: f64,
        price: f64,
        traded_at: &str,
    ) -> Transaction {
        Transaction {
            id: id.to_string(),
            holding_id: None,
            account_id: account_id.to_string(),
            symbol: symbol.to_string(),
            name: symbol.to_string(),
            market: market.to_string(),
            transaction_type: transaction_type.to_string(),
            shares,
            price,
            total_amount: shares * price,
            commission: 0.0,
            currency: if market.eq_ignore_ascii_case("CN") {
                "CNY".to_string()
            } else {
                "USD".to_string()
            },
            traded_at: traded_at.to_string(),
            notes: None,
            created_at: format!("{}T00:00:00Z", &traded_at[..10]),
        }
    }

    #[test]
    fn merges_same_day_same_direction_fills_with_weighted_price() {
        // A regression to one-action-per-fill, or using an unweighted average,
        // must make this fail.
        let mut first = transaction(
            "buy-1",
            "acct:1",
            "BRK/B",
            "BUY",
            40.0,
            100.0,
            "2024-01-02T09:31:00Z",
        );
        first.commission = 1.0;
        let mut second = transaction(
            "buy-2",
            "acct:1",
            "BRK/B",
            "BUY",
            60.0,
            110.0,
            "2024-01-02T09:32:00Z",
        );
        second.commission = 2.0;

        let result = build_stock_actions(&[second, first], &[]);

        assert_eq!(result.actions.len(), 1);
        let action = &result.actions[0];
        assert_eq!(
            action.action_id,
            "action:acct%3A1:BRK%2FB:2024-01-02:buy:buy-1"
        );
        assert_eq!(action.transaction_ids, vec!["buy-1", "buy-2"]);
        assert_eq!(action.action_type, "open");
        assert_eq!(action.weighted_average_price, Some(106.0));
        assert_eq!(action.gross_amount, Some(10600.0));
        assert_eq!(action.fees, Some(3.0));
        assert_eq!(action.shares_before, Some(0.0));
        assert_eq!(action.shares_after, Some(100.0));
    }

    #[test]
    fn classifies_open_add_reduce_close_from_position_path() {
        // A wrong long-position state transition must make this fail.
        let transactions = vec![
            transaction(
                "open",
                "acct",
                "AAPL",
                "BUY",
                10.0,
                100.0,
                "2024-01-02T09:30:00Z",
            ),
            transaction(
                "add",
                "acct",
                "AAPL",
                "BUY",
                5.0,
                110.0,
                "2024-01-03T09:30:00Z",
            ),
            transaction(
                "reduce",
                "acct",
                "AAPL",
                "SELL",
                7.0,
                120.0,
                "2024-01-04T09:30:00Z",
            ),
            transaction(
                "close",
                "acct",
                "AAPL",
                "SELL",
                8.0,
                130.0,
                "2024-01-05T09:30:00Z",
            ),
        ];

        let result = build_stock_actions(&transactions, &[]);

        assert_eq!(
            result
                .actions
                .iter()
                .map(|action| action.action_type.as_str())
                .collect::<Vec<_>>(),
            vec!["open", "add", "reduce", "close"]
        );
        assert_eq!(
            result
                .actions
                .iter()
                .map(|action| (action.shares_before, action.shares_after))
                .collect::<Vec<_>>(),
            vec![
                (Some(0.0), Some(10.0)),
                (Some(10.0), Some(15.0)),
                (Some(15.0), Some(8.0)),
                (Some(8.0), Some(0.0))
            ]
        );
    }

    #[test]
    fn excludes_cash_pay_and_synthetic_open_from_review_actions() {
        // Accidentally treating cash, dividends, or imported opening balances as
        // investable review actions must make this fail.
        let transactions = vec![
            transaction(
                "cash",
                "acct",
                "$CASH-USD",
                "BUY",
                1000.0,
                1.0,
                "2024-01-02T09:00:00Z",
            ),
            transaction(
                "pay",
                "acct",
                "AAPL",
                "PAY",
                0.0,
                0.0,
                "2024-01-02T10:00:00Z",
            ),
            transaction(
                "opening",
                "acct",
                "AAPL",
                "OPEN",
                10.0,
                90.0,
                "2024-01-02T11:00:00Z",
            ),
            transaction(
                "buy",
                "acct",
                "AAPL",
                "BUY",
                5.0,
                100.0,
                "2024-01-03T09:30:00Z",
            ),
        ];

        let result = build_stock_actions(&transactions, &[]);

        assert_eq!(result.actions.len(), 1);
        assert_eq!(result.actions[0].transaction_ids, vec!["buy"]);
        assert_eq!(result.actions[0].action_type, "add");
        assert_eq!(
            result
                .position_events
                .iter()
                .map(|event| event.transaction_id.as_str())
                .collect::<Vec<_>>(),
            vec!["opening", "buy"]
        );
    }

    #[test]
    fn date_only_reversal_is_kept_and_marked_order_uncertain() {
        // Collapsing a same-date reversal or silently trusting an arbitrary
        // date-only order must make this fail.
        let transactions = vec![
            transaction("buy", "acct", "AAPL", "BUY", 10.0, 100.0, "2024-01-02"),
            transaction("sell", "acct", "AAPL", "SELL", 10.0, 110.0, "2024-01-02"),
        ];

        let result = build_stock_actions(&transactions, &[]);

        assert_eq!(
            result
                .actions
                .iter()
                .map(|action| action.action_type.as_str())
                .collect::<Vec<_>>(),
            vec!["open", "close"]
        );
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "same_day_order_uncertain"));
    }

    #[test]
    fn mixed_case_same_day_order_replays_one_symbol_and_uses_confirmed_order() {
        // Case-sensitive position keys or ordering checks would replay the
        // SELL first and ignore the confirmed order for one economic symbol.
        let transactions = vec![
            transaction("open", "acct", "AAPL", "BUY", 10.0, 100.0, "2024-01-01"),
            transaction("a-sell", "acct", "AAPL", "SELL", 15.0, 110.0, "2024-01-02"),
            transaction("z-buy", "acct", "aapl", "BUY", 5.0, 105.0, "2024-01-02"),
        ];
        let overrides = vec![override_record(
            "order",
            "same_day_order",
            &["a-sell", "z-buy"],
            r#"["z-buy","a-sell"]"#,
        )];

        let result = build_stock_actions(&transactions, &overrides);

        assert_eq!(
            result
                .actions
                .iter()
                .map(|action| action.action_type.as_str())
                .collect::<Vec<_>>(),
            vec!["open", "add", "close"]
        );
        assert!(!result
            .issues
            .iter()
            .any(|issue| issue.code == "negative_position"));
        assert!(!result
            .issues
            .iter()
            .any(|issue| issue.code == "same_day_order_uncertain"));
        assert_eq!(result.actions[1].symbol, "aapl");
    }

    #[test]
    fn mixed_case_duplicate_and_non_trade_overrides_keep_id_based_exclusions_intact() {
        // Symbol-key normalization must not broaden ID-scoped duplicate or
        // non-trade exclusions to unrelated records that only share a symbol.
        let transactions = vec![
            transaction("duplicate", "acct", "aapl", "BUY", 1.0, 100.0, "2024-01-01"),
            transaction("non-trade", "acct", "MsFt", "BUY", 1.0, 100.0, "2024-01-01"),
            transaction("kept", "acct", "AAPL", "BUY", 2.0, 100.0, "2024-01-02"),
        ];
        let overrides = vec![
            override_record("duplicate", "duplicate", &["duplicate"], "{}"),
            override_record("non-trade", "non_trade", &["non-trade"], "{}"),
        ];
        let result = build_stock_actions(&transactions, &overrides);
        assert_eq!(result.actions.len(), 1);
        assert_eq!(result.actions[0].transaction_ids, vec!["kept"]);
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "source_ledger_conflict"));
    }

    #[test]
    fn applies_overrides_without_mutating_source_transactions() {
        // Ignoring an override, mutating the input ledger, or failing to flag a
        // duplicate ledger conflict must make this fail.
        let transactions = vec![
            transaction(
                "non-trade",
                "acct",
                "MSFT",
                "BUY",
                1.0,
                50.0,
                "2024-01-01T09:30:00Z",
            ),
            transaction("buy", "acct", "AAPL", "BUY", 10.0, 100.0, "2024-01-02"),
            transaction(
                "duplicate",
                "acct",
                "AAPL",
                "BUY",
                10.0,
                100.0,
                "2024-01-02",
            ),
            transaction("sell", "acct", "AAPL", "SELL", 10.0, 110.0, "2024-01-02"),
            transaction(
                "transfer",
                "other",
                "AAPL",
                "BUY",
                5.0,
                120.0,
                "2024-01-03T09:30:00Z",
            ),
        ];
        let overrides = vec![
            override_record(
                "order",
                "same_day_order",
                &["buy", "sell"],
                r#"["buy","sell"]"#,
            ),
            override_record("non-trade", "non_trade", &["non-trade"], "{}"),
            override_record("duplicate", "duplicate", &["duplicate"], "{}"),
            override_record("transfer", "transfer", &["transfer"], "{}"),
        ];

        let result = build_stock_actions(&transactions, &overrides);

        assert_eq!(
            transactions
                .iter()
                .map(|transaction| transaction.id.as_str())
                .collect::<Vec<_>>(),
            vec!["non-trade", "buy", "duplicate", "sell", "transfer"]
        );
        assert_eq!(
            result
                .actions
                .iter()
                .map(|action| action.action_type.as_str())
                .collect::<Vec<_>>(),
            vec!["open", "close", "open"]
        );
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "source_ledger_conflict"));
        assert!(!result
            .position_events
            .iter()
            .any(|event| event.transaction_id == "non-trade"));
        assert!(
            result
                .position_events
                .iter()
                .find(|event| event.transaction_id == "transfer")
                .unwrap()
                .is_transfer
        );
        assert!(!result
            .issues
            .iter()
            .any(|issue| issue.code == "same_day_order_uncertain"));
    }

    #[test]
    fn duplicate_override_reports_conflict_when_its_trade_date_is_malformed() {
        // Dropping an excluded duplicate just because its date is malformed
        // would hide a source-ledger conflict from the report.
        let transactions = vec![transaction(
            "duplicate",
            "acct",
            "AAPL",
            "BUY",
            10.0,
            100.0,
            "not-a-date",
        )];
        let overrides = vec![override_record(
            "duplicate",
            "duplicate",
            &["duplicate"],
            "{}",
        )];

        let result = build_stock_actions(&transactions, &overrides);

        let conflict = result
            .issues
            .iter()
            .find(|issue| issue.code == "source_ledger_conflict")
            .expect("duplicate conflicts must be reported regardless of date quality");
        assert_eq!(conflict.affected_date, None);
    }

    #[test]
    fn partial_same_day_order_override_keeps_reversal_order_uncertain() {
        // Treating one ranked leg as confirmation of a two-leg reversal would
        // hide the remaining ordering ambiguity.
        let transactions = vec![
            transaction("buy", "acct", "AAPL", "BUY", 10.0, 100.0, "2024-01-02"),
            transaction("sell", "acct", "AAPL", "SELL", 10.0, 110.0, "2024-01-02"),
        ];
        let overrides = vec![override_record(
            "partial-order",
            "same_day_order",
            &["buy", "sell"],
            r#"["buy"]"#,
        )];

        let result = build_stock_actions(&transactions, &overrides);

        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "same_day_order_uncertain"));
    }

    #[test]
    fn same_day_rank_never_reorders_different_market_positions() {
        // Omitting market from the rank guard lets a malformed cross-market
        // override reorder otherwise independent positions.
        let transactions = vec![
            transaction_in_market(
                "a-us",
                "acct",
                "AAPL",
                "US",
                "BUY",
                1.0,
                100.0,
                "2024-01-02",
            ),
            transaction_in_market("z-cn", "acct", "aapl", "CN", "BUY", 2.0, 50.0, "2024-01-02"),
        ];
        let overrides = vec![override_record(
            "bad-cross-market-order",
            "same_day_order",
            &["a-us", "z-cn"],
            r#"["z-cn","a-us"]"#,
        )];

        let result = build_stock_actions(&transactions, &overrides);

        assert_eq!(result.actions.len(), 2);
        assert_eq!(result.actions[0].transaction_ids, vec!["a-us"]);
        assert_eq!(result.actions[0].market, "US");
        assert_eq!(result.actions[0].action_type, "open");
        assert_eq!(result.actions[1].transaction_ids, vec!["z-cn"]);
        assert_eq!(result.actions[1].market, "CN");
        assert_eq!(result.actions[1].action_type, "open");
    }

    #[test]
    fn malformed_cross_market_duplicate_override_does_not_exclude_either_security() {
        // The deterministic core must not apply a duplicate disposition across
        // markets even if an unvalidated fixture reaches this pure boundary.
        let transactions = vec![
            transaction_in_market(
                "us-buy",
                "acct",
                "AAPL",
                "US",
                "BUY",
                1.0,
                100.0,
                "2024-01-02T09:30:00Z",
            ),
            transaction_in_market(
                "cn-buy",
                "acct",
                "aapl",
                "CN",
                "BUY",
                1.0,
                100.0,
                "2024-01-02T09:31:00Z",
            ),
        ];
        let overrides = vec![override_record(
            "bad-cross-market-duplicate",
            "duplicate",
            &["us-buy", "cn-buy"],
            "{}",
        )];

        let result = build_stock_actions(&transactions, &overrides);

        assert_eq!(result.actions.len(), 2);
        assert!(result.corrected_transactions.iter().all(|row| {
            row.transaction.id != "us-buy" && row.transaction.id != "cn-buy" || row.has_cash_effect
        }));
        assert!(!result
            .issues
            .iter()
            .any(|issue| issue.code == "source_ledger_conflict"));
    }

    fn override_record(
        id: &str,
        override_type: &str,
        transaction_ids: &[&str],
        value_json: &str,
    ) -> StockReviewOverride {
        StockReviewOverride {
            id: id.to_string(),
            override_type: override_type.to_string(),
            transaction_ids_json: serde_json::to_string(transaction_ids).unwrap(),
            value_json: value_json.to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }
}
