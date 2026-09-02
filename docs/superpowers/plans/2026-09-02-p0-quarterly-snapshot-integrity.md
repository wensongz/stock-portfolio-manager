# P0 Quarterly Snapshot Integrity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make quarterly snapshot creation and refresh produce the same historically accurate, atomic result without zero-price or fabricated-FX fallbacks.

**Architecture:** A new `quarterly/rebuild.rs` module owns position reconstruction, price/rate resolution, calculation, and transactional replacement. Public create and refresh commands become small adapters around that single path.

**Tech Stack:** Rust 1.97, Tokio, rusqlite 0.40, SQLite, chrono, existing quote and exchange-rate services.

**Spec:** `docs/superpowers/specs/2026-09-02-p0-quarterly-snapshot-integrity-design.md`

## Global Constraints

- No schema migration or frontend protocol change.
- Historical snapshots never use live quotes or current/fabricated FX as historical data.
- Missing required price/rate aborts before persistence.
- Existing quarterly and per-account holding notes survive rebuilds.
- Every behavior change follows a failing-test-first RED/GREEN cycle.

---

### Task 1: Historical position reconstruction

**Files:**
- Create: `src-tauri/src/services/quarterly/rebuild.rs`
- Modify: `src-tauri/src/services/quarterly_service.rs`
- Test: `src-tauri/src/services/quarterly/rebuild.rs`

**Interfaces:**
- Produces: `PositionKey`, `WorkingHolding`, `load_historical_holdings(db, end_date)`.
- Consumes: `quote_provider_service::market_adjusts_sell_pay_cost` and existing transaction schema.

- [ ] **Step 1: Write failing tests** for buy before quarter, add/sell inside quarter, full sale before/after quarter-end, orphaned historical holding metadata, and the same symbol in two accounts.
- [ ] **Step 2: Run** `cargo test --manifest-path src-tauri/Cargo.toml --lib quarterly::rebuild::tests::historical -- --nocapture` and confirm failures describe current/live holding dependence.
- [ ] **Step 3: Implement** grouped transaction loading and deterministic replay through the supplied end date, validating finite values and non-negative ending shares.
- [ ] **Step 4: Re-run** the focused tests and confirm all pass.

### Task 2: Strict historical price and FX resolution

**Files:**
- Modify: `src-tauri/src/services/quarterly/rebuild.rs`
- Test: `src-tauri/src/services/quarterly/rebuild.rs`

**Interfaces:**
- Produces: `resolve_historical_prices`, `resolve_current_prices`, `load_historical_rates`, `validate_rates`.
- Consumes: `(UPPER(symbol), market)` position keys, `daily_holding_snapshots`, `daily_portfolio_values`, quote provider config.

- [ ] **Step 1: Write failing tests** proving price keys include market, cached closes are on/before quarter-end, cash is 1.0, missing price errors, and missing/malformed/non-positive historical FX errors.
- [ ] **Step 2: Run** the focused `quarterly::rebuild::tests` filter and verify each test fails for the intended fallback behavior.
- [ ] **Step 3: Implement** local-first historical close lookup, bounded provider lookup for remaining securities, complete-price validation, historical FX parsing, and positive-finite FX validation.
- [ ] **Step 4: Re-run** the focused tests and confirm all pass.

### Task 3: Canonical atomic rebuild

**Files:**
- Modify: `src-tauri/src/services/quarterly/rebuild.rs`
- Modify: `src-tauri/src/services/quarterly_service.rs`
- Test: `src-tauri/src/services/quarterly/rebuild.rs`

**Interfaces:**
- Produces: `pub(super) async fn rebuild_quarterly_snapshot(...) -> Result<QuarterlySnapshot, String>`.
- Consumes: Tasks 1–2 loaders and the existing quarterly model.

- [ ] **Step 1: Write failing integration tests** for create/refresh equivalence, account-scoped note preservation, missing-price no-write, missing-FX no-write, and replacement rollback after an injected SQLite constraint/type failure.
- [ ] **Step 2: Run** the focused integration tests and verify current create/refresh divergence is observed.
- [ ] **Step 3: Implement** all preflight calculation before one transactional delete/insert/upsert, preserving ID, timestamps and notes.
- [ ] **Step 4: Replace** the bodies of public create and refresh with adapters to the canonical rebuild and remove obsolete duplicate helpers/imports.
- [ ] **Step 5: Run** all quarterly tests plus `cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check`.
- [ ] **Step 6: Commit** with `fix: rebuild historical quarterly snapshots accurately` including this spec and plan.

### Task 4: First-subproject verification

**Files:**
- Verify only.

**Interfaces:**
- Consumes: completed quarterly rebuild.
- Produces: fresh regression evidence before the commit.

- [ ] **Step 1: Run** `cargo test --manifest-path src-tauri/Cargo.toml --lib quarterly -- --nocapture`.
- [ ] **Step 2: Run** `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings`.
- [ ] **Step 3: Inspect** `git diff --check` and `git status --short`.
- [ ] **Step 4: Commit** only after all commands exit zero.
