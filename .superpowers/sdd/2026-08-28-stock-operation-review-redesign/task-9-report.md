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

## Fix round 2: coherent corrected replay and source authority

This section supersedes the round-1 statements that designated benchmark quote rows could provide session authority, that PAY rows could certify shadow dividends, and that no schema change was needed.

- Implementation commit: `01dec52d906209c12ad83c7a02380fe989f3f168` (`fix: unify corrected stock review replay`).
- No dependency was added. The pre-existing untracked `node_modules` directory remained untouched and unstaged.

### Corrected replay and deterministic live preparation

- `ActionBuildResult.corrected_transactions` is the one ordered, override-corrected ledger. Opening/current cash, opening holdings, actual attribution, risk/turnover actions, Campaign flows, action mapping, and security discovery consume it instead of independently looping raw transactions.
- Confirmed `non_trade` rows are excluded everywhere; a valid duplicate group keeps one canonical row; a confirmed transfer retains the position movement but has no investment cash effect, action, turnover, fee drag, or Campaign cash flow. Same-day override order is retained before every consumer sees the ledger.
- Every corrected source fill receives its real grouped action ID. Campaign flow `action_id` references therefore always resolve to an action in that Campaign, including grouped same-day fills.
- The price/cache horizon is independent from the report valuation cutoff. It extends through the 120th authoritative session after historical actions when today and cached data permit; report and Campaign terminal values remain fixed at `query.end_date`.
- A new `stock_market_sessions` cache is the sole session authority. Benchmark and stock quote rows are prices only. If explicit session authority is absent, exact forward/Campaign metrics are unavailable with `market_calendar_unavailable`; a missing stock or local-benchmark close on an authoritative target session cannot shift to another quote date.
- PAY remains actual account cash income but no longer proves complete per-share dividends for shadow holdings. Shadow total return uses `ExplicitDividends` only when the corporate-action field covers every relevant holding/session, otherwise complete adjusted close, otherwise honest price-only degradation.
- Pre-origin splits scale each source position lot through all later splits up to the explicit actual origin exactly once. Only splits after origin enter the replay engine, preventing both omitted adjustment and double application.
- Filtered/mixed-currency NAV converts every snapshot row from its local market currency with exact-date FX before aggregation. Missing FX suppresses average NAV and dependent turnover/fee-drag precision with `snapshot_fx_unavailable`; raw local values are never summed as base currency. Required FX currencies include opening positions/cash and discovered securities, not only transaction rows.
- Incomplete opening cash clears automatic mixed benchmark return, excess/active return, shadow comparison curves, and all value-add comparable fields while preserving the actual recorded TWR and unrelated status regions.
- Campaign annotations now require an exact campaign/action scope or matching stock scope plus account applicability. Same-symbol notes no longer leak across accounts or logical cycles.
- Forward quality is derived only from its exact action-window dependencies and maturity. Unrelated global security/FX gaps can still degrade overall quality without changing a valid forward region.

### Override preview consistency and concurrency

- Query validation rejects corrections whose references do not affect the requested account, market, or report cutoff; pre-period references remain legal when they actually affect reconstruction.
- The canonical candidate override is inserted before Task 5/6/7 materialization, so preview risk, attribution, Campaigns, cash, and actions reflect the same correction that will be persisted.
- Candidate persistence now fingerprints the complete conservative source set: transactions, holdings, portfolio/holding snapshots, stock and benchmark prices, explicit sessions, splits, annotations, and cached FX, in addition to the active override revision and referenced transactions.
- Candidate-owned async cache preparation refreshes only the source revision after cache fill. The original active-override revision remains pinned. Persistence rechecks both under one SQLite write transaction; any concurrent source mutation rejects the stale candidate before insertion.
- A genuine post-in-memory-insertion replay failure returns an error with zero override rows. A successful returned candidate matches a fresh report rebuilt from the saved override state.

### Production-path acceptance coverage and RED evidence

The initial round-2 live suite produced 13 failures (19 passes), including these literal contradictions:

```text
corrected opening cash: actual 750, expected 900
pre-origin split quantity: actual 10, expected 20
historical 60-session window: Pending, expected Available
missing authoritative target close: Available, expected Unavailable
mixed-currency filtered NAV: actual 800, expected 200
incomplete opening cash excess_return: Some(0), expected None
grouped Campaign flow referenced a nonexistent reconstructed action ID
candidate replay failure returned Ok and source revision mutation still saved
```

The final DB/cache-backed tests assert values, statuses, issues, actions, campaigns, and database row counts for:

1. transfer/non-trade/duplicate consistency across cash, attribution, turnover, and Campaign flows;
2. historical actions maturing beyond the report cutoff without leaking future terminal value;
3. authoritative interior-session and benchmark endpoint holes;
4. PAY with a sold actual position while shadow still holds the security;
5. pre-origin and post-origin splits applied exactly once;
6. USD-base CN and multi-market snapshot conversion, plus missing exact-FX suppression;
7. out-of-scope correction rejection, successful preview/save identity, genuine candidate-build failure/no-write, active override race, and full-source revision race;
8. incomplete opening-cash dependent-field suppression while actual TWR stays visible;
9. grouped fills and two-account annotation isolation;
10. unrelated coverage gaps leaving forward quality independent;
11. two cycles of one symbol, two accounts, pre-period actions, and historical as-of pricing;
12. the original deterministic twelve-scenario matrix plus same-day uncertainty, duplicate conflict, fixed benchmark, suspension/delisting, and fee zero.

### Final verification

```text
cargo test --lib services::stock_review_service::tests --quiet
33 passed; 0 failed

cargo test --lib services::stock_review_persistence::tests --quiet
13 passed; 0 failed

cargo test --lib services::stock_review_quality::tests --quiet
5 passed; 0 failed

cargo test --lib services::stock_action_builder::tests --quiet
9 passed; 0 failed

cargo test --lib
469 passed; 0 failed; 8 ignored

cargo check --lib
passed

npm run build
TypeScript and Vite production build passed; existing large-chunk warning only

rustfmt --edition 2021 --check <seven modified Rust files>
exit code: 0

git diff --check
exit code: 0
```

### Remaining honest limitations after round 2

- The repository still has no production exchange-calendar provider or importer. The explicit `stock_market_sessions` boundary supports deterministic/cache-fed authority; live databases without that cache suppress exact-session forward and Campaign metrics instead of treating sparse quote dates as sessions.
- The current price provider does not certify complete adjusted-close or corporate-action dividend coverage. PAY can support actual account income only; shadow total-return metrics remain price-only/unavailable unless a complete source is cached.
- Account/market-filtered snapshots still lack authoritative historical cash. Their actual TWR remains unavailable even when exact local-to-base stock NAV is displayable; no cash balance is fabricated.
- Full-source candidate revision is deliberately conservative and global. An unrelated write to a reviewed source table can force a rebuild, preferring rejection over returning a stale candidate.

## Fix round 3: scoped source snapshots and explicit authority

This section supersedes round 2's global candidate fingerprint, its session-row-only calendar boundary, and its treatment of filtered stock NAV as an authoritative portfolio denominator.

- Implementation commit: `13e27ae` (`fix: enforce stock review source authority`).
- No dependency was added. The pre-existing untracked `node_modules` directory remained untouched and unstaged.

### Coherent candidate snapshot and scoped revisions

- Candidate preparation now separates user-owned and cache-owned revisions. The query scope pins account, market, and report cutoff before asynchronous cache work. Cache fills may advance only the cache revision; if transactions, holdings, snapshots, splits, annotations, or quarterly context change during the async phase, pinning rejects the candidate instead of blessing the write.
- After cache preparation, all candidate inputs are materialized against the pinned scoped revision and rechecked after reads. `save_override_candidate` recomputes the same scoped revision inside its SQLite write transaction before inserting. A concurrent source mutation therefore returns an error and has zero override side effects.
- User revisions include scoped transactions, holdings, daily holding/portfolio snapshots, splits, annotations, and joined quarterly snapshot/holding context. Cache revisions include scoped stock/benchmark prices, exchange-session rows, calendar coverage metadata, and cached FX. An unrelated account transaction no longer invalidates a single-account candidate, while a report-visible quarterly mutation does.
- Same-ID preview insertion replaces both the active and stale forms in memory and removes that record's stale issue. The returned preview matches a fresh report after persistence.
- A production-path test injects an in-scope holding mutation precisely after async cache fill and before candidate pinning. Preparation rejects it with `A user-owned report source changed during cache preparation`, and the override table remains empty.

### NAV, FX, Campaign, and split authority

- Filtered NAV now has an explicit completeness flag. Missing FX on any valuation row clears the whole filtered series rather than averaging surviving rows. Even fully converted stock-only filtered snapshots cannot supply an authoritative cash denominator, so average NAV, turnover, and fee drag remain unavailable with `filtered_nav_cash_unavailable`.
- Missing exact action-date FX clears action notional, fees, turnover, and fee-drag inputs with `action_fx_unavailable`; no partial or zero result is fabricated.
- Campaign flows preserve their local amount and currency even when exact FX is absent. Base amounts and all dependent Campaign P&L/excursion fields become unavailable with `campaign_fx_unavailable`, so an economic BUY/SELL/PAY/fee row never disappears merely because conversion is missing.
- Legacy current holdings with no position ledger are reversed through every recorded post-origin split, then post-origin split events replay exactly once. The live regression proves a current quantity of 20 after a 2:1 split becomes opening quantity 10 and ending quantity 20 with value preserved.

### Authoritative calendar coverage

- Added `stock_market_calendar_coverage` with market, source, complete range, revision, and an explicit `encodes_closed_dates` flag. `stock_market_sessions` now records `is_session`, allowing every calendar day in a certified range to be represented as open or closed.
- Exact-session calculations require coverage spanning the requested origin/target or Campaign lifetime. A nonempty session table without coverage metadata is unavailable. A declared range with any missing interior calendar-day row is invalid. Campaign terminal metrics are unavailable when certified coverage stops before the report cutoff; the last cached session is never substituted.
- Existing installations migrate `is_session` with a safe default, but old rows receive no implicit authority because coverage metadata is absent. Reset clears both session rows and coverage metadata. No live importer was invented; databases without an authoritative calendar remain honestly unavailable.

### Annotation lifetime and interface cost

- Campaign-scoped and action-scoped annotations remain exact. A stock-scoped annotation attaches to a Campaign only when `effective_date`, `effective_start`/`effective_end`, or the historical `snapshot_date` overlaps that Campaign, or when there is only one unambiguous account/symbol cycle. `created_at` is never treated as an economic date. Ambiguous undated same-symbol notes remain report/stock-level.
- `CampaignTimelineItem.amount_base` is now optional and retains `amount_local` plus `currency`. The four base-currency Campaign P&L components are optional as well. This minimal model extension is required to preserve an unconverted economic flow while preventing partial base-currency metrics; snake-case serialization remains unchanged.

### TDD RED/GREEN evidence

The first round-3 production-path suite failed before implementation:

```text
persistence: 3 intended failures / 16 tests
- in-scope holding mutation was blessed after async preparation
- quarterly context mutation was invisible
- unrelated account transaction invalidated a global source revision

service: 5 intended failures / 38 tests
- filtered NAV averaged the surviving exact-FX subset
- stock-only filtered snapshots supplied an authoritative NAV denominator
- missing action-date FX produced zero-valued ratio inputs
- Campaign missing FX dropped economic flows
- a legacy current holding replayed a split twice

database: 2 intended failures
- calendar coverage metadata table absent
- reset did not clear coverage authority
```

Additional focused REDs captured a nonempty session table being accepted without coverage metadata, a missing interior day under declared coverage, Campaign coverage ending before its report cutoff, same-ID stale preview leakage, and the annotation-lifetime API mismatch:

```text
error[E0061]: annotation_applies_to_campaign expected 2 arguments but the cycle set was supplied
```

The async race regression first failed to compile because the cache-fill boundary did not exist, then passed only after the hook exercised the production preparation path and the user-source pin rejected the mutation. All focused suites are now GREEN without weakening status expectations.

### Final verification

```text
cargo test --lib services::stock_review_persistence::tests
16 passed; 0 failed

cargo test --lib services::stock_review_service::tests
44 passed; 0 failed

cargo test --lib db::tests
45 passed; 0 failed

cargo test --lib
483 passed; 0 failed; 8 ignored

cargo check --lib
passed

npm run build
TypeScript and Vite production build passed; existing large-chunk warning only

rustfmt --edition 2021 --check <eight modified Rust files>
exit code: 0

git diff --check
exit code: 0
```

### Remaining honest limitations after round 3

- The repository still has no production exchange-calendar importer. The new schema is an explicit authority boundary for a future provider or deterministic fixture; live databases without certified complete ranges suppress exact-session metrics.
- Scoped revisions are conservative where a source table has no account or symbol ownership, notably all-account portfolio totals, benchmark prices, and cached FX. A relevant-scope cache or global total mutation can still force a safe rebuild.
- Filtered daily holding snapshots contain stock rows but no authoritative historical cash balance. They can display fully converted stock values, but portfolio-denominator ratios remain unavailable until account/market cash snapshots are persisted.
- Legacy holdings with no transaction history remain an explicit fallback for no-trade reports. Recorded splits are handled exactly once, but an unrecorded trade or unknown holding as-of timestamp cannot be reconstructed; imports should preserve authoritative OPEN/transaction history.
- The current quote provider still does not certify complete adjusted-close or corporate-action dividend coverage. Account PAY rows remain actual cash income only and cannot establish shadow total-return completeness.

## Fix round 4: aligned candidate snapshots and historical display cutoffs

This section supersedes round 3's looser candidate-range fingerprint and row-vector serialization approach.

- Implementation commit: `441ca81405c3d83c2ed91a1157e0616936bd09b2` (`fix: align stock review candidate snapshots`).
- No dependency was added. The pre-existing untracked `node_modules` directory remained untouched and unstaged.

### Exact consumed-range model and coherent reload

- `CandidateRevisionScope` is the explicit dependency contract for confirmation. It carries report start/end, price start, exact evaluation end, current-ledger/split horizon, display cutoff, selected accounts and markets, exact `(symbol, market)` securities, portfolio/local benchmark symbols, and required currencies.
- Candidate discovery first pins the broad in-scope user ledger and active-override set through today. Async price/benchmark cache preparation may then complete. User-owned transactions, holdings, snapshots, splits, annotations, and quarterly history are rechecked immediately after that boundary and are never refreshed or blessed.
- Cache-owned calendar rows and coverage are reloaded after cache preparation, and `evaluation_end` is recomputed from the 120th authoritative local-market session for every report action. The exact scope is pinned only after this dependency plan is current.
- Every mutable report source is then reloaded: active/stale overrides with candidate replacement, transactions and corrected actions/Campaigns, holding/security discovery, calendar authority, stock prices, selected benchmark prices, and local benchmark prices. Security dependencies must still equal discovery, and the compact exact-scope revision is checked after materialization.
- The save transaction recomputes the identical active-override, user-source, and cache-source digests before persistence. A post-end transaction or split that can affect current-cash unwind/opening reconstruction is included through `current_horizon`; prices, benchmark rows, sessions, and coverage are included through `evaluation_end`; quarterly/display inputs use `display_cutoff`.
- A cache fill that legitimately changes future session/coverage data is visible to the recomputed plan rather than leaving stale pre-fill rows in the candidate. A concurrent user mutation during the same async boundary rejects the candidate and produces no override write.

### Scoped streaming revisions

- The former nested JSON row vectors were replaced with deterministic sorted SQLite scans feeding separate 16-character user/cache/override digests. Rows are hashed as they are read, so memory use is O(1) with respect to source history and the write transaction compares only compact digest values.
- Transactions remain broad within the selected account/market through the current horizon because even a post-report trade changes authoritative current-cash unwind. Holdings, snapshots, prices, benchmarks, sessions, coverage, annotations, and quarterly context are narrowed to the exact ranges and keys each loader consumes.
- Security SQL uses exact `(symbol, market)` tuple identity rather than independent symbol/market filters. A cross-product cache row such as `AAPL/CN` therefore cannot invalidate a candidate consuming only `AAPL/US` and `600000/CN`.
- Active-override revision scans only override rows whose referenced transactions intersect the query's selected account/market/current-horizon source scope. An unrelated-account override update no longer invalidates confirmation, while an in-scope override mutation still does.

### Historical display as-of

- Quarterly holding notes and manual `decision_quality` rows are loaded only when the joined quarterly snapshot date is on or before `query.end_date`.
- Annotation visibility uses explicit `effective_date`, `snapshot_date`, or `effective_start`; `created_at` is not treated as an economic date. Campaign matching receives the report as-of date, caps an active Campaign's effective end at that date, and rejects annotation ranges that begin later.
- These annotations remain display-only and never enter metric, attribution, replay, or quality inputs.

### TDD RED/GREEN evidence

The round-four production-path/persistence RED suite failed before implementation with these intended contradictions:

```text
candidate_revisions_are_compact_streaming_digests:
active override revision length was 2 instead of 16

unrelated_account_override_does_not_invalidate_scoped_candidate:
candidate save was rejected by a global override revision

candidate_rejects_post_report_transaction_and_split_mutation:
expected revision error but preparation returned Ok(CachedStockReviewInput)

candidate_reloads_future_evaluation_calendar_after_cache_fill:
candidate retained the stale future session plan

historical_display_excludes_future_quarterly_notes:
2 notes loaded instead of 1

active_historical_campaign_rejects_annotation_starting_after_report_as_of:
error[E0061]: annotation matcher had no report-as-of argument

unrelated_symbol_market_pair_does_not_invalidate_scoped_candidate:
candidate was rejected by an unrelated cross-product price row
```

All fail-first tests passed after the coherent snapshot, exact range, historical cutoff, and tuple-scope changes. Existing Task 9 corrected-ledger, 120-session maturity, calendar authority, dividend mode, split, FX completeness, opening suppression, Campaign integrity, forward-quality independence, stale-replacement, AI provenance, and twelve-scenario acceptance tests remained unchanged and GREEN.

### Final verification

```text
cargo test --lib services::stock_review_persistence::tests --quiet
19 passed; 0 failed

cargo test --lib services::stock_review_service::tests --quiet
48 passed; 0 failed

cargo test --lib commands::review::tests --quiet
2 passed; 0 failed

cargo test --lib
490 passed; 0 failed; 8 ignored

cargo check --lib
passed

npm run build
TypeScript and Vite production build passed; existing large-chunk warning only

rustfmt --edition 2021 <two modified Rust files>
exit code: 0

git diff --check
exit code: 0
```

### Remaining honest limitations after round 4

- The repository still has no production exchange-calendar importer. Cache preparation can consume authoritative rows supplied through the explicit calendar/coverage boundary, but a normal live database without certified coverage continues to suppress exact-session forward and Campaign metrics.
- Cached FX is a global singleton rather than a per-date/per-scope source table. Its revision is necessarily conservative, and historical non-base metrics remain unavailable where exact daily FX cannot be resolved from recorded portfolio snapshots.
- Filtered daily holding snapshots still do not persist authoritative historical cash by account/market. They can display complete converted stock NAV, but denominator-based turnover and fee drag remain unavailable rather than using a partial portfolio value.
- Legacy holdings with no transaction history still require the documented fallback. Recorded splits are reversed/replayed exactly once, but an unrecorded trade or unknown holding as-of timestamp cannot be reconstructed.
- The compact digest is intentionally a lightweight deterministic concurrency guard, not a cryptographic hash. It avoids full-history serialization and global scans, while direct SQL mutations in the exact consumed scope remain detectable by the regression suite.

## Fix round 5: complete revision coverage and scoped stale state

This final Task 9 correction aligns every remaining revision query with the production consumer that reads the source.

- Implementation commit: `86d690c1d509fdf7adbf92bf849be32da8dfd4b2` (`fix: complete stock review revision coverage`).
- No dependency or schema change was added. The pre-existing untracked `node_modules` directory remained untouched and unstaged.

### Snapshot, split, and override scope

- Daily holding snapshot revision now covers every row read by filtered NAV, attribution, and risk: selected account(s), optional market, and `report_start..=report_end`. It intentionally does not filter by discovered current/transaction securities because historical snapshot-only positions are real consumer inputs.
- Split parameters use the shared trimmed ASCII-uppercase stock identity and SQL applies `UPPER(TRIM(stock_code))`. A mutation to a stored ` aapl ` split therefore invalidates an `AAPL` candidate exactly as replay would match it.
- A shared scoped-override SQL predicate defines account, market, and current-ledger horizon semantics for both active-override revision and report loading. Both pre-cache and post-cache report preparation now load only query-relevant active and stale rows and generate issues only for that set. The global `list_overrides` audit API remains global by design.
- A relevant stale correction remains visible as `stale_override`; an unrelated-account stale correction neither appears in the report nor invalidates its candidate.

### Annotation date semantics

- `effective_date`, `effective_start`, `effective_end`, and `snapshot_date` are parsed by one shared typed boundary. Present values must be strings, must be real calendar dates in exact `YYYY-MM-DD` form, and `effective_start` must not exceed `effective_end`.
- Annotation save validates dates before opening a write transaction, so every rejected date fixture has zero database side effects. Report/Campaign visibility uses the same parser; a malformed legacy row is unavailable as display context instead of silently becoming an undated note.
- Candidate revision fingerprints every annotation row the account-scoped loader can return. This deliberately includes future-effective rows before the pure display cutoff is applied, so a concurrent future annotation mutation cannot pass unnoticed. `created_at` remains audit metadata, never an economic date.

### Structurally framed streaming digest

- The 16-character digest remains a compact, O(1)-memory, noncryptographic FNV-based concurrency guard.
- Every structural component now has an explicit frame tag and payload length. SQLite rows encode row ordinal, declared column count, column index, row terminator, and a distinct type tag for NULL, integer, real, text, and blob before the value bytes.
- Direct regressions prove that NULL differs from text `null`, text differs from a blob with identical bytes, integer differs from real, and two-column boundary rearrangements differ. This removes the prior NULL/text and text/blob alias while retaining deterministic streaming.

### TDD RED/GREEN evidence

The initial persistence run executed 27 tests with 6 intended failures and 21 passes:

```text
historical_snapshot_symbol_outside_discovery_still_invalidates_candidate:
candidate save unexpectedly succeeded after the snapshot-only MSFT row changed

normalized_split_symbol_mutation_invalidates_candidate:
candidate save unexpectedly succeeded after the stored ` aapl ` split changed

annotation_rejects_invalid_economic_dates_without_writing:
invalid 2024-02-30 effective_date was accepted

future_annotation_mutation_is_part_of_candidate_revision:
candidate save unexpectedly succeeded after the future note changed

digest_distinguishes_null_from_text_null:
both digests were 1e409891730aaf9e

digest_distinguishes_text_from_blob_with_the_same_bytes:
both digests were d183d9261a9ad525
```

The integer/real and multi-column boundary digest tests already passed before the structural change and remain as mutation guards. Scoped override loading first failed to compile with `E0432` because the query-scoped API did not exist; after the API was introduced, the production report RED exposed 2 stale issues instead of the expected single relevant issue. The final report regression returns exactly the relevant stale record and excludes the unrelated account.

### Final verification

```text
cargo test --lib services::stock_review_persistence::tests --quiet
28 passed; 0 failed

cargo test --lib services::stock_review_service::tests --quiet
50 passed; 0 failed

cargo test --lib commands::review::tests --quiet
2 passed; 0 failed

cargo test --lib
501 passed; 0 failed; 8 ignored

cargo check --lib
passed

npm run build
TypeScript and Vite production build passed; existing large-chunk warning only

rustfmt --edition 2021 <two modified Rust files>
exit code: 0

git diff --check
exit code: 0
```

### Remaining honest limitations after round 5

- The repository still has no production exchange-calendar importer. Exact-session forward and Campaign metrics remain unavailable without certified calendar coverage.
- Cached FX remains a global singleton and historical non-base metrics still require exact daily FX from recorded portfolio snapshots; its candidate revision is necessarily conservative.
- Filtered daily holding snapshots still lack authoritative historical cash by account/market, so denominator-based turnover and fee drag remain unavailable instead of using partial NAV.
- Legacy holdings without a complete transaction ledger still use the documented fallback; recorded splits are handled exactly once, but unrecorded economic events cannot be reconstructed.
- The streaming digest is collision-resistant only in the practical concurrency-guard sense, not cryptographic. Its structural framing prevents deterministic type/boundary aliases but does not provide adversarial integrity guarantees.
