#[cfg(test)]
mod tests {
    use super::build_stock_campaigns;
    use crate::models::stock_review::{
        StockCampaignStatus, StockReviewIssueSeverity, StockReviewOverride,
    };
    use crate::models::Transaction;
    use crate::services::stock_action_builder::build_stock_actions;
    use chrono::NaiveDate;

    fn transaction(
        id: &str,
        account_id: &str,
        symbol: &str,
        transaction_type: &str,
        shares: f64,
        traded_at: &str,
    ) -> Transaction {
        Transaction {
            id: id.to_string(),
            holding_id: None,
            account_id: account_id.to_string(),
            symbol: symbol.to_string(),
            name: symbol.to_string(),
            market: "US".to_string(),
            transaction_type: transaction_type.to_string(),
            shares,
            price: 100.0,
            total_amount: shares * 100.0,
            commission: 0.0,
            currency: "USD".to_string(),
            traded_at: traded_at.to_string(),
            notes: None,
            created_at: format!("{}T00:00:00Z", &traded_at[..10]),
        }
    }

    #[test]
    fn derives_completed_active_and_reentered_account_fragments() {
        // Merging a later re-entry into the completed position, failing to end
        // at zero, or reporting the open position as completed must fail here.
        let action_result = build_stock_actions(
            &[
                transaction(
                    "open-1",
                    "acct:A",
                    "BRK/B",
                    "BUY",
                    10.0,
                    "2024-01-02T09:30:00Z",
                ),
                transaction(
                    "add-1",
                    "acct:A",
                    "BRK/B",
                    "BUY",
                    5.0,
                    "2024-01-03T09:30:00Z",
                ),
                transaction(
                    "reduce-1",
                    "acct:A",
                    "BRK/B",
                    "SELL",
                    9.0,
                    "2024-01-04T09:30:00Z",
                ),
                transaction(
                    "close-1",
                    "acct:A",
                    "BRK/B",
                    "SELL",
                    6.0,
                    "2024-01-05T09:30:00Z",
                ),
                transaction(
                    "open-2",
                    "acct:A",
                    "BRK/B",
                    "BUY",
                    4.0,
                    "2024-02-01T09:30:00Z",
                ),
                transaction(
                    "add-2",
                    "acct:A",
                    "BRK/B",
                    "BUY",
                    3.0,
                    "2024-02-02T09:30:00Z",
                ),
            ],
            &[],
        );

        let result = build_stock_campaigns(
            &action_result.position_events,
            &action_result.actions,
            &[],
            NaiveDate::from_ymd_opt(2024, 2, 10).unwrap(),
        );

        assert_eq!(result.fragments.len(), 2);
        assert_eq!(result.campaigns.len(), 2);
        assert_eq!(
            result.fragments[0].fragment_id,
            "campaign:acct%3AA:BRK%2FB:open-1"
        );
        assert_eq!(
            result.fragments[0].logical_campaign_id,
            result.fragments[0].fragment_id
        );
        assert_eq!(result.fragments[0].status, StockCampaignStatus::Completed);
        assert_eq!(result.fragments[0].started_at, "2024-01-02T09:30:00Z");
        assert_eq!(
            result.fragments[0].ended_at,
            Some("2024-01-05T09:30:00Z".to_string())
        );
        assert_eq!(
            result.fragments[0].action_ids,
            vec![
                "action:acct%3AA:BRK%2FB:2024-01-02:buy:open-1",
                "action:acct%3AA:BRK%2FB:2024-01-03:buy:add-1",
                "action:acct%3AA:BRK%2FB:2024-01-04:sell:reduce-1",
                "action:acct%3AA:BRK%2FB:2024-01-05:sell:close-1",
            ]
        );
        assert_eq!(
            result.fragments[1].fragment_id,
            "campaign:acct%3AA:BRK%2FB:open-2"
        );
        assert_eq!(result.fragments[1].status, StockCampaignStatus::Active);
        assert_eq!(result.fragments[1].ended_at, None);
        assert_eq!(
            result.fragments[1].action_ids,
            vec![
                "action:acct%3AA:BRK%2FB:2024-02-01:buy:open-2",
                "action:acct%3AA:BRK%2FB:2024-02-02:buy:add-2",
            ]
        );
        assert_eq!(
            result.action_campaign_ids["action:acct%3AA:BRK%2FB:2024-02-01:buy:open-2"],
            "campaign:acct%3AA:BRK%2FB:open-2"
        );
    }

    #[test]
    fn keeps_position_active_when_a_later_close_is_after_as_of() {
        // Replaying a close after the requested reporting date would make the
        // historical campaign lifecycle incorrect and must make this fail.
        let action_result = build_stock_actions(
            &[
                transaction("open", "acct", "AAPL", "BUY", 10.0, "2024-01-02T09:30:00Z"),
                transaction(
                    "future-close",
                    "acct",
                    "AAPL",
                    "SELL",
                    10.0,
                    "2024-02-01T09:30:00Z",
                ),
            ],
            &[],
        );

        let result = build_stock_campaigns(
            &action_result.position_events,
            &action_result.actions,
            &[],
            NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
        );

        assert_eq!(result.fragments.len(), 1);
        assert_eq!(result.fragments[0].status, StockCampaignStatus::Active);
        assert_eq!(result.fragments[0].ended_at, None);
        assert_eq!(
            result.fragments[0].action_ids,
            vec!["action:acct:AAPL:2024-01-02:buy:open"]
        );
    }

    #[test]
    fn connects_confirmed_cross_account_transfer_without_investment_transfer_actions() {
        // Treating a paired transfer as an investment close/open, or leaving
        // its account fragments disconnected, must make this fail.
        let overrides = vec![override_record("move:1", &["a-out", "b-in"])];
        let action_result = build_stock_actions(
            &[
                transaction(
                    "a-open",
                    "acct-A",
                    "AAPL",
                    "BUY",
                    100.0,
                    "2024-01-02T09:30:00Z",
                ),
                transaction(
                    "a-out",
                    "acct-A",
                    "AAPL",
                    "SELL",
                    100.0,
                    "2024-01-10T09:30:00Z",
                ),
                transaction(
                    "b-in",
                    "acct-B",
                    "AAPL",
                    "BUY",
                    100.0,
                    "2024-01-10T10:30:00Z",
                ),
            ],
            &overrides,
        );

        let result = build_stock_campaigns(
            &action_result.position_events,
            &action_result.actions,
            &overrides,
            NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
        );

        assert_eq!(result.fragments.len(), 2);
        assert_eq!(result.campaigns.len(), 1);
        assert_eq!(
            result.campaigns[0].campaign_id,
            "campaign:transfer:move%3A1"
        );
        assert_eq!(result.campaigns[0].account_ids, vec!["acct-A", "acct-B"]);
        assert_eq!(
            result.campaigns[0].campaign_status,
            StockCampaignStatus::Active
        );
        assert_eq!(
            result.campaigns[0].action_ids,
            vec!["action:acct-A:AAPL:2024-01-02:buy:a-open"]
        );
        assert_eq!(
            result.fragments[0]
                .transfer_out
                .as_ref()
                .unwrap()
                .transaction_id,
            "a-out"
        );
        assert_eq!(
            result.fragments[1]
                .transfer_in
                .as_ref()
                .unwrap()
                .transaction_id,
            "b-in"
        );
        assert_eq!(
            result.action_campaign_ids["action:acct-A:AAPL:2024-01-02:buy:a-open"],
            "campaign:transfer:move%3A1"
        );
        assert!(!result
            .action_campaign_ids
            .contains_key("action:acct-A:AAPL:2024-01-10:sell:a-out"));
        assert!(!result
            .action_campaign_ids
            .contains_key("action:acct-B:AAPL:2024-01-10:buy:b-in"));
    }

    #[test]
    fn mixed_case_confirmed_transfer_links_the_same_economic_symbol() {
        // A persistence-accepted transfer must not be rejected downstream
        // solely because source and destination retain their display casing.
        let overrides = vec![override_record("move-case", &["a-out", "b-in"])];
        let action_result = build_stock_actions(
            &[
                transaction(
                    "a-open",
                    "acct-A",
                    "AAPL",
                    "BUY",
                    10.0,
                    "2024-01-02T09:30:00Z",
                ),
                transaction(
                    "a-out",
                    "acct-A",
                    "AAPL",
                    "SELL",
                    10.0,
                    "2024-01-10T09:30:00Z",
                ),
                transaction(
                    "b-in",
                    "acct-B",
                    "aapl",
                    "BUY",
                    10.0,
                    "2024-01-10T10:30:00Z",
                ),
            ],
            &overrides,
        );
        let result = build_stock_campaigns(
            &action_result.position_events,
            &action_result.actions,
            &overrides,
            NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
        );

        assert_eq!(result.campaigns.len(), 1);
        assert_eq!(
            result.campaigns[0].campaign_id,
            "campaign:transfer:move-case"
        );
        assert!(!result
            .issues
            .iter()
            .any(|issue| issue.code == "invalid_transfer_override"));
        assert_eq!(result.fragments[1].symbol, "aapl");
    }

    #[test]
    fn preserves_ordinary_same_day_fill_when_suppressing_transfer_action() {
        // Grouping a transfer and ordinary buy into one action would suppress
        // the ordinary investment action and must make this fail.
        let overrides = vec![override_record("move", &["a-out", "b-in"])];
        let action_result = build_stock_actions(
            &[
                transaction(
                    "a-open",
                    "acct-A",
                    "AAPL",
                    "BUY",
                    100.0,
                    "2024-01-02T09:30:00Z",
                ),
                transaction(
                    "a-out",
                    "acct-A",
                    "AAPL",
                    "SELL",
                    100.0,
                    "2024-01-10T09:30:00Z",
                ),
                transaction(
                    "b-in",
                    "acct-B",
                    "AAPL",
                    "BUY",
                    100.0,
                    "2024-01-10T09:30:00Z",
                ),
                transaction(
                    "b-ordinary",
                    "acct-B",
                    "AAPL",
                    "BUY",
                    10.0,
                    "2024-01-10T10:30:00Z",
                ),
            ],
            &overrides,
        );

        let result = build_stock_campaigns(
            &action_result.position_events,
            &action_result.actions,
            &overrides,
            NaiveDate::from_ymd_opt(2024, 1, 20).unwrap(),
        );

        assert_eq!(
            action_result
                .actions
                .iter()
                .map(|action| action.action_id.as_str())
                .collect::<Vec<_>>(),
            vec![
                "action:acct-A:AAPL:2024-01-02:buy:a-open",
                "action:acct-A:AAPL:2024-01-10:sell:a-out",
                "action:acct-B:AAPL:2024-01-10:buy:b-in",
                "action:acct-B:AAPL:2024-01-10:buy:b-ordinary",
            ]
        );
        assert_eq!(
            result.campaigns[0].action_ids,
            vec![
                "action:acct-A:AAPL:2024-01-02:buy:a-open",
                "action:acct-B:AAPL:2024-01-10:buy:b-ordinary",
            ]
        );
        assert_eq!(
            result.action_campaign_ids["action:acct-B:AAPL:2024-01-10:buy:b-ordinary"],
            "campaign:transfer:move"
        );
    }

    #[test]
    fn malformed_transfer_override_leaves_fragments_independent_and_reports_error() {
        // Joining a transfer override without two matched legs would merge
        // independent account histories and must make this fail.
        let overrides = vec![override_record("broken", &["a-out"])];
        let action_result = build_stock_actions(
            &[
                transaction(
                    "a-open",
                    "acct-A",
                    "AAPL",
                    "BUY",
                    10.0,
                    "2024-01-02T09:30:00Z",
                ),
                transaction(
                    "a-out",
                    "acct-A",
                    "AAPL",
                    "SELL",
                    10.0,
                    "2024-01-03T09:30:00Z",
                ),
                transaction(
                    "b-open",
                    "acct-B",
                    "AAPL",
                    "BUY",
                    10.0,
                    "2024-01-03T09:30:00Z",
                ),
            ],
            &overrides,
        );

        let result = build_stock_campaigns(
            &action_result.position_events,
            &action_result.actions,
            &overrides,
            NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
        );

        assert_eq!(result.campaigns.len(), 2);
        assert_eq!(
            result
                .campaigns
                .iter()
                .map(|campaign| campaign.campaign_id.as_str())
                .collect::<Vec<_>>(),
            vec!["campaign:acct-A:AAPL:a-open", "campaign:acct-B:AAPL:b-open"]
        );
        assert!(result
            .issues
            .iter()
            .any(|issue| issue.code == "invalid_transfer_override"));
    }

    #[test]
    fn same_symbol_in_two_accounts_without_transfer_stays_separate() {
        // Grouping same-symbol holdings across accounts without a confirmed
        // transfer would erase account boundaries and must make this fail.
        let action_result = build_stock_actions(
            &[
                transaction(
                    "a-open",
                    "acct-A",
                    "AAPL",
                    "BUY",
                    10.0,
                    "2024-01-02T09:30:00Z",
                ),
                transaction(
                    "b-open",
                    "acct-B",
                    "AAPL",
                    "BUY",
                    10.0,
                    "2024-01-02T09:30:00Z",
                ),
            ],
            &[],
        );

        let result = build_stock_campaigns(
            &action_result.position_events,
            &action_result.actions,
            &[],
            NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
        );

        assert_eq!(result.campaigns.len(), 2);
        assert_eq!(result.campaigns[0].account_ids, vec!["acct-A"]);
        assert_eq!(
            result.campaigns[0].campaign_id,
            "campaign:acct-A:AAPL:a-open"
        );
        assert_eq!(result.campaigns[1].account_ids, vec!["acct-B"]);
        assert_eq!(
            result.campaigns[1].campaign_id,
            "campaign:acct-B:AAPL:b-open"
        );
    }

    #[test]
    fn stops_campaign_inference_after_negative_position_path() {
        // Continuing into a later buy after an oversell would invent a
        // campaign history and must make this fail.
        let action_result = build_stock_actions(
            &[
                transaction("open", "acct", "MSFT", "BUY", 10.0, "2024-01-02T09:30:00Z"),
                transaction(
                    "oversell",
                    "acct",
                    "MSFT",
                    "SELL",
                    11.0,
                    "2024-01-03T09:30:00Z",
                ),
                transaction(
                    "later-buy",
                    "acct",
                    "MSFT",
                    "BUY",
                    5.0,
                    "2024-01-04T09:30:00Z",
                ),
            ],
            &[],
        );

        let result = build_stock_campaigns(
            &action_result.position_events,
            &action_result.actions,
            &[],
            NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
        );

        assert!(result.fragments.is_empty());
        assert!(result.campaigns.is_empty());
        assert!(result.action_campaign_ids.is_empty());
        let issue = result
            .issues
            .iter()
            .find(|issue| issue.code == "campaign_unavailable")
            .expect("the invalid position path must be explicit");
        assert_eq!(issue.severity, StockReviewIssueSeverity::Error);
        assert_eq!(issue.affected_symbol, Some("MSFT".to_string()));
        assert_eq!(
            issue.affected_date,
            Some(NaiveDate::from_ymd_opt(2024, 1, 3).unwrap())
        );
    }

    #[test]
    fn synthetic_open_seeds_active_campaign_without_evaluable_action() {
        // Rejecting an imported opening balance would discard a reconstructed
        // holding and must make this fail.
        let action_result = build_stock_actions(
            &[transaction(
                "imported-open",
                "acct",
                "NVDA",
                "OPEN",
                10.0,
                "2024-01-02T09:30:00Z",
            )],
            &[],
        );

        let result = build_stock_campaigns(
            &action_result.position_events,
            &action_result.actions,
            &[],
            NaiveDate::from_ymd_opt(2024, 1, 10).unwrap(),
        );

        assert_eq!(result.fragments.len(), 1);
        assert_eq!(result.fragments[0].status, StockCampaignStatus::Active);
        assert_eq!(result.fragments[0].action_ids, Vec::<String>::new());
        assert_eq!(result.campaigns.len(), 1);
        assert_eq!(result.campaigns[0].action_ids, Vec::<String>::new());
        assert!(result.action_campaign_ids.is_empty());
        assert!(!result
            .issues
            .iter()
            .any(|issue| issue.code == "campaign_unavailable"));
    }

    fn override_record(id: &str, transaction_ids: &[&str]) -> StockReviewOverride {
        StockReviewOverride {
            id: id.to_string(),
            override_type: "transfer".to_string(),
            transaction_ids_json: serde_json::to_string(transaction_ids).unwrap(),
            value_json: "{}".to_string(),
            created_at: "2024-01-01T00:00:00Z".to_string(),
            updated_at: "2024-01-01T00:00:00Z".to_string(),
        }
    }
}
use crate::models::stock_review::{
    normalized_stock_symbol, stock_symbols_equal, AccountCampaignFragment, MetricAvailability,
    MetricStatus, StockActionReview, StockCampaignStatus, StockCampaignSummary,
    StockCampaignTransferFact, StockReviewIssue, StockReviewIssueSeverity, StockReviewOverride,
};
use crate::services::stock_action_builder::PositionEvent;
use chrono::NaiveDate;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

const EPSILON: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub struct CampaignBuildResult {
    pub campaigns: Vec<StockCampaignSummary>,
    pub fragments: Vec<AccountCampaignFragment>,
    pub action_campaign_ids: HashMap<String, String>,
    pub issues: Vec<StockReviewIssue>,
}

#[derive(Debug, Clone)]
struct FragmentBuildState {
    fragment: AccountCampaignFragment,
    event_transaction_ids: Vec<String>,
}

#[derive(Debug, Clone)]
struct TransferLink {
    override_id: String,
    source_fragment: usize,
    destination_fragment: usize,
    source_event: PositionEvent,
    destination_event: PositionEvent,
}

pub fn build_stock_campaigns(
    events: &[PositionEvent],
    actions: &[StockActionReview],
    overrides: &[StockReviewOverride],
    as_of: NaiveDate,
) -> CampaignBuildResult {
    let action_by_transaction = action_ids_by_transaction(actions);
    let retained_events = events
        .iter()
        .filter(|event| event.trade_date <= as_of)
        .cloned()
        .collect::<Vec<_>>();
    let mut events_by_position: BTreeMap<(String, String), Vec<&PositionEvent>> = BTreeMap::new();
    for event in &retained_events {
        events_by_position
            .entry((event.account_id.clone(), symbol_key(&event.symbol)))
            .or_default()
            .push(event);
    }

    let mut fragments = Vec::new();
    let mut issues = Vec::new();
    for ((account_id, _symbol_key), position_events) in events_by_position {
        let symbol = &position_events[0].symbol;
        derive_position_fragments(
            &account_id,
            &symbol,
            &position_events,
            &action_by_transaction,
            &mut fragments,
            &mut issues,
        );
    }

    let transfer_links = valid_transfer_links(overrides, &retained_events, &fragments, &mut issues);
    let transfer_action_ids =
        apply_transfer_links(&mut fragments, &transfer_links, &action_by_transaction);
    for fragment in &mut fragments {
        fragment
            .fragment
            .action_ids
            .retain(|action_id| !transfer_action_ids.contains(action_id));
    }

    let fragment_values = fragments
        .iter()
        .map(|state| state.fragment.clone())
        .collect::<Vec<_>>();
    let campaigns = summarize_campaigns(&fragment_values);
    let action_campaign_ids = fragment_values
        .iter()
        .flat_map(|fragment| {
            fragment
                .action_ids
                .iter()
                .map(move |action_id| (action_id.clone(), fragment.logical_campaign_id.clone()))
        })
        .collect();

    CampaignBuildResult {
        campaigns,
        fragments: fragment_values,
        action_campaign_ids,
        issues,
    }
}

fn derive_position_fragments(
    account_id: &str,
    symbol: &str,
    events: &[&PositionEvent],
    action_by_transaction: &HashMap<String, String>,
    fragments: &mut Vec<FragmentBuildState>,
    issues: &mut Vec<StockReviewIssue>,
) {
    let mut active: Option<FragmentBuildState> = None;
    let mut previous_after = 0.0;
    for event in events {
        let before_is_zero = is_zero(event.shares_before);
        let after_is_zero = is_zero(event.shares_after);
        let is_synthetic_opening = active.is_none()
            && event.transaction_type == "OPEN"
            && before_is_zero
            && !after_is_zero;
        if event.shares_before < -EPSILON
            || event.shares_after < -EPSILON
            || !approximately_equal(event.shares_before, previous_after)
            || !approximately_equal(event.shares_after, event.shares_before + event.shares_delta)
            || (!action_by_transaction.contains_key(&event.transaction_id) && !is_synthetic_opening)
        {
            issues.push(unavailable_issue(event));
            return;
        }
        match (&mut active, before_is_zero, after_is_zero) {
            (None, true, false) => {
                let action_ids = action_by_transaction
                    .get(&event.transaction_id)
                    .cloned()
                    .into_iter()
                    .collect();
                active = Some(FragmentBuildState {
                    fragment: AccountCampaignFragment {
                        fragment_id: fragment_id(account_id, symbol, &event.transaction_id),
                        logical_campaign_id: fragment_id(account_id, symbol, &event.transaction_id),
                        account_id: account_id.to_string(),
                        symbol: symbol.to_string(),
                        market: event.market.clone(),
                        started_at: event.traded_at.clone(),
                        ended_at: None,
                        status: StockCampaignStatus::Active,
                        action_ids,
                        transfer_in: None,
                        transfer_out: None,
                    },
                    event_transaction_ids: vec![event.transaction_id.clone()],
                });
            }
            (Some(fragment), false, false) => append_event(fragment, event, action_by_transaction),
            (Some(_), false, true) => {
                let mut completed = active.take().expect("active fragment must exist");
                append_event(&mut completed, event, action_by_transaction);
                completed.fragment.ended_at = Some(event.traded_at.clone());
                completed.fragment.status = StockCampaignStatus::Completed;
                fragments.push(completed);
            }
            _ => {
                issues.push(unavailable_issue(event));
                return;
            }
        }
        previous_after = event.shares_after;
    }
    if let Some(fragment) = active {
        fragments.push(fragment);
    }
}

fn append_event(
    fragment: &mut FragmentBuildState,
    event: &PositionEvent,
    action_by_transaction: &HashMap<String, String>,
) {
    if let Some(action_id) = action_by_transaction.get(&event.transaction_id) {
        if fragment.fragment.action_ids.last() != Some(action_id) {
            fragment.fragment.action_ids.push(action_id.clone());
        }
    }
    fragment
        .event_transaction_ids
        .push(event.transaction_id.clone());
}

fn valid_transfer_links(
    overrides: &[StockReviewOverride],
    events: &[PositionEvent],
    fragments: &[FragmentBuildState],
    issues: &mut Vec<StockReviewIssue>,
) -> Vec<TransferLink> {
    let events_by_id = events
        .iter()
        .map(|event| (event.transaction_id.as_str(), event))
        .collect::<HashMap<_, _>>();
    let mut links = Vec::new();
    let mut transfer_overrides = overrides
        .iter()
        .filter(|override_record| override_record.override_type == "transfer")
        .collect::<Vec<_>>();
    transfer_overrides.sort_by(|left, right| left.id.cmp(&right.id));
    for override_record in transfer_overrides {
        let ids = parse_ids(&override_record.transaction_ids_json);
        if ids.len() != 2 {
            issues.push(invalid_transfer_issue(override_record, None));
            continue;
        }
        let Some(first) = events_by_id.get(ids[0].as_str()) else {
            issues.push(invalid_transfer_issue(override_record, None));
            continue;
        };
        let Some(second) = events_by_id.get(ids[1].as_str()) else {
            issues.push(invalid_transfer_issue(override_record, None));
            continue;
        };
        let (source, destination) =
            if first.shares_delta < -EPSILON && second.shares_delta > EPSILON {
                (*first, *second)
            } else if second.shares_delta < -EPSILON && first.shares_delta > EPSILON {
                (*second, *first)
            } else {
                issues.push(invalid_transfer_issue(override_record, Some(first)));
                continue;
            };
        let source_fragment = fragment_for_transaction(fragments, &source.transaction_id);
        let destination_fragment = fragment_for_transaction(fragments, &destination.transaction_id);
        if !source.is_transfer
            || !destination.is_transfer
            || source.account_id == destination.account_id
            || !stock_symbols_equal(&source.symbol, &destination.symbol)
            || !approximately_equal(source.shares_delta.abs(), destination.shares_delta.abs())
            || source_fragment.is_none()
            || destination_fragment.is_none()
        {
            issues.push(invalid_transfer_issue(override_record, Some(source)));
            continue;
        }
        links.push(TransferLink {
            override_id: override_record.id.clone(),
            source_fragment: source_fragment.expect("checked above"),
            destination_fragment: destination_fragment.expect("checked above"),
            source_event: source.clone(),
            destination_event: destination.clone(),
        });
    }
    links
}

fn apply_transfer_links(
    fragments: &mut [FragmentBuildState],
    links: &[TransferLink],
    action_by_transaction: &HashMap<String, String>,
) -> HashSet<String> {
    let mut parents = (0..fragments.len()).collect::<Vec<_>>();
    for link in links {
        union(
            &mut parents,
            link.source_fragment,
            link.destination_fragment,
        );
    }
    let mut override_ids_by_root: HashMap<usize, Vec<&str>> = HashMap::new();
    for link in links {
        let root = find(&mut parents, link.source_fragment);
        override_ids_by_root
            .entry(root)
            .or_default()
            .push(&link.override_id);
    }
    for (index, state) in fragments.iter_mut().enumerate() {
        let root = find(&mut parents, index);
        if let Some(override_ids) = override_ids_by_root.get(&root) {
            let override_id = override_ids.iter().min().expect("link group is non-empty");
            state.fragment.logical_campaign_id = transfer_campaign_id(override_id);
        }
    }
    let mut transfer_action_ids = HashSet::new();
    for link in links {
        let source_action_id = action_by_transaction
            .get(&link.source_event.transaction_id)
            .cloned();
        let destination_action_id = action_by_transaction
            .get(&link.destination_event.transaction_id)
            .cloned();
        if let Some(action_id) = &source_action_id {
            transfer_action_ids.insert(action_id.clone());
        }
        if let Some(action_id) = &destination_action_id {
            transfer_action_ids.insert(action_id.clone());
        }
        fragments[link.source_fragment].fragment.transfer_out = Some(StockCampaignTransferFact {
            transaction_id: link.source_event.transaction_id.clone(),
            action_id: source_action_id,
            traded_at: link.source_event.traded_at.clone(),
        });
        fragments[link.destination_fragment].fragment.transfer_in =
            Some(StockCampaignTransferFact {
                transaction_id: link.destination_event.transaction_id.clone(),
                action_id: destination_action_id,
                traded_at: link.destination_event.traded_at.clone(),
            });
    }
    transfer_action_ids
}

fn summarize_campaigns(fragments: &[AccountCampaignFragment]) -> Vec<StockCampaignSummary> {
    let mut grouped: BTreeMap<&str, Vec<&AccountCampaignFragment>> = BTreeMap::new();
    for fragment in fragments {
        grouped
            .entry(&fragment.logical_campaign_id)
            .or_default()
            .push(fragment);
    }
    grouped
        .into_iter()
        .map(|(campaign_id, mut fragments)| {
            fragments.sort_by(|left, right| {
                left.started_at
                    .cmp(&right.started_at)
                    .then_with(|| left.fragment_id.cmp(&right.fragment_id))
            });
            let account_ids = fragments
                .iter()
                .map(|fragment| fragment.account_id.clone())
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>();
            let action_ids = fragments
                .iter()
                .flat_map(|fragment| fragment.action_ids.iter().cloned())
                .collect::<Vec<_>>();
            let campaign_status = if fragments
                .iter()
                .any(|fragment| fragment.status == StockCampaignStatus::Active)
            {
                StockCampaignStatus::Active
            } else {
                StockCampaignStatus::Completed
            };
            let started_at = fragments
                .first()
                .expect("campaign group is non-empty")
                .started_at
                .clone();
            let ended_at = (campaign_status == StockCampaignStatus::Completed).then(|| {
                fragments
                    .iter()
                    .filter_map(|fragment| fragment.ended_at.clone())
                    .max()
                    .expect("completed fragments have end timestamps")
            });
            let first = fragments.first().expect("campaign group is non-empty");
            let symbol = first.symbol.clone();
            let market = first.market.clone();
            let fragment_values = fragments.into_iter().cloned().collect();
            StockCampaignSummary {
                campaign_id: campaign_id.to_string(),
                account_ids,
                action_ids,
                fragments: fragment_values,
                campaign_status,
                availability: MetricAvailability {
                    status: MetricStatus::Available,
                    note: None,
                },
                symbol,
                market,
                started_at,
                ended_at,
                contribution: None,
            }
        })
        .collect()
}

fn action_ids_by_transaction(actions: &[StockActionReview]) -> HashMap<String, String> {
    actions
        .iter()
        .flat_map(|action| {
            action
                .transaction_ids
                .iter()
                .map(move |transaction_id| (transaction_id.clone(), action.action_id.clone()))
        })
        .collect()
}

fn fragment_for_transaction(
    fragments: &[FragmentBuildState],
    transaction_id: &str,
) -> Option<usize> {
    fragments.iter().position(|fragment| {
        fragment
            .event_transaction_ids
            .iter()
            .any(|id| id == transaction_id)
    })
}

fn unavailable_issue(event: &PositionEvent) -> StockReviewIssue {
    StockReviewIssue {
        code: "campaign_unavailable".to_string(),
        severity: StockReviewIssueSeverity::Error,
        message: "Position replay is inconsistent; later campaign inference is unavailable for this account and symbol.".to_string(),
        affected_symbol: Some(event.symbol.clone()),
        affected_date: Some(event.trade_date),
    }
}

fn invalid_transfer_issue(
    override_record: &StockReviewOverride,
    event: Option<&PositionEvent>,
) -> StockReviewIssue {
    StockReviewIssue {
        code: "invalid_transfer_override".to_string(),
        severity: StockReviewIssueSeverity::Error,
        message: format!(
            "Transfer override {} does not connect equal opposite position events in different accounts.",
            override_record.id
        ),
        affected_symbol: event.map(|event| event.symbol.clone()),
        affected_date: event.map(|event| event.trade_date),
    }
}

fn parse_ids(json: &str) -> Vec<String> {
    serde_json::from_str(json).unwrap_or_default()
}

fn symbol_key(symbol: &str) -> String {
    normalized_stock_symbol(symbol).unwrap_or_else(|| symbol.trim().to_string())
}

fn fragment_id(account_id: &str, symbol: &str, opening_event_id: &str) -> String {
    format!(
        "campaign:{}:{}:{}",
        escape_component(account_id),
        escape_component(symbol),
        escape_component(opening_event_id),
    )
}

fn transfer_campaign_id(override_id: &str) -> String {
    format!("campaign:transfer:{}", escape_component(override_id))
}

fn escape_component(value: &str) -> String {
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

fn approximately_equal(left: f64, right: f64) -> bool {
    (left - right).abs() <= EPSILON
}

fn is_zero(value: f64) -> bool {
    value.abs() <= EPSILON
}

fn find(parents: &mut [usize], index: usize) -> usize {
    if parents[index] != index {
        let root = find(parents, parents[index]);
        parents[index] = root;
    }
    parents[index]
}

fn union(parents: &mut [usize], left: usize, right: usize) {
    let left_root = find(parents, left);
    let right_root = find(parents, right);
    if left_root != right_root {
        parents[right_root] = left_root;
    }
}
