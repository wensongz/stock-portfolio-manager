# Task 12 report: portfolio-first stock operation review page

## Status and implementation hash

- Complete.
- Implementation commit: `860a430578d627cd2b8e686b4c75dff819788594` (`feat: redesign stock operation review page`).
- Review-remediation implementation commit: `ab76456c88dde0e177db7de7f7db5d71c9a0662d` (`fix: address stock review page findings`).
- Second review-remediation implementation commit: `891e8dd7a59cff477f52e7243a42bff9d8cf5f3c` (`fix: close stock review race and persistence gaps`).
- No npm or Cargo dependency was added.
- The pre-existing untracked `node_modules` symlink/directory was preserved and never staged.

## Component map and wiring

- `StockReviewTab` is now the portfolio-first container. It restores `review_stock_filters_v1`, follows the global base currency, loads the persisted report on mount without a stock-selection gate, persists every valid filter change, exposes an explicit refresh, preserves a prior successful report after a later command error, and uses a full-page retry state only when no report exists.
- `StockReviewFilters` covers all accounts/single account, QTD/previous quarter/YTD/1Y/custom, all/single market, automatic/specified benchmark, and the global base currency. Its controls have explicit accessible labels and wrap at narrow widths.
- `StockReviewDataQuality` keeps normal coverage concise and expands independently sorted blocker/warning/info issues plus actual/shadow/benchmark return methods, coverage, FX, benchmark, algorithm version, and interval-drawdown methodology. The Rust contract does not expose a standalone provider name, so the UI explicitly says the provider is backend-managed instead of inventing one.
- `StockReviewSummaryCards` renders exactly five titled cards in the approved order: result quality, max drawdown, rebalance value-add, forward effect, and risk structure. Every scalar is a backend value; null/non-finite values render `—`, while a real zero remains zero. Each card keeps its backend availability/note visible; 60-day and 120-day status/sample maturity are separate.
- `PortfolioComparisonChart` directly plots the backend's aligned 100-base actual/shadow/benchmark points, retains null gaps with `connectNulls=false`, omits unavailable series, explains absent shadow/benchmark data, shows all three return methods in the tooltip, and maps action markers through the backend action-to-Campaign identities before opening the drawer.
- `RebalanceAttributionPanel` groups the four factual action types without recomputing financial aggregates, then displays backend contributor/detractor rows, dividend/fee/FX/cash components, ending difference, and residual. It labels percentage attribution as explanatory approximation rather than exact TWR decomposition.
- `RiskStructurePanel` shows stock-denominator max weight/CR5, cash separately, turnover/fee statuses and values, opening/ending/peak comparisons, and puts HHI/weights/data hints behind an expansion.
- `StockActionsTable` shows the full backend action fields, filters by symbol/action/Campaign/status, defaults to date descending, and supports date/amount/contribution/60-day sorting through the tested pure helper. Null sort values remain last. Click, Enter, or Space opens the backend-associated Campaign.
- `StockCampaignDrawer` shows lifecycle, account fragments/transfers, action and cash-flow timelines, costs, dividends, fees, P&L, excess return, remaining market value, maximum invested capital, MAE/MFE, holding drawdown, 20/60/120-day effects, contribution, sample status, quarterly manual history, annotations, and issues. Active P&L explicitly says it includes remaining market value and is not realized return. Annotation saves call the stock review Store. The four correction types open a confirmation modal listing selected transaction IDs and expected impact; `confirmOverride` is called only by the modal confirmation action.
- The current Rust/TypeScript Campaign detail contract has no standalone maximum-holding-amount/weight fields. Those two values therefore render `—` with an explicit contract note; the frontend does not infer them from action rows.
- `LegacyStockReviewPanel` retains the existing `reviewStore` quarterly selection, timeline, notes, manual decision-quality editing, and statistics under a default-collapsed “历史手工决策记录” entry. No new-report component reads `decision_quality` as a core metric.
- Portfolio and Campaign AI buttons use the exact Task 11 builders and report methodology query, navigate with `autoSend: false`, and carry the executable tool arguments. `AiAssistant` consumes the one-shot `stock-review` activation, stages it for the next turn, and clears the navigation state. Both `get_stock_review` and `save_stock_review_annotation` have Chinese labels in `ToolCallCard` and the legacy badge fallback.

## TDD evidence

The first display/prefill run was executed after adding tests and before adding the production exports:

```text
node --test src/pages/Review/stockReviewViewModel.test.ts src/pages/AiAssistant/prefill.test.ts

SyntaxError: stockReviewViewModel.ts does not provide
STOCK_REVIEW_SUMMARY_CARD_ORDER
SyntaxError: prefill.ts does not provide readAiPrefillActiveSkill
2 test files failed; exit code 1
```

The minimal helper implementation made the unchanged focused suite green:

```text
28 tests passed; 0 failed; exit code 0
```

Those behavior tests cover all four statuses and Chinese colors/labels, null versus real zero, four action labels, gap preservation and unavailable-series omission, severity ordering, fixed five-card order, empty/partial/full-error page states, four backend-only action sorts, portfolio/Campaign executable prefills, and the non-auto-send stock-review activation guard.

## Accessibility and visual verification

- Filters, correction controls, annotation input, sort controls, and the drawer have explicit labels.
- Action rows are keyboard-focusable buttons with Enter/Space activation; the legacy symbol list uses native buttons.
- Tables use horizontal scrolling rather than clipping fields, cards use responsive Ant Design grid breakpoints, filter controls use wrapping flex layout, and the drawer uses Ant Design's responsive large size.
- Browser smoke at the default desktop viewport and a temporary `900 × 800` viewport confirmed the page directly shows filters and the full-page retry state, wraps controls without horizontal page overflow, and keeps the legacy section collapsed. The temporary viewport override was reset.
- A standalone browser does not have the Tauri bridge, so this smoke intentionally exercised the command-failure path. Its remaining Tauri event/invoke errors came from running outside the desktop shell. Component API deprecation warnings introduced during implementation were removed before completion.

## Final verification

```text
node --test src/**/*.test.ts
84 passed; 0 failed

npm run build
TypeScript and Vite production build passed; existing large-chunk warning only

cargo check --lib
passed

cargo test --lib commands::review::tests --quiet
2 passed; 0 failed; 543 filtered out

git diff --check
exit code 0
```

## Handoff concerns

- Visual testing with live deterministic report data still requires the Tauri desktop shell because the browser cannot invoke Rust commands. The production build, Store tests, command boundary tests, and browser failure-path smoke are green.
- The backend contract limitation for Campaign maximum holding amount/weight and standalone market provider is surfaced honestly in the UI. Adding those fields belongs in the deterministic Rust report rather than a frontend calculation.
- No full Rust regression was rerun because Task 12 changed no Rust source; the exact consumed command boundary was checked and `cargo check --lib` passed.

## Review remediation

The requested six findings are closed without changing the approved information architecture:

1. The portfolio/Campaign route prefill now validates and stages the exact `get_stock_review` argument object independently of the visible approved prompt. `chatStore` consumes that object atomically on the next outbound turn, clears it, and sends it through `ChatRequest`; the Rust AI service permits only the read-only `get_stock_review` tool, executes that exact scope once before model reasoning, and exposes the normal tool lifecycle UI. A retry/regenerate in memory retains its captured scope; after persistence and reload, regeneration reconstructs the scope only from exactly one completed reserved `prefilled-stock-review` read-tool record, revalidates every field, and restores the `stock-review` skill. A later unrelated turn does not inherit it.
2. Campaign detail loads and override confirmations now use `StockReviewFilters` reconstructed from the retained report's `methodology.query`. A newer filter edit whose refresh failed can no longer mix its account/date/market/benchmark/currency with the visible older report.
3. Correction candidates now come from every action in the retained report, not the open Campaign. Each row shows transaction, stock, account, Campaign, and date, so cross-account/cross-Campaign transfer pairs can be selected. Each correction type states cardinality and economic eligibility: transfer requires exactly two rows, duplicate/same-day order require at least two, and non-trade requires exactly one, matching `stock_review_persistence`. `same_day_order` has explicit keyboard-labelled up/down controls, and that ordered state is serialized unchanged into both transaction IDs and the backend order value.
4. Campaign detail now includes Campaign return, benchmark return, remaining shares, all five backend availability regions and notes, and the note attached to each 20/60/120-day forward window. The existing explicit maximum-holding contract limitation remains visible and no value is inferred.
5. `comparable_price_only` changes the third summary title to `调仓价格增益`; missing recovery renders `—`; and the nested 120-day status/sample/note remains visible.
6. Any non-null report is content, even when curves/actions/Campaigns are all empty. Summary, attribution, and risk remain rendered; curve, Campaign, and action collections show their own local empty states.

### Remediation TDD evidence

The focused behavior tests were extended first. Before production exports/wiring existed, the run failed nonzero with missing `consumeAiPrefillToolContext` and `buildStockReviewReportFilters` exports:

```text
node --test --experimental-strip-types \
  src/pages/AiAssistant/prefill.test.ts \
  src/pages/Review/stockReviewViewModel.test.ts

2 test files failed; exit code 1
```

The unchanged focused behavior suite then passed `36/36`. It covers exact portfolio/Campaign scope, one-shot consumption, invalid structured context, retained-report identity, cross-Campaign/account candidates, explicit reordering, per-type cardinality, price-only naming, null recovery, 120-day notes, all five Campaign availability regions, and collection-empty report semantics. A focused Rust test also verifies that only the exact read-only stock-review scope is accepted.

### Remediation verification

```text
node --test --experimental-strip-types src/**/*.test.ts
92 passed; 0 failed

npm run build
TypeScript and Vite production build passed; existing large-chunk warning only

cargo check --lib
passed

cargo test --lib services::ai_chat_service::prefilled_tool_tests --quiet
1 passed; 0 failed

cargo test --lib --quiet
538 passed; 0 failed; 8 ignored

git diff --check
exit code 0
```

The new modal controls use explicit accessible labels, the selected transaction list is ordered and numbered, and all additions reuse Ant Design's responsive `Space`, `Descriptions`, `List`, `Card`, and `Drawer` primitives. The prior desktop/narrow browser smoke remains applicable to the unchanged page architecture; live Campaign data and its Tauri-only drawer still require the desktop shell.

Remaining explicit backend contract limitations are unchanged: the report does not expose standalone Campaign peak-holding amount/weight or a standalone market-provider name, so the UI continues to show `—`/backend-managed explanatory text. Correction candidates intentionally include every eligible transaction ID exposed by report actions; dividend/fee timeline records and already-confirmed transfers are not offered as new trade-classification corrections.

## Second review remediation

The three follow-up findings are closed with scoped Store/view-model changes:

1. Campaign requests now capture the exact source report identity (`generated_at` plus methodology query), reject filters that do not match that report, and verify the same identity again on response. A successful new report commit increments the Campaign request generation and clears detail/loading a second time. Therefore an old detail resolving either before or after the new report cannot remain attached. Campaign annotation and override callbacks carry the same identity and Campaign ID; the Store rejects them without invoking Tauri while a refresh is active or after either identity changes.
2. The `non_trade` guidance and client validity rule now require exactly one transaction, matching Rust `validate_non_trade`; zero and multiple selections keep confirmation disabled.
3. Completed host-prefilled tool calls are already persisted in assistant `tool_calls`. Session loading now reconstructs explicit context only when there is exactly one completed reserved `prefilled-stock-review` / `get_stock_review` record whose JSON arguments pass the same allowlist and required-field validation. It also restores the explicit `stock-review` skill. Live staging remains one-shot, and arbitrary tools, running/malformed/duplicate reserved records, and extra arguments are not elevated.

### Second remediation TDD evidence

Tests were written before the production fixes. The first focused run exited nonzero with four failures:

```text
- non_trade guidance still said “at least one”
- both Campaign response-order tests left the old detail attached
- the chat Store reload had no explicitToolContext
```

After fixing the Node test harness's extension resolution, the chat Store assertion failed with `actual: undefined` for the restored context. A further trust-boundary RED proved that a reserved record missing a completed status was incorrectly accepted. The final focused suite passed `58/58`, including the full Store path `send → tool event → persistence snapshot → load → regenerate`.

### Second remediation verification

```text
node --test --experimental-strip-types src/**/*.test.ts
96 passed; 0 failed

npm run build
TypeScript and Vite production build passed; existing large-chunk warning only

cargo test --lib services::stock_review_persistence::tests::validate_override_checks_all_types_without_writing_and_save_is_idempotent --quiet
1 passed; 0 failed

cargo test --lib --quiet
538 passed; 0 failed; 8 ignored

cargo check --lib
passed

git diff --check
exit code 0
```

No new dependency or database column was added. Regeneration restoration depends on the reserved completed tool-call record already written by successful chat persistence; older sessions created before tool-call persistence cannot reconstruct context and safely regenerate without the explicit scope.
