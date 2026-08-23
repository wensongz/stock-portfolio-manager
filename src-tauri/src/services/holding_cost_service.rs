use crate::services::quote_provider_service::market_adjusts_sell_pay_cost;
use rusqlite::Connection;

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct HoldingCostState {
    pub shares: f64,
    pub avg_cost: f64,
}

#[derive(Debug, Clone, Copy)]
pub struct CostTransaction<'a> {
    pub transaction_type: &'a str,
    pub shares: f64,
    pub price: f64,
    pub total_amount: f64,
    pub commission: f64,
}

/// Apply one transaction to a holding cost state.
///
/// Every forward transaction path must call this function so importing,
/// editing and replaying the same transactions always produce the same cost.
pub fn apply_transaction(
    state: HoldingCostState,
    transaction: CostTransaction<'_>,
    adjust_sell_pay_cost: bool,
) -> HoldingCostState {
    match transaction.transaction_type {
        "OPEN" => HoldingCostState {
            shares: transaction.shares,
            avg_cost: transaction.price,
        },
        "BUY" => {
            let shares = state.shares + transaction.shares;
            let avg_cost = if shares > 0.0 {
                (state.shares * state.avg_cost
                    + transaction.shares * transaction.price
                    + transaction.commission)
                    / shares
            } else {
                transaction.price
            };
            HoldingCostState { shares, avg_cost }
        }
        "SELL" => {
            let shares = state.shares - transaction.shares;
            let avg_cost = if adjust_sell_pay_cost {
                if shares > 0.0 {
                    // Net proceeds are total_amount - commission. Only those
                    // proceeds reduce the remaining position's cost basis.
                    (state.shares * state.avg_cost - transaction.total_amount
                        + transaction.commission)
                        / shares
                } else {
                    0.0
                }
            } else {
                state.avg_cost
            };
            HoldingCostState { shares, avg_cost }
        }
        "PAY" => {
            let avg_cost = if adjust_sell_pay_cost && state.shares > 0.0 {
                let net_amount = transaction.total_amount - transaction.commission;
                (state.shares * state.avg_cost - net_amount) / state.shares
            } else {
                state.avg_cost
            };
            HoldingCostState {
                shares: state.shares,
                avg_cost,
            }
        }
        _ => state,
    }
}

/// Apply one transaction using the persisted policy for its market.
pub fn apply_transaction_with_config(
    conn: &Connection,
    market: &str,
    state: HoldingCostState,
    transaction: CostTransaction<'_>,
) -> HoldingCostState {
    let adjust = market_adjusts_sell_pay_cost(conn, market);
    apply_transaction(state, transaction, adjust)
}

/// Replay transactions using the same persisted market policy and the same
/// per-transaction function as incremental updates.
pub fn replay_transactions_with_config<'a, I>(
    conn: &Connection,
    market: &str,
    transactions: I,
) -> HoldingCostState
where
    I: IntoIterator<Item = CostTransaction<'a>>,
{
    let adjust = market_adjusts_sell_pay_cost(conn, market);
    transactions
        .into_iter()
        .fold(HoldingCostState::default(), |state, transaction| {
            apply_transaction(state, transaction, adjust)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn incremental_and_replay_use_identical_sell_commission_formula() {
        let transactions = [
            CostTransaction {
                transaction_type: "BUY",
                shares: 100.0,
                price: 10.0,
                total_amount: 1_000.0,
                commission: 5.0,
            },
            CostTransaction {
                transaction_type: "SELL",
                shares: 40.0,
                price: 15.0,
                total_amount: 600.0,
                commission: 6.0,
            },
            CostTransaction {
                transaction_type: "PAY",
                shares: 0.0,
                price: 0.0,
                total_amount: 30.0,
                commission: 0.0,
            },
        ];

        let incremental = transactions
            .iter()
            .fold(HoldingCostState::default(), |state, transaction| {
                apply_transaction(state, *transaction, true)
            });
        let replayed = transactions
            .into_iter()
            .fold(HoldingCostState::default(), |state, transaction| {
                apply_transaction(state, transaction, true)
            });

        assert_eq!(incremental, replayed);
        assert!((incremental.shares - 60.0).abs() < 1e-9);
        assert!((incremental.avg_cost - 6.35).abs() < 1e-9);
    }
}
