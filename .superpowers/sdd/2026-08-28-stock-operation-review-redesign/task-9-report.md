# Task 9 report: expose deterministic stock review report

## Status

- Complete.
- Implementation commit: `878ca7b` (`feat: expose deterministic stock review report`).
- The pre-existing untracked `node_modules` directory was preserved and not staged.

## Deterministic orchestration boundary

- Added `CachedStockReviewInput` and `CachedCampaignData` as the typed, cached-data dependency boundary.
- `build_stock_review_artifacts` is the single deterministic core. It produces the full report and the campaign-detail map in one pass; `build_stock_review_report_from_cached_data` and `build_stock_campaign_detail_from_cached_data` are projections of that same result. Commands and detail views contain no copied metric logic.
- The core executes the binding sequence: retain pre-period source transactions for opening reconstruction, apply active validated overrides, build actions/campaigns, build actual/shadow/benchmark curves, compute metrics/attribution, merge quality, then attach annotations and manually assessed quarterly history.
- Report action filtering applies to the requested period while campaigns can begin earlier. Annotations and quarterly `decision_quality` entries are emitted only as display context and are never calculation inputs; historical entries are tagged `historical_manual_assessment` and `display_only: true`.
- One service and one query contract cover all accounts or a single account. Market filtering uses the same predicate for transactions and external cash flows, so another market's cash cannot become investment income.

## Live preparation and degradation contract

- The async production path loads database transactions, corrections, annotations, historical notes, snapshots, price/benchmark cache data, and current holdings, then calls the exact deterministic core used by fixtures.
- Cache preparation may attempt network fills, but fill errors do not abort when exact cached data is sufficient. Missing required values remain unavailable at the affected metric boundary; no hidden price or FX forward-fill was introduced.
- Benchmark inputs retain canonical per-market sessions, fixed start weights for mixed benchmarks, and exact endpoint closes. Actual TWR keeps its explicit origin. Actual/shadow comparable return and ending value remain mode-tagged bundles.
- Cash inputs retain explicit daily keys and known zero cash returns. A non-base external flow is converted only with an explicit cached daily portfolio FX observation on or after its date; an unavailable rate makes actual TWR unavailable rather than relabeling the flow as income.
- Base currency is the only implicit FX ratio of `1.0`. Persisted FX uses its real `updated_at` source date and is never backdated. Forward-filled non-base FX is explicit, raises `fx_forward_fill`, and downgrades only dependent quality.
- Campaign OHLC retains expected sessions and missing closes, and non-base OHLC uses explicit daily FX. Current holdings allow a no-trade report to exist. No evaluable action is a precise unavailable issue, while value-add and turnover remain zero only when genuinely known.
- Active overrides alone enter replay. Stale overrides remain surfaced as audit issues. Stable annotation/override IDs and explicit AI-confirmation provenance are preserved.

## Override confirmation transaction boundary

- `confirm_stock_review_override` performs: read-only validation, cached input preparation, in-memory candidate insertion, complete candidate report construction, persistence, then returns that same candidate report.
- Candidate construction failure occurs before `save_override`; the integration test proves zero database side effects.
- `save_override` revalidates inside its write transaction after candidate construction. A concurrent source-ledger change therefore fails persistence instead of saving an override against changed references. The database transaction is intentionally not held across asynchronous cache preparation or network work.

## Command and interface decisions

- Added four Tauri commands with camel-case arguments and snake-case serialized models: `get_stock_review_report`, `get_stock_campaign_detail`, `save_stock_review_annotation`, and `confirm_stock_review_override`.
- Registered all four in the existing invoke handler without removing any handler.
- The application has no shared `AppState` type. Commands follow the existing application convention, `State<Database>`, rather than introducing a parallel state wrapper solely for this feature.
- The query boundary rejects unparseable dates, `start_date > end_date`, unsupported base currencies or markets, blank account/benchmark/campaign identifiers, and nonexistent account IDs with displayable errors. Supported currencies are USD/CNY/HKD and supported market filters are US/CN/HK.
- No dependency or database schema change was required. Existing review models were sufficient; the new cached dependency structures are service-local production/test inputs and do not expand persisted contracts.

## Report contract

- The report contains the five independent summary regions: actual result, rebalance value-add, action quality, campaign effectiveness, and risk structure.
- Actual-ledger results remain visible when benchmark, override replay, attribution, or other data is unavailable.
- Actual, shadow, and benchmark curves share an explicit 100-base alignment origin.
- Methodology records filters, benchmark, base currency, return mode, coverage, and algorithm version.
- Actions, campaigns, issues, annotations, and quarterly historical notes are included alongside the summaries.

## Acceptance scenarios

The deterministic scenario test asserts values plus statuses/issues/actions/campaigns, not merely successful execution:

1. No trades: zero actions/campaigns, known zero value-add/turnover, and precise no-evaluable-actions issue.
2. Buy then rise: positive forward effect and positive action attribution.
3. Sell then fall: positive effective-avoidance contribution.
4. Sell then rise: negative opportunity-loss contribution without judgmental labeling.
5. External deposit: actual and shadow TWR are cash-flow neutral.
6. Confirmed transfer: no investment action, one linked campaign, and zero turnover.
7. Split: no review action, value neutrality, and zero turnover.
8. Dividend: actual and shadow total return both include the income consistently.
9. Recent trade: immature forward window is pending, not zero or failed.
10. Multi-currency: action and currency effects are separate and use explicit FX.
11. Missing target data: only the affected forward metric is unavailable while actual result/risk remain visible.
12. Attribution conservation: explained contribution and ending-value change reconcile with explicit zero residual.

Boundary coverage additionally asserts same-day ordering uncertainty, duplicate-source conflict, fixed mixed-market benchmark weights, halted/delisted exact-session gaps, and zero-fee import guidance. Separate tests cover other-market external-flow exclusion, FX source-date honesty, explicit FX forward-fill degradation, non-base cash-flow conversion, requested-base actual snapshot conversion, shared report/detail core, and failed-candidate zero-side-effect persistence.

## TDD evidence

Initial integration RED, before production implementation:

```text
cargo test --lib stock_review_service::tests::builds_complete_report_from_cached_data -- --nocapture
error[E0432]: unresolved imports `build_stock_review_report_from_cached_data`, `complete_cached_fixture`
exit code: 101
```

Behavior-focused RED/GREEN checkpoints:

```text
market_filter_excludes_other_market_external_cash_flow
left: 1000.0; right: 100.0

cached_fx_is_not_backdated_before_its_source_date
expected: None; actual: Some(0.142857...)

forward_filled_fx_is_explicit_and_degrades_only_dependent_quality
expected: Degraded; actual: Available

non_base_external_flow_uses_cached_daily_fx_or_degrades_honestly
compile RED: missing `external_flows_base_from_db`

actual_snapshot_values_are_converted_to_the_requested_base_currency
left: 1000.0; right: 7000.0
```

Each production correction was followed by its focused GREEN. The first two unexpected test outcomes were test-only: one brittle exact floating-point assertion and one detail-window expectation that omitted the deliberately retained 20-day campaign history. The assertions were corrected without changing production behavior.

Mutation sensitivity is covered by the market-flow predicate RED, the FX source-date RED, and the candidate-persistence integration test: removing each respective guard changes an asserted value/status/database row count.

## Final verification

```text
cargo test --lib commands::review::tests -- --nocapture
1 passed; 0 failed; 0 ignored

cargo test --lib stock_review_service::tests -- --nocapture
10 passed; 0 failed; 0 ignored

cargo test --lib
443 passed; 0 failed; 8 ignored

npm run build
TypeScript and Vite production build passed; existing large-chunk warning only

git diff --check
exit code: 0

rustfmt --edition 2021 --check src-tauri/src/services/stock_review_service.rs src-tauri/src/services/mod.rs src-tauri/src/commands/review.rs
exit code: 0
```

## Files

- `src-tauri/src/services/stock_review_service.rs`
- `src-tauri/src/services/mod.rs`
- `src-tauri/src/commands/review.rs`
- `src-tauri/src/lib.rs`
- `.superpowers/sdd/2026-08-28-stock-operation-review-redesign/task-9-report.md`

## Concerns for Task 10

- The existing database does not contain a complete account-scoped historical daily cash ledger plus all actual/shadow replay inputs. The live service keeps precise attribution unavailable when those inputs cannot be reconstructed; deterministic cached fixtures prove the full calculation path without inventing data.
- The current quote candle provider contract does not reliably expose adjusted closes and dividend fields. Live value-add is therefore price-only or unavailable according to the established return-mode contract until providers supply total-return-quality data.
- Static/latest exchange rates remain cache-fill hints only and are never silently backdated. Historical FX absence must stay visible in UI quality/issue regions.
- Task 10 should present the five status regions independently, preserve actual-ledger visibility during benchmark/replay gaps, display `historical_manual_assessment` as manual context, and require explicit confirmation before choosing the AI-confirmed annotation context or calling override confirmation.

## Fix round 1: live replay materialization

This section supersedes the original report's limitations where the live path previously supplied empty Task 5/6/7 inputs.

- Live replay implementation: `f36dcb7` (`fix: materialize live stock review replay`).
- Opening-position follow-up: `a1774c3` (`fix: keep in-period buys out of opening holdings`).
- The pre-existing untracked `node_modules` directory remained untouched and unstaged.

### Production boundary and interface decisions

- `prepare_cached_stock_review_input` now materializes the same `CachedStockReviewInput` consumed by the deterministic report/detail core: exact opening holdings and cash, actual and shadow daily balances, exact prices, daily FX, explicit zero cash-return keys, fees, recorded splits, recorded PAY dividends, corrected action batches, attribution, and reliable full-portfolio risk snapshots where the database supports them.
- Opening cash replays all pre-origin cash effects through the application's canonical `cash_delta`, including deposits, withdrawals, BUY, SELL, PAY, OPEN, and fees. A current authoritative cash holding is unwound with every later database transaction, including transactions after a historical report cutoff. If neither a complete cash ledger anchor nor authoritative current cash exists, `opening_cash_incomplete` disables shadow and opening-weight-dependent mixed benchmark/value-add outputs while leaving actual-ledger results visible.
- Current stock holdings are used as opening positions only for account/symbol keys with no source-ledger position events. A holding created by an in-period BUY is never projected backward; no-history legacy holdings still produce a no-trade result.
- Recorded `stock_splits` become replay/attribution events. PAY rows with positive shares become explicit per-share dividend events. Otherwise the existing adjusted-close/price-only return-mode contract degrades total-return-dependent regions honestly.
- Daily attribution now carries actual and shadow positions/cash, exact daily price and FX keys, corrected action batches, splits, dividends, fees, and one unique explicit `0.0` cash-return observation per date/currency. Base currency remains the only implicit FX ratio. Static FX retains its source date and may only be used by the existing explicit forward-fill contract.
- Campaign inputs are keyed by logical `campaign_id`, account set, and campaign lifetime rather than symbol. They include pre-period actions, lifetime-scoped flows, local-market benchmark sessions, exact report-cutoff terminal prices, issues, and relevant annotations. Two cycles of one symbol and the same symbol in two accounts remain separate.
- A stock's designated broad-market cache (`^GSPC`, `000300.SS`, or `^HSI`) supplies local session authority for forward/Campaign calculations. The user-selected benchmark remains a portfolio-result comparison only. Sparse stock quotes never define sessions, exact endpoint holes stay unavailable, and the report emits `derived_market_calendar_authority` because this repository has no first-class exchange-calendar table.
- Risk turnover and fees are built from corrected replay actions. Confirmed transfers, splits, and non-trade corrections do not become turnover, actions, or new campaigns.
- Forward 60/120 statuses and maturity now feed `QualityInput` directly. Pending and unavailable endpoint states propagate to forward quality while actual result, risk, attribution, and other status regions remain independent.
- Override confirmation now normalizes and validates once, captures the stored override-set revision and source fingerprints, inserts the exact canonical record only in memory, builds a complete candidate report, rechecks revision/fingerprints transactionally, then persists and returns that same candidate. Candidate-build failure has no write; a concurrent stored override or referenced-source change rejects the candidate.
- The general Tauri annotation command no longer accepts caller-controlled AI authority. It always persists user provenance. `AiAfterExplicitUserConfirmation` is reachable only through a crate-private typed capability boundary reserved for Task 10's real confirmation interaction.
- `StockCampaignDetail` gained only one required contract field, `issues`, so Campaign-specific missing-session/price/FX problems remain displayable. No dependency or schema change was introduced.

### DB/cache-backed acceptance coverage

The live preparation tests assert values, status, issue, action, and campaign behavior rather than mere completion:

1. Deposit 1000, then pre-period BUY 600 plus fee 1 reconstructs opening cash as 399.
2. Authoritative current cash is unwound past a historical report cutoff to its exact origin value.
3. Incomplete opening cash emits `opening_cash_incomplete`, suppresses shadow/mixed fixed benchmark/value-add, and preserves actual result.
4. A recorded 1:2 split preserves shadow ending value and zero return.
5. An in-period BUY present in current holdings is excluded from opening positions, while a no-history holding produces available risk, one-third cash, and known-zero value-add/turnover.
6. PAY dividends and exact daily FX populate shadow and attribution; explicit currency contribution is known zero for a fully CNY-funded CNY trade while action contribution is separately positive.
7. Two AAPL cycles plus a second account remain three logical campaigns; the closed cycle includes pre-period actions, account-scoped flows/annotations, and no price beyond the historical cutoff.
8. A selected QQQ portfolio benchmark cannot replace the AAPL local `^GSPC` sessions; a missing exact stock close makes the 60-session effect and forward-quality region unavailable.
9. A confirmed cross-account transfer yields zero displayed actions, one campaign, zero turnover, and unique date/currency cash-return keys.
10. A successful canonical `non_trade` preview already has zero actions before persistence; a genuine after-in-memory-insertion candidate failure writes zero override rows.
11. A concurrent override-set mutation invalidates a prepared candidate and leaves only the concurrent row.
12. Recent observations propagate Pending, missing endpoints propagate Unavailable, and unrelated quality regions keep their own statuses.

The original deterministic 12-scenario matrix remains intact, including same-day uncertainty, duplicate conflict, fixed benchmark, suspension/delisting, and fee `0.0` coverage.

### TDD RED/GREEN and mutation evidence

The initial DB/cache-backed RED suite failed at the live boundary before production edits:

```text
opening cash: actual 1000, expected 399
recorded splits: actual 0, expected 1
no-trade value-add: actual None, expected Some(0)
dividend events: actual 0; attribution batches empty
local benchmark: selected QQQ close 200 used, expected ^GSPC close 100
historical first Campaign actions: actual 0, expected 2
confirmed-transfer turnover: actual 0.1, expected 0
```

Additional behavior REDs captured canonical preview returning one action instead of zero, a genuine post-insertion invalid candidate returning `Ok`, missing prepare/save candidate concurrency APIs, recent quality returning Available instead of Pending, incomplete-cash shadow returning `Some(0)` instead of `None`, a missing Campaign `issues` field, caller-controlled annotation provenance, and historical authoritative cash returning today's 800 instead of origin 1000.

Two final mutation-sensitive REDs were also captured:

```text
multi-account cash returns: 1 unique date/currency key, 2 stored rows
in-period current holding: opening_positions unexpectedly non-empty
```

Each failed before its production guard and passed after it. The CNY attribution fixture's initial expectation of a positive currency effect was corrected as a test-only hand-calculation error: CNY cash converted into CNY stock has a literal zero currency differential, while the action contribution remains independently positive. No production status or threshold was weakened.

### Final verification

```text
cargo test --lib services::stock_review_service::tests --quiet
21 passed; 0 failed

cargo test --lib services::stock_review_persistence::tests --quiet
12 passed; 0 failed

cargo test --lib services::stock_review_quality::tests --quiet
5 passed; 0 failed

cargo test --lib services::stock_review_metrics::tests --quiet
30 passed; 0 failed

cargo test --lib commands::review::tests --quiet
2 passed; 0 failed

cargo test --lib --quiet
456 passed; 0 failed; 8 ignored

npm run build
TypeScript and Vite production build passed; existing large-chunk warning only

rustfmt --edition 2021 --check <six modified Rust files>
exit code: 0

git diff --check
exit code: 0
```

### Remaining honest limitations

- The database has no first-class exchange-calendar table. Live session authority is therefore the designated broad-market benchmark cache and is explicitly labeled as derived; if that cache is insufficient, forward/Campaign path metrics are unavailable rather than inferred from sparse stock quotes.
- Account- or market-filtered daily holding snapshots do not contain authoritative historical cash totals. Their actual TWR and risk cash ratios remain unavailable where exact reconstruction is impossible; the all-account path uses recorded portfolio totals and remains available when coverage is complete.
- Dividend total return is available only when a recorded PAY event or complete adjusted-close series exists. A price-only provider without recorded corporate-action income cannot support a fabricated total-return result.
- Legacy current holdings with no transaction history are treated as the authoritative opening position so the required no-trade report remains usable. The report cannot infer an unrecorded acquisition date; import workflows should preserve OPEN/source-ledger rows when historical dating matters.
- Stored overrides have no persisted `is_active` column; replay activity is computed by fingerprint revalidation. The confirmation revision therefore fingerprints all stored override rows conservatively, so even a stale-row mutation can require a candidate rebuild.
