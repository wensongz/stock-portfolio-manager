# Task 12 report: portfolio-first stock operation review page

## Status and implementation hash

- Complete.
- Implementation commit: `860a430578d627cd2b8e686b4c75dff819788594` (`feat: redesign stock operation review page`).
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
