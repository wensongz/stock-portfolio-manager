# Task 9 Corrected Replay Round 2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. This task explicitly forbids subagents.

**Goal:** Make every stock-review consumer use one corrected, source-versioned replay with authoritative sessions and exact-date currency conversion.

**Architecture:** Extend the action builder's ordered override replay into a typed corrected ledger that owns transaction disposition, cash effects, action mapping, split-adjusted position events, and same-day order. Live preparation fills caches first, then materializes one immutable cached input from corrected ledger rows, explicit cached exchange sessions, exact-date prices/FX, and a full source revision; the report and Campaign detail remain projections of the existing deterministic core.

**Tech Stack:** Rust, rusqlite/SQLite, chrono, existing Task 4-8 stock-review services, Tokio tests.

**Spec:** `.superpowers/sdd/2026-08-28-stock-operation-review-redesign/task-9-brief.md`

## Global Constraints

- Work only in `.worktrees/stock-operation-review-redesign`; preserve untracked `node_modules`.
- No subagents, new dependencies, copied metric formulas, hidden forward fills, or fabricated availability.
- Every production behavior begins with a production-path failing test and captured RED.
- Report valuation/Campaign terminal value stops at `query.end_date`; action evaluation may extend through 120 authoritative sessions capped at today.
- Commit only Task 9 changes and update `task-9-report.md` with exact evidence.

---

### Task 1: Production-path RED matrix

**Files:**
- Modify: `src-tauri/src/services/stock_review_service.rs`
- Modify: `src-tauri/src/services/stock_review_persistence.rs`
- Modify: `src-tauri/src/db/tests.rs`

**Interfaces:**
- Consumes: current `prepare_cached_stock_review_input`, override confirmation, and real in-memory SQLite database.
- Produces: failing tests for corrected cash/flows, horizon, calendar holes, dividend completeness, pre-origin splits, FX/NAV, preview revision/scope, suppression, Campaign references/annotations, and quality independence.

- [ ] Add literal DB/cache fixtures for A-K, naming the production mutation each test catches.
- [ ] Run each focused test and record a non-zero RED caused by current production behavior.

### Task 2: Canonical corrected replay

**Files:**
- Modify: `src-tauri/src/services/stock_action_builder.rs`
- Modify: `src-tauri/src/services/stock_campaign_builder.rs`
- Modify: `src-tauri/src/services/stock_review_service.rs`

**Interfaces:**
- Produces: ordered corrected ledger entries with `included`, `transfer`, cash-effect disposition, grouped real action ID, and split-adjusted position events.
- Consumers: opening/current cash, shadow flows/dividends, attribution cash/batches, risk, Campaign cash flows/action references, and holdings reconstruction.

- [ ] Add corrected-ledger unit RED for non-trade, duplicate, transfer, grouped fills, same-day order, and pre-origin split.
- [ ] Implement one ordered replay and replace every raw-transaction consumer.
- [ ] Run corrected-ledger and live consistency tests GREEN.

### Task 3: Authoritative sessions and extended evaluation cache

**Files:**
- Modify: `src-tauri/src/db/mod.rs`
- Modify: `src-tauri/src/db/tests.rs`
- Modify: `src-tauri/src/commands/reset.rs`
- Modify: `src-tauri/src/services/stock_review_market_data.rs`
- Modify: `src-tauri/src/services/stock_review_service.rs`

**Interfaces:**
- Produces: cached `stock_market_sessions(market,date,source,updated_at)` boundary and exact 60/120-session targets.
- Consumers: forward actions and Campaign expected sessions; benchmark quotes supply prices only.

- [ ] Add schema/cache/session loader RED and missing-interior-benchmark regression.
- [ ] Load/fill stock and local benchmark prices through the authoritative 120th session capped at today.
- [ ] Suppress exact-session metrics with structured issues when session authority/coverage is absent.
- [ ] Verify report and Campaign as-of remain capped at query end.

### Task 4: Return-mode, split, and exact FX authority

**Files:**
- Modify: `src-tauri/src/services/stock_review_service.rs`
- Modify: `src-tauri/src/services/stock_review_market_data.rs`

**Interfaces:**
- Produces: adjusted-close total return only when complete; explicit dividends only from complete corporate-action coverage; exact-date FX for every required currency/source row; base-valued NAV.

- [ ] Add sold-before-dividend, pre-origin split, CN/base-USD, mixed-market, and opening-only currency REDs.
- [ ] Separate actual PAY cash income from shadow dividend-source completeness.
- [ ] Convert snapshot rows before aggregation and propagate precise missing-FX issues/status.
- [ ] Run focused GREEN tests.

### Task 5: Full candidate source revision and scope

**Files:**
- Modify: `src-tauri/src/services/stock_review_persistence.rs`
- Modify: `src-tauri/src/services/stock_review_service.rs`

**Interfaces:**
- Produces: canonical candidate query-scope validation and a transactionally rechecked source revision covering all report source tables.

- [ ] Add out-of-scope/no-effect, concurrent split/price/session/FX/source mutation, genuine post-insertion failure, and saved-state equality REDs.
- [ ] Capture the post-fill materialization revision, build the candidate, and reject before persistence when any source changes.
- [ ] Run persistence/service GREEN tests.

### Task 6: Dependent-status and Campaign integrity cleanup

**Files:**
- Modify: `src-tauri/src/services/stock_review_service.rs`
- Modify: `src-tauri/src/services/stock_review_quality.rs`

**Interfaces:**
- Produces: internally consistent opening-cash suppression, forward-only quality dependency, real Campaign action references, and account/lifetime-scoped annotations.

- [ ] Add contradiction, unrelated-gap, grouped-fill, and cross-account annotation REDs.
- [ ] Clear all shadow/automatic-mixed dependent comparable outputs together and isolate forward quality.
- [ ] Map flows through corrected ledger action IDs and enforce annotation applicability.
- [ ] Run focused GREEN tests.

### Task 7: Verification, report, and commit

**Files:**
- Modify: `.superpowers/sdd/2026-08-28-stock-operation-review-redesign/task-9-report.md`

- [ ] Run scoped rustfmt, focused suites, full `cargo test --lib`, command tests, frontend build, and `git diff --check`.
- [ ] Mutation-check every realistic branch named by A-K.
- [ ] Record RED/GREEN evidence, interface decisions, counts, and honest limitations.
- [ ] Commit with `fix: unify corrected stock review replay` and a report commit if the implementation hash must be recorded.
