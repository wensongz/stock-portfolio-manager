# Issue 154: negative cash in current portfolio views

**Goal:** Include cash overdrafts in statistics and dashboard holdings and net-value position weights.

**Architecture:** Keep the existing backend debit-cash read model. Correct the frontend active-holding filters and preserve cash's unit cost in aggregate rows and account breakdowns.

**Stack:** React/TypeScript, Node tests, Bun render probes, Rust/SQLite.

**Spec:** https://github.com/wensongz/stock-portfolio-manager/issues/154

## Constraints

- Work in the current checkout; preserve the existing mixed-currency commit. The user requested a local commit after verification; no push, migration or issue update is needed.
- Keep zero-balance filtering and inactive-stock filtering unchanged.
- Cash overdrafts reduce net assets and position-weight denominators; retain native row currencies.

## Task 1: Preserve cash overdrafts throughout current portfolio views

**Files:** `src/pages/Statistics/categoryHoldings.ts`, `src/pages/Dashboard/aggregateDashboardHoldings.ts`, `src/pages/Statistics/{OverviewTab,MarketTab,CategoryTab}.tsx`, and associated regression tests.

- [x] Add failing filter and aggregation tests for positive CNY cash, negative HKD cash and ordinary stocks; verify signed market values and net position weights.
- [x] Add real statistics render coverage for negative cash, unit average cost and position percentages across overview, market and cash-category views.
- [x] Fix the filters to preserve negative cash while excluding inactive stocks. Keep cash average cost at 1 in all affected aggregates, including offsetting account balances.
- [x] Run targeted tests, the frontend suite/build, Rust suite, formatting and clippy; review the diff.

## Review focus

- Same-currency positive and negative cash across accounts must offset, including a zero aggregate.
- Negative ordinary stocks remain excluded where active holdings are filtered.
- Account breakdowns use the same net denominator as aggregate rows.
- Negative-only cash must remain visible without invalid weights or profit ratios.
- Existing backend accounting and native currencies remain intact.

## Execution record

- Current HEAD `3b53417` already reads negative cash and includes a database-backed foreign-stock-purchase regression. The remaining issue is reproducible in the frontend filters.
- RED: four filter/aggregation regressions failed because negative cash was discarded. Six rendered-view subtests failed on missing cash, zero average cost or incorrect position weights.
- GREEN: all 22 targeted tests passed after the frontend changes, including offsetting account balances and debit-only portfolios.
- Backend verification: 849 tests passed, 13 existing tests ignored; rustfmt and clippy (all targets/features, warnings denied) passed.
- Final frontend verification: 548/548 tests passed; TypeScript and production build passed. Vite retains its chunk-size advisory.
- Independent review found no actionable Critical/Important issues or material test gaps; no deferred findings. Diff whitespace checks passed. The user subsequently requested committing these verified changes locally.
