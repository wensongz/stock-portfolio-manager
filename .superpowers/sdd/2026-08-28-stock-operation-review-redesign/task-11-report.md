# Task 11 report: stock review frontend data layer

## Status and implementation hash

- Complete.
- Implementation commit: `71661c298a879246525b44f60701a56b2cb96c31` (`feat: add stock review frontend data layer`).
- No npm or Cargo dependency was added.
- The pre-existing untracked `node_modules` symlink/directory was preserved and never staged.

## TypeScript contract mirror

- `src/types/index.ts` now mirrors the current `src-tauri/src/models/stock_review.rs` snake-case JSON contract rather than the older design sketch.
- The mirror covers `MetricAvailability`, query/methodology/coverage, the five summary regions, all return curves, attribution items and totals, risk snapshots and weights, actions and observation windows, logical Campaign summaries/fragments/transfer facts, complete Campaign detail/P&L/timeline, independent quality regions, issues, annotations, and annotation/override mutation inputs and outputs.
- Rust `Option<T>` fields are required TypeScript properties with `T | null`; none is weakened to an optional property. Required vectors and scalar fields remain required.
- Serialized Rust enums use exact snake-case unions: four metric states, action types, Campaign lifecycle states, cash-flow kinds, and issue severities.
- `StockReviewFilters` is intentionally a frontend camel-case view model. Tauri calls serialize it to the commands' exact camel-case arguments, while AI prefill serializes it to the `get_stock_review` tool's strict snake-case schema and omits absent optional arguments rather than sending rejected `null` values.

## ViewModel behavior

- Date presets derive the civil date in `Asia/Shanghai`, then perform date-only arithmetic, avoiding the UTC shift produced by formatting the supplied `+08:00` instant directly as UTC.
- The fixed 2026-08-28 boundary produces YTD `2026-01-01..2026-08-28`, QTD `2026-07-01..2026-08-28`, previous quarter `2026-04-01..2026-06-30`, and a 365-calendar-day 1Y window `2025-08-29..2026-08-28`.
- `review_stock_filters_v1` stores account, preset/range, market, benchmark, and base currency. Missing JSON, malformed JSON, unknown fields/values, empty identifiers, invalid dates, and reversed custom ranges fall back to all accounts + YTD + all markets + automatic benchmark. The live application base currency passed by the exchange-rate store remains authoritative over a persisted old currency.
- Custom ranges use exact real `YYYY-MM-DD` validation and reject before storage writes when incoherent.
- Display mapping preserves the backend status/note and maps only null or non-finite display scalars to `—`; a real zero remains `0`. It does not compute or replace any financial result.
- Portfolio and Campaign AI prefills use the two approved Chinese prompts, activate only `stock-review`, expose complete executable `get_stock_review` arguments, and set `autoSend: false`. Campaign symbol comparison form is trim + uppercase; the stable Campaign ID is trimmed without changing case.

## Store concurrency and mutation rulings

- Report and Campaign requests have independent monotonic request IDs. A stale report success/error cannot modify the latest filters, report, error, or loading state; a stale Campaign response cannot replace the newest drawer detail and Campaign loading never clears the portfolio report.
- Errors retain the last successful report/detail. Error provenance is tracked internally so a successful report, Campaign, or mutation clears only its own stale error rather than hiding an unrelated failure.
- Mutations use a monotonic mutation ID plus a pending-operation count. The newest mutation owns data/error changes, while `mutating` remains true until every overlapping mutation settles.
- Override confirmation participates in the report-generation sequence: it invalidates report loads that started earlier, while a report load started after confirmation began remains the latest filter authority. This closes both directions of the report/override race.
- Annotation results are bound to the report/Campaign generations from which they were submitted. An authoritative old-scope response cannot be attached to a newer filter report.
- `saveAnnotation` applies the command-returned annotation row, including authoritative source/timestamps/value, by stable ID. It updates Campaign annotations only for matching period, Campaign, action, or normalized stock/account scope and leaves all report metrics untouched.
- `confirmOverride` adopts the command-returned `StockReviewReport` directly, invalidates stale Campaign detail, and never issues a second report load.
- Report, detail, annotation, and override command names and camel-case argument objects match the four Task 9 Tauri command signatures exactly.

## TDD evidence

Initial ViewModel RED, before either production file or contract additions existed:

```text
node --test src/pages/Review/stockReviewViewModel.test.ts
Error [ERR_MODULE_NOT_FOUND]: Cannot find module .../stockReviewViewModel.ts
1 failed; exit code 1
```

Initial Store RED, before the Zustand store existed:

```text
node --test src/stores/stockReviewStore.test.ts
Error [ERR_MODULE_NOT_FOUND]: Cannot find module .../stockReviewStore.ts
1 failed; exit code 1
```

Display-safety RED, before the mapping export existed:

```text
SyntaxError: ... does not provide an export named 'mapStockReviewMetricForDisplay'
1 failed; exit code 1
```

The first cross-channel mutation race run executed ten store tests with three intended failures and seven passes:

```text
old report load replaced generated_at/algorithm_version `confirmed` with `obsolete-load`
old-filter override replaced the newer `new-filter` report
old period annotation was appended to the newly loaded report
exit code 1
```

The three tests passed only after report-generation pinning was added. They are direct mutation guards: removing the confirmation generation bump, the generation equality check, or the annotation context check changes the asserted report identity/annotation array.

One build-only RED was also captured after the first GREEN unit run:

```text
npm run build
TS6133: Zustand initializer argument `get` was declared but never read
exit code 1
```

The root cause was the new store following the two-argument initializer shape while using only `set`; the minimal fix removed the unused argument, after which the unchanged production build passed.

## Final verification

```text
node --test src/pages/Review/stockReviewViewModel.test.ts src/stores/stockReviewStore.test.ts
17 passed; 0 failed

node --test src/**/*.test.ts
57 passed; 0 failed

npm run build
TypeScript and Vite production build passed; existing large-chunk warning only

cargo test --lib commands::review::tests --quiet
2 passed; 0 failed; 543 filtered out

git diff --check
exit code 0
```

The repository has no `npm test` script, so `node --test src/**/*.test.ts` is the stable full frontend test command. No full Rust run was needed because Task 11 changes no Rust code; the focused command suite verifies the consumed Tauri boundary.

## Settings reset and handoff concerns

- General Settings removes `review_stock_filters_v1` alongside the existing explicit application-owned key list. It still calls `removeItem` only for that allowlist, so unrelated browser storage is preserved.
- Task 12 should read `report.methodology.query` (the actual Rust contract), not the older plan prose's `methodology.filters` wording.
- Task 12 can stage `prefill.activeSkill`, navigate with `prefill.prompt`, and must continue honoring `autoSend: false`; the executable tool arguments remain available for deterministic context/debug display without frontend financial calculation.
- The backend's documented calendar, historical FX/cash, total-return provider, and legacy-ledger limitations remain represented through statuses/issues. The frontend data layer does not hide or repair them.
- Code-review subagent dispatch was intentionally not performed because the Task 11 assignment explicitly prohibited subagents; the parent SDD controller remains the review boundary.
