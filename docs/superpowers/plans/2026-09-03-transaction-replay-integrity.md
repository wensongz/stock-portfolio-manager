# Transaction Replay Integrity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make transaction edit/delete and cost-policy changes derive holdings from chronological history inside one SQLite transaction.

**Architecture:** Add a focused `position_replay` service containing the pure position projection and targeted/bulk database rebuilds. Keep Tauri commands as transaction-owning adapters and make quote-provider persistence call the same bulk rebuild when a cost flag changes.

**Tech Stack:** Rust, rusqlite, Tauri 2, React, TypeScript

**Spec:** `docs/superpowers/specs/2026-09-03-p0-data-integrity-design.md`

## Global Constraints

- Preserve existing BUY, SELL, OPEN, PAY, commission, cash, and per-market cost-policy formulas.
- Reject, rather than persist, transaction history that produces an unexplained negative position.
- Make every edit, delete, and policy-triggered rebuild atomic.
- Do not change option matching behavior in this plan.

---

### Task 1: Add the chronological position projection

**Files:**
- Create: `src-tauri/src/services/position_replay.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/position_replay.rs`

**Interfaces:**
- Produces: `PositionKey::new(account_id: &str, symbol: &str) -> PositionKey`.
- Produces: `rebuild_position_group(conn: &Connection, key: &PositionKey) -> Result<Option<String>, String>`.
- Produces: `rebuild_all_position_groups(conn: &Connection) -> Result<(), String>`.

- [ ] **Step 1: Write failing replay tests**

Add hand-derived fixtures covering BUY/BUY/SELL average cost, PAY with the policy on/off, and BUY 10 followed by SELL 11. Assert exact final shares/cost and an error containing `historical position` for the negative case.

- [ ] **Step 2: Run the focused tests and verify RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml services::position_replay -- --nocapture` and confirm the missing replay interface causes failure.

- [ ] **Step 3: Implement the pure replay and targeted rebuild**

Implement chronological ordering, negative-position validation, primary-holding reuse/creation, relinking, and duplicate deletion. Every SQL error must be propagated.

- [ ] **Step 4: Implement and test the bulk rebuild**

Load holdings and non-cash transactions once, group them by case-insensitive account/symbol keys, run the same pure projection, then apply all group writes through the caller's connection.

- [ ] **Step 5: Run the focused tests and verify GREEN**

Run `cargo test --manifest-path src-tauri/Cargo.toml services::position_replay -- --nocapture` and require zero failures.

### Task 2: Route edit and delete through replay

**Files:**
- Modify: `src-tauri/src/services/portfolio_mutation.rs`
- Modify: `src-tauri/src/commands/transactions.rs`
- Test: `src-tauri/src/commands/transactions.rs`

**Interfaces:**
- Produces: `update_transaction_in(conn: &Connection, id: &str, input: &CreateTransactionInput) -> Result<Transaction, String>`.
- Produces: `delete_transaction_in(conn: &Connection, id: &str) -> Result<(), String>`.
- Consumes: `rebuild_position_group` from Task 1.

- [ ] **Step 1: Write failing mutation regression tests**

Create real in-memory accounts and transactions. Assert deleting the first BUY from BUY 10/SELL 10 fails without changing rows, editing BUY 10 to BUY 5 before SELL 10 fails without changing rows, and moving a SELL to a key with no position fails without creating an orphan row.

- [ ] **Step 2: Verify the tests fail for the current inverse-arithmetic implementation**

Run the three named tests with `cargo test --manifest-path src-tauri/Cargo.toml commands::transactions::tests:: -- --nocapture` and confirm the observed state violates the assertions.

- [ ] **Step 3: Implement caller-owned mutation helpers**

Use `validate_transaction_values`, update/delete the transaction row, reverse/apply additive cash deltas, rebuild each distinct non-cash old/new key, and query the committed projection for the returned transaction.

- [ ] **Step 4: Reduce Tauri commands to transaction adapters**

Replace manual `BEGIN/COMMIT/ROLLBACK` and inverse cost formulas with `transaction_with_behavior(Immediate)`, the new helper, and `commit()`.

- [ ] **Step 5: Verify focused and portfolio tests**

Run `cargo test --manifest-path src-tauri/Cargo.toml commands::transactions services::portfolio_mutation services::position_replay -- --nocapture` and require zero failures.

### Task 3: Make cost-policy persistence and rebuild atomic

**Files:**
- Modify: `src-tauri/src/services/quote_provider_service.rs`
- Modify: `src-tauri/src/commands/quote_provider.rs`
- Modify: `src-tauri/src/commands/transactions.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/pages/Settings/GeneralSettings.tsx`
- Test: `src-tauri/src/services/quote_provider_service.rs`

**Interfaces:**
- `update_quote_provider_config(db, config)` remains the public service interface.
- It invokes `rebuild_all_position_groups` only when one of the three cost flags changes.

- [ ] **Step 1: Write a failing atomic rollback test**

Install a SQLite trigger that aborts holding updates, change one cost flag, call `update_quote_provider_config`, and assert both the configuration and holding values retain their original values.

- [ ] **Step 2: Verify RED**

Run `cargo test --manifest-path src-tauri/Cargo.toml services::quote_provider_service::tests::cost_policy -- --nocapture`; the current two-command flow cannot satisfy the atomic contract.

- [ ] **Step 3: Persist configuration through a connection-level helper**

Validate first, begin an immediate transaction, load the prior flags, persist the new row, run the bulk rebuild only when flags changed, and commit. Propagate every error.

- [ ] **Step 4: Remove the redundant public recalculation command**

Delete `recalculate_holdings_cost`, unregister it, and remove the frontend's second invoke. Keep the existing loading indicator around the single provider-update request.

- [ ] **Step 5: Verify and commit the transaction replay change**

Run `cargo fmt --manifest-path src-tauri/Cargo.toml --all`, focused Rust tests, `bun run test`, `bun run build`, and `git diff --check`. Commit as `fix: rebuild holdings from transaction history`.
