# Option Import Integrity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent options CSV import from coercing malformed values or committing a partial accepted batch.

**Architecture:** Keep the current two-pass acceptance rules, but parse each accepted row into a validated typed record before database writes. Run all accepted inserts and status recomputation through one caller-owned SQLite transaction and propagate every status write error.

**Tech Stack:** Rust, csv, rusqlite, Tauri 2

**Spec:** `docs/superpowers/specs/2026-09-03-p0-data-integrity-design.md`

## Global Constraints

- Preserve row-level partial acceptance: invalid input rows are reported while other valid rows may import.
- A database or status-recompute failure rolls back every accepted row in that invocation.
- Blank optional numeric fields may retain their existing zero default; malformed non-empty fields must not.
- Do not consolidate the three split-matching implementations in this P0 plan.

---

### Task 1: Reject malformed non-empty numeric values

**Files:**
- Modify: `src-tauri/src/commands/options.rs`
- Test: `src-tauri/src/commands/options.rs`

**Interfaces:**
- Produces: a local decimal parser that distinguishes blank from malformed input and rejects non-finite values.
- Produces: a quantity parser returning `Result<i64, String>` for non-empty input.

- [ ] **Step 1: Write failing malformed-value tests**

Import rows containing `price=oops`, `amount=NaN`, and `quantity=1.5`. Assert `imported == 0`, one field-specific row error per case, and zero database rows.

- [ ] **Step 2: Verify RED**

Run the named tests and confirm the current importer either inserts zero-valued records or silently skips them.

- [ ] **Step 3: Implement strict typed numeric parsing**

Return row errors for malformed non-empty numbers, reject non-finite floats, retain zero only for blank optional decimals, and do not add invalid rows to `parsed`.

- [ ] **Step 4: Verify GREEN**

Run `cargo test --manifest-path src-tauri/Cargo.toml commands::options::tests::test_import_rejects_malformed -- --nocapture` and require zero failures.

### Task 2: Make accepted inserts and status writes atomic

**Files:**
- Modify: `src-tauri/src/commands/options.rs`
- Test: `src-tauri/src/commands/options.rs`

**Interfaces:**
- Produces: `recompute_option_statuses_in(conn: &Connection, account_id: &str) -> Result<(), String>`.
- `import_options_csv_inner` owns an immediate transaction and calls the connection-level recompute before commit.

- [ ] **Step 1: Write a failing second-insert rollback test**

Create a trigger that raises `forced option insert failure` for the second symbol, import two accepted rows, assert the command returns that error, and assert the account has zero option rows.

- [ ] **Step 2: Write a failing status-update rollback test**

Create a trigger that aborts `contract_status` updates, import a valid open row, assert an error is returned, and assert zero option rows remain.

- [ ] **Step 3: Verify both tests RED**

Run both tests; confirm the current implementation leaves inserted rows or reports success after the status failure.

- [ ] **Step 4: Introduce the caller-owned status recompute**

Move status SQL to `recompute_option_statuses_in`, replace every ignored `conn.execute` result with propagated errors, and make the standalone wrapper own an immediate transaction.

- [ ] **Step 5: Wrap import writes and recomputation in one transaction**

Acquire a mutable connection, begin an immediate transaction after parsing and boundary reads, insert all accepted rows, recompute statuses through the same transaction, then commit. Return no successful result after a database failure.

- [ ] **Step 6: Verify focused and full options tests**

Run `cargo test --manifest-path src-tauri/Cargo.toml commands::options -- --nocapture`, format, and run strict Clippy.

- [ ] **Step 7: Commit the option import change**

Run `git diff --check` and commit as `fix: make option csv imports atomic`.

### Task 3: Run the complete repository gate

**Files:**
- Verify all files changed by Tasks 1-2.

- [ ] **Step 1: Run the complete gate**

Run `bun run check` and require 134 frontend tests, all non-ignored Rust tests, build, fmt check, and strict Clippy to pass.

- [ ] **Step 2: Inspect repository state**

Run `git status --short --branch`, `git log --oneline --decorate -5`, and `git diff main...HEAD --check`. Confirm only the approved P0 files and plan/spec documents changed.
