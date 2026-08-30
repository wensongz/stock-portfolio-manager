use crate::models::Transaction;
use chrono::NaiveDate;
use std::collections::HashMap;

const EPSILON: f64 = 1e-9;
const CASH_SYMBOL_PREFIX: &str = "$CASH-";

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawStockOperation {
    pub action_id: String,
    pub transaction_ids: Vec<String>,
    pub account_id: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub action_type: String,
    pub traded_at: String,
    pub trade_date: NaiveDate,
    pub quantity: f64,
    pub trade_price: f64,
    pub trade_notional_local: f64,
    pub fee_local: f64,
    pub currency: String,
    pub shares_before: f64,
    pub shares_after: f64,
}

pub(crate) fn normalize_stock_symbol(symbol: &str) -> Option<String> {
    let symbol = symbol.trim();
    (!symbol.is_empty()).then(|| symbol.to_ascii_uppercase())
}

pub(crate) fn normalize_stock_market(market: &str) -> Option<String> {
    let market = market.trim();
    (!market.is_empty()).then(|| market.to_ascii_uppercase())
}

pub(crate) fn stock_securities_equal(
    left_symbol: &str,
    left_market: &str,
    right_symbol: &str,
    right_market: &str,
) -> bool {
    normalize_stock_symbol(left_symbol) == normalize_stock_symbol(right_symbol)
        && normalize_stock_market(left_market) == normalize_stock_market(right_market)
}

pub(crate) fn build_raw_stock_operations(transactions: &[Transaction]) -> Vec<RawStockOperation> {
    let mut ordered = transactions.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.traded_at
            .cmp(&right.traded_at)
            .then_with(|| left.created_at.cmp(&right.created_at))
            .then_with(|| left.id.cmp(&right.id))
    });

    let mut shares_by_position: HashMap<(String, String, String), f64> = HashMap::new();
    let mut actions: Vec<RawStockOperation> = Vec::new();
    let mut active_action_key: Option<(String, String, String, NaiveDate, &'static str)> = None;

    for transaction in ordered {
        if is_cash_symbol(&transaction.symbol) || transaction.transaction_type == "PAY" {
            continue;
        }
        let Some(trade_date) = trade_date(&transaction.traded_at) else {
            continue;
        };
        let (Some(symbol_key), Some(market_key)) = (
            normalize_stock_symbol(&transaction.symbol),
            normalize_stock_market(&transaction.market),
        ) else {
            continue;
        };
        let position_key = (
            transaction.account_id.clone(),
            symbol_key.clone(),
            market_key.clone(),
        );

        if transaction.transaction_type == "OPEN" {
            *shares_by_position.entry(position_key).or_default() += transaction.shares;
            active_action_key = None;
            continue;
        }

        let shares_before = *shares_by_position.get(&position_key).unwrap_or(&0.0);
        let (side, action_type, shares_after) = match transaction.transaction_type.as_str() {
            "BUY" => (
                "buy",
                if shares_before <= EPSILON {
                    "open"
                } else {
                    "add"
                },
                shares_before + transaction.shares,
            ),
            "SELL" => {
                let shares_after = shares_before - transaction.shares;
                if shares_before <= EPSILON || shares_after < -EPSILON {
                    active_action_key = None;
                    continue;
                }
                (
                    "sell",
                    if shares_after <= EPSILON {
                        "close"
                    } else {
                        "reduce"
                    },
                    shares_after,
                )
            }
            _ => continue,
        };
        shares_by_position.insert(position_key, shares_after);

        let action_key = (
            transaction.account_id.clone(),
            symbol_key,
            market_key,
            trade_date,
            side,
        );
        if active_action_key.as_ref() == Some(&action_key) {
            let action = actions
                .last_mut()
                .expect("an active action group must have an action");
            let weighted_total =
                action.trade_price * action.quantity + transaction.shares * transaction.price;
            action.quantity += transaction.shares;
            action.trade_price = weighted_total / action.quantity;
            action.trade_notional_local += transaction.total_amount.abs();
            action.fee_local += transaction.commission;
            action.transaction_ids.push(transaction.id.clone());
            action.shares_after = shares_after;
        } else {
            actions.push(RawStockOperation {
                action_id: action_id(transaction, trade_date, side),
                transaction_ids: vec![transaction.id.clone()],
                account_id: transaction.account_id.clone(),
                symbol: transaction.symbol.clone(),
                name: transaction.name.clone(),
                market: transaction.market.clone(),
                action_type: action_type.to_string(),
                traded_at: transaction.traded_at.clone(),
                trade_date,
                quantity: transaction.shares,
                trade_price: transaction.price,
                trade_notional_local: transaction.total_amount.abs(),
                fee_local: transaction.commission,
                currency: transaction.currency.clone(),
                shares_before,
                shares_after,
            });
        }
        active_action_key = Some(action_key);
    }

    actions
}

fn is_cash_symbol(symbol: &str) -> bool {
    symbol
        .trim()
        .to_ascii_uppercase()
        .starts_with(CASH_SYMBOL_PREFIX)
}

fn trade_date(traded_at: &str) -> Option<NaiveDate> {
    traded_at
        .get(..10)
        .and_then(|date| NaiveDate::parse_from_str(date, "%Y-%m-%d").ok())
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
    use super::{
        build_raw_stock_operations, normalize_stock_market, normalize_stock_symbol,
        stock_securities_equal,
    };
    use crate::models::Transaction;

    fn transaction(
        id: &str,
        transaction_type: &str,
        shares: f64,
        price: f64,
        traded_at: &str,
    ) -> Transaction {
        Transaction {
            id: id.to_string(),
            holding_id: None,
            account_id: "account-1".to_string(),
            symbol: "AAPL".to_string(),
            name: "Apple".to_string(),
            market: "US".to_string(),
            transaction_type: transaction_type.to_string(),
            shares,
            price,
            total_amount: shares * price,
            commission: 1.0,
            currency: "USD".to_string(),
            traded_at: traded_at.to_string(),
            notes: None,
            created_at: traded_at.to_string(),
        }
    }

    #[test]
    fn raw_replay_groups_same_day_fills_and_ignores_non_trade_rows() {
        let rows = vec![
            transaction("opening", "OPEN", 100.0, 10.0, "2026-06-30"),
            transaction("buy-1", "BUY", 20.0, 11.0, "2026-07-02T10:00:00Z"),
            transaction("buy-2", "BUY", 30.0, 12.0, "2026-07-02T11:00:00Z"),
            transaction("dividend", "PAY", 1.0, 5.0, "2026-07-03"),
            transaction("sell", "SELL", 50.0, 13.0, "2026-07-10T10:00:00Z"),
        ];

        let actions = build_raw_stock_operations(&rows);
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].action_type, "add");
        assert_eq!(
            actions[0].transaction_ids,
            vec!["buy-1".to_string(), "buy-2".to_string()],
        );
        assert_eq!(actions[0].quantity, 50.0);
        assert!((actions[0].trade_price - 11.6).abs() < 1e-12);
        assert_eq!(
            (actions[0].shares_before, actions[0].shares_after),
            (100.0, 150.0)
        );
        assert_eq!(actions[1].action_type, "reduce");
        assert_eq!(
            (actions[1].shares_before, actions[1].shares_after),
            (150.0, 100.0)
        );
    }

    #[test]
    fn raw_replay_classifies_position_transitions_as_four_operation_types() {
        let actions = build_raw_stock_operations(&[
            transaction("open", "BUY", 100.0, 10.0, "2026-07-01T10:00:00Z"),
            transaction("add", "BUY", 20.0, 11.0, "2026-07-02T10:00:00Z"),
            transaction("reduce", "SELL", 50.0, 12.0, "2026-07-03T10:00:00Z"),
            transaction("close", "SELL", 70.0, 13.0, "2026-07-04T10:00:00Z"),
        ]);

        assert_eq!(actions.len(), 4);
        assert_eq!(actions[0].action_type, "open");
        assert_eq!(actions[1].action_type, "add");
        assert_eq!(actions[2].action_type, "reduce");
        assert_eq!(actions[3].action_type, "close");
        assert_eq!(
            (actions[3].shares_before, actions[3].shares_after),
            (70.0, 0.0)
        );
    }

    #[test]
    fn raw_replay_keeps_intraday_buy_sell_buy_as_separate_action_groups() {
        let rows = vec![
            transaction("opening", "OPEN", 100.0, 10.0, "2026-07-01"),
            transaction("buy-1", "BUY", 10.0, 11.0, "2026-07-02T10:00:00Z"),
            transaction("sell", "SELL", 20.0, 12.0, "2026-07-02T11:00:00Z"),
            transaction("buy-2", "BUY", 30.0, 13.0, "2026-07-02T12:00:00Z"),
        ];

        let actions = build_raw_stock_operations(&rows);

        assert_eq!(actions.len(), 3);
        assert_eq!(actions[0].transaction_ids, vec!["buy-1".to_string()]);
        assert_eq!(
            (actions[0].shares_before, actions[0].shares_after),
            (100.0, 110.0)
        );
        assert_eq!(actions[1].transaction_ids, vec!["sell".to_string()]);
        assert_eq!(
            (actions[1].shares_before, actions[1].shares_after),
            (110.0, 90.0)
        );
        assert_eq!(actions[2].transaction_ids, vec!["buy-2".to_string()]);
        assert_eq!(
            (actions[2].shares_before, actions[2].shares_after),
            (90.0, 120.0)
        );
    }

    #[test]
    fn raw_replay_rejects_cash_invalid_dates_and_unexplained_sells() {
        let mut opening = transaction("opening", "OPEN", 10.0, 10.0, "2026-06-30");
        opening.symbol = "AAPL".to_string();
        opening.market = "US".to_string();

        let mut normalized_buy = transaction("buy", "BUY", 5.0, 12.0, "2026-07-02");
        normalized_buy.symbol = "aapl".to_string();
        normalized_buy.market = "us".to_string();

        let mut cash_buy = transaction("cash", "BUY", 100.0, 1.0, "2026-07-01");
        cash_buy.symbol = "$CASH-USD".to_string();

        let mut invalid_date_buy = transaction("invalid", "BUY", 3.0, 11.0, "");
        invalid_date_buy.symbol = "AAPL".to_string();

        let mut unexplained_sell = transaction("short", "SELL", 2.0, 10.0, "2026-07-01");
        unexplained_sell.symbol = "MSFT".to_string();

        let actions = build_raw_stock_operations(&[
            cash_buy,
            invalid_date_buy,
            unexplained_sell,
            opening,
            normalized_buy,
        ]);

        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].transaction_ids, vec!["buy".to_string()]);
        assert_eq!(actions[0].symbol, "aapl");
        assert_eq!(
            (actions[0].shares_before, actions[0].shares_after),
            (10.0, 15.0)
        );
        assert_eq!(normalize_stock_symbol(" aapl "), Some("AAPL".to_string()));
        assert_eq!(normalize_stock_market(" us "), Some("US".to_string()));
        assert!(stock_securities_equal("aapl", "us", "AAPL", "US"));
    }
}
