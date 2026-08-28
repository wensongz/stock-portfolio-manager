# Task 9 Source Authority Round 3 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. This task explicitly forbids subagents.

**Goal:** Enforce one coherent candidate source snapshot and explicit completeness authority for filtered NAV, legacy holdings, market calendars, Campaign FX, and cycle-scoped annotations.

**Architecture:** Keep asynchronous cache fill outside the source snapshot. Pin a query-scoped user revision before async work, pin separate user/cache revisions after fills, materialize all report inputs, and reject if either revision changes before returning or saving. Represent calendar and currency completeness explicitly so deterministic calculators receive either complete inputs or unavailable status, never surviving subsets.

**Tech Stack:** Rust, rusqlite/SQLite, chrono, existing Task 2–9 deterministic services, Tokio tests.

**Spec:** `.superpowers/sdd/2026-08-28-stock-operation-review-redesign/task-9-brief.md`

## Global Constraints

- Work only in `.worktrees/stock-operation-review-redesign`; preserve untracked `node_modules`.
- No subagents, new dependencies, copied formulas, hidden forward fills, or fabricated availability.
- Every production behavior starts with a production-path RED and literal expected values/statuses/issues.
- Report and Campaign valuation stop at `query.end_date`; forward evaluation may extend through authoritative 120-session targets.
- Update `task-9-report.md` with exact RED/GREEN evidence, schema/interface rulings, counts, hashes, and honest limitations.

---

### Task 1: Source snapshot REDs and scoped revision boundary

**Files:**
- Modify: `src-tauri/src/services/stock_review_service.rs`
- Modify: `src-tauri/src/services/stock_review_persistence.rs`

**Interfaces:**
- Produces: query-scoped `ReviewSourceRevision { user, cache }` and candidate pin/verify operations.
- Covers: user mutation during async preparation, quarterly mutation before save, and unrelated account writes.

- [ ] Add a deterministic after-cache-fill race hook test that mutates an in-scope user transaction and expects candidate rejection with zero override rows.
- [ ] Add persistence REDs proving quarterly notes affect the revision and unrelated accounts do not.
- [ ] Replace global table serialization with scoped user/cache row revisions, pin user state before async fill, pin full state after fill, verify around materialization, and recheck inside save transaction.
- [ ] Run focused candidate/persistence tests GREEN.

### Task 2: Explicit NAV, action FX, and Campaign FX completeness

**Files:**
- Modify: `src-tauri/src/models/stock_review.rs`
- Modify: `src-tauri/src/services/stock_review_metrics.rs`
- Modify: `src-tauri/src/services/stock_review_service.rs`

**Interfaces:**
- Produces: explicit filtered NAV completeness; risk FX completeness; Campaign flows that retain local economics with nullable base amount.

- [ ] Add REDs for two filtered dates with one missing FX row, filtered stock-only NAV without cash, missing action-date FX, and non-base Campaign trade/PAY dates missing exact FX.
- [ ] Carry an explicit NAV-complete flag; never average surviving rows or treat stock-only filtered NAV as total NAV.
- [ ] Make turnover and fee drag unavailable when any scoped action notional/fee lacks exact FX and emit a scoped issue.
- [ ] Extend Campaign timeline amounts minimally to preserve local amount/currency when base conversion is missing; invalidate P&L/excursion authority without dropping the flow.
- [ ] Run service/metric REDs GREEN and update serialized frontend types only if the Rust contract requires it.

### Task 3: Legacy holding split reconstruction

**Files:**
- Modify: `src-tauri/src/services/stock_review_service.rs`

**Interfaces:**
- Produces: synthetic legacy opening quantity reversed through every recorded post-origin split, with only report-period splits replayed.

- [ ] Add a DB-backed RED: current legacy holding 20 after a 2:1 split yields opening 10, ending 20, and preserved value.
- [ ] Load splits through current as-of for legacy reversal while passing only `(origin, query.end_date]` events to replay.
- [ ] Emit an unavailable issue when legacy timing/source facts cannot establish a safe synthetic opening.
- [ ] Run legacy and transaction-backed split tests GREEN.

### Task 4: Calendar coverage authority

**Files:**
- Modify: `src-tauri/src/db/mod.rs`
- Modify: `src-tauri/src/db/tests.rs`
- Modify: `src-tauri/src/commands/reset.rs`
- Modify: `src-tauri/src/services/stock_review_market_data.rs`
- Modify: `src-tauri/src/services/stock_review_service.rs`

**Interfaces:**
- Produces: `stock_market_calendar_coverage` metadata plus day rows that explicitly encode open/closed dates.

- [ ] Add schema/reset/backcompat REDs and service REDs for nonempty session rows without authority, missing interior calendar day, incomplete through-date, and stale Campaign terminal coverage.
- [ ] Add coverage metadata and migrate session rows with `is_session`; validate every calendar day inside the claimed range.
- [ ] Permit exact windows only when declared coverage spans the required action/Campaign interval; otherwise return structured unavailable status.
- [ ] Run DB, market-data, and live service tests GREEN.

### Task 5: Annotation-cycle and stale-candidate integrity

**Files:**
- Modify: `src-tauri/src/services/stock_review_service.rs`

**Interfaces:**
- Produces: effective-date/range stock annotation matching and candidate replacement of same-ID stale rows/issues.

- [ ] Add a same-account/two-cycle stock annotation RED: undated stock note attaches to neither detail; explicit effective date attaches to exactly one.
- [ ] Add stale same-ID override preview RED asserting no stale issue and equality with a fresh post-save report.
- [ ] Match exact campaign/action scopes directly; parse optional stock annotation effective dates/ranges; keep ambiguous undated stock annotations report-level only.
- [ ] Remove same-ID stale record and its issue before inserting the in-memory canonical candidate.
- [ ] Run Campaign and override preview tests GREEN.

### Task 6: Verification, report, and commits

**Files:**
- Modify: `.superpowers/sdd/2026-08-28-stock-operation-review-redesign/task-9-report.md`

- [ ] Run scoped rustfmt, focused suites, full `cargo test --lib`, `cargo check --lib`, frontend build if the public model changed, and `git diff --check`.
- [ ] Mutation-check each new authority/completeness branch.
- [ ] Commit implementation as `fix: enforce stock review source authority`.
- [ ] Record the implementation hash and final evidence in the report, then commit the report.
