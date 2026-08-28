# Task 9 Candidate Snapshot Round 4 Implementation Plan

> **For agentic workers:** Execute inline under strict test-driven development. Do not delegate this plan.

**Goal:** Align candidate revisions, live materialization, and historical display with the exact dependency scope consumed by a stock-review report.

**Architecture:** Introduce a typed `CandidateRevisionScope` derived from the same discovery plan that drives cache fill. Pin user state across async work, refresh only cache state, then reload user/cache sources and build from the pinned scope. Replace nested JSON row snapshots with compact deterministic streaming digests over sorted, scoped rows.

**Tech Stack:** Rust, rusqlite/SQLite, chrono, existing Task 9 services and tests; no new dependencies.

**Spec:** `.superpowers/sdd/2026-08-28-stock-operation-review-redesign/task-9-brief.md` plus the parent-assigned round-4 rulings.

## Global constraints

- Work only in the Task 9 worktree and preserve the pre-existing untracked `node_modules`.
- Use `apply_patch` for edits and add no dependency.
- Add and capture production-path RED tests before production changes.
- Keep the single deterministic report/Campaign core and prior availability contracts intact.

### Task 1: Exact candidate dependency scope

**Files:** `src-tauri/src/services/stock_review_persistence.rs`, `src-tauri/src/services/stock_review_service.rs`

- [ ] Add failing async tests for mutation of a post-report transaction/split and a future evaluation session/coverage row.
- [ ] Add a failing persistence test proving an unrelated-account override does not invalidate a scoped candidate.
- [ ] Define `CandidateRevisionScope` with report, evaluation, current-ledger/split, display, account, market, security, benchmark, and currency keys.
- [ ] Pin the discovery user digest before async work, set the exact scope after dependency discovery, refresh only cache digest, reload sources, and recheck compact digests before save.
- [ ] Run the focused persistence and service tests GREEN.

### Task 2: Streaming scoped digests

**Files:** `src-tauri/src/services/stock_review_persistence.rs`

- [ ] Write a failing digest-scope regression or compile-time wished-for API assertion.
- [ ] Replace nested row vectors/JSON with deterministic O(1)-memory FNV-1a streaming over typed, sorted scoped query values.
- [ ] Scope active overrides by referenced transactions that can affect the candidate query and include quarterly/display sources through the display cutoff.
- [ ] Run persistence tests GREEN and mutation-check the new scope predicates.

### Task 3: Historical display as-of

**Files:** `src-tauri/src/services/stock_review_service.rs`

- [ ] Add failing tests for a future stock annotation on an active historical Campaign and a future quarterly note.
- [ ] Filter display context at `query.end_date`; pass report as-of into Campaign annotation matching and cap active Campaign lifetime at that date.
- [ ] Run service tests GREEN.

### Task 4: Verification and report

**Files:** `.superpowers/sdd/2026-08-28-stock-operation-review-redesign/task-9-report.md`

- [ ] Run focused suites, full `cargo test --lib`, `cargo check --lib`, frontend build, scoped rustfmt, and `git diff --check`.
- [ ] Document RED/GREEN evidence, exact range model, compact digest approach, remaining limitations, and implementation hash.
- [ ] Commit implementation as `fix: align stock review candidate snapshots`, then commit the report with the implementation hash.
