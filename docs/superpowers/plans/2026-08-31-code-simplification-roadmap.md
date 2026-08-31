# Code Simplification and Optimization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the confirmed import and backup data-integrity risks, reduce redundant performance work and frontend startup cost, then consolidate duplicated workflows without changing product semantics.

**Architecture:** Preserve the React/Zustand/Tauri/SQLite boundaries. Establish one canonical Rust mutation path for holdings and transactions, keep SQLite schema and backup operations explicit and testable, aggregate read models at the Tauri boundary, and extract frontend workflow shells while retaining domain-specific adapters.

**Tech Stack:** React 19, TypeScript 7, Vite 8, Zustand 5, Tauri 2, Rust 1.97.1, rusqlite 0.40, SQLite

**Spec:** `docs/superpowers/specs/2026-08-31-code-simplification-audit.md`

## Global Constraints

- Do not change the existing symbol aggregation invariant.
- Keep quote behavior as startup cache synchronization plus user-triggered refresh.
- Preserve current BUY, SELL, OPEN, PAY, cash-balance, commission, and per-market cost-adjustment semantics.
- Keep broker-specific parsing rules separate from the shared import workflow.
- Do not combine dependency upgrades with these refactors.
- Every task must leave `node --test`, `bun run build`, `cargo fmt --all -- --check`, `cargo test --lib`, and strict Clippy passing.
- Use small commits at the task boundaries below; do not mix phases in one commit.

---

### Task 1: Restore Reproducible Quality Gates

**Files:**
- Modify: `package.json`
- Modify: `.gitignore`
- Track: `bun.lock`
- Create: `.github/workflows/check.yml`
- Modify: `src-tauri/src/services/ai_chat_service.rs`
- Modify: `src-tauri/src/services/option_review_service.rs`
- Modify: `src-tauri/src/services/stock_operation_review_calculator.rs`

**Interfaces:**
- Produces: `bun run test`, `bun run check:frontend`, and `bun run check` repository commands.
- Produces: pull-request CI that uses the tracked Bun lockfile and Rust 1.97.1.
- Preserves: all runtime behavior.

- [ ] **Step 1: Record the current failing strict-lint baseline**

Run:

```bash
cd src-tauri
cargo clippy --all-targets --all-features -- -D warnings
```

Expected: failure containing `manual_inspect`, `manual_is_multiple_of`, test helper `too_many_arguments`, `items_after_test_module`, and four `useless_vec` diagnostics.

- [ ] **Step 2: Fix the production Clippy diagnostics without suppressing them**

Apply the compiler suggestions in `ai_chat_service.rs` and `option_review_service.rs`: use `inspect_err` for the side-effect-only error observer and `is_multiple_of(2)` for the median branch.

- [ ] **Step 3: Simplify the test helpers instead of adding broad allows**

Replace the long option-review insertion argument lists with fixture structs/builders local to the test module, move production items before the calculator test module, replace the 8-argument `action` test helper with an input struct, and use arrays for the four fixed collections.

- [ ] **Step 4: Add stable repository scripts**

Add these script responsibilities to `package.json`:

```json
{
  "test": "node --test",
  "check:frontend": "bun run test && bun run build",
  "check": "bun run check:frontend && cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check && cargo test --manifest-path src-tauri/Cargo.toml --lib && cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings"
}
```

- [ ] **Step 5: Make the Bun lockfile authoritative**

Remove only `bun.lock` from `.gitignore`, run `bun install`, and stage the resulting `bun.lock`. Keep `package-lock.json` ignored because the documented and CI package manager is Bun.

- [ ] **Step 6: Add pull-request CI**

Create `.github/workflows/check.yml` for `pull_request` and pushes to `main`. Install the same Linux Tauri prerequisites used by `build.yml`, set up Bun and Rust 1.97.1, run `bun install --frozen-lockfile`, then run `bun run check`.

- [ ] **Step 7: Verify the quality gate**

Run:

```bash
bun run check
git diff --check
```

Expected: all 75 existing frontend tests, all 400 non-ignored Rust tests, production build, fmt, and strict Clippy pass.

- [ ] **Step 8: Commit**

```bash
git add package.json .gitignore bun.lock .github/workflows/check.yml src-tauri/src/services/ai_chat_service.rs src-tauri/src/services/option_review_service.rs src-tauri/src/services/stock_operation_review_calculator.rs
git commit -m "chore: restore reproducible quality gates"
```

### Task 2: Make CSV Parsing and Confirmation Lossless

**Files:**
- Modify: `src-tauri/src/models/import_export.rs`
- Modify: `src-tauri/src/services/import_export_service.rs`
- Modify: `src-tauri/src/commands/import_export.rs`
- Modify: `src/pages/Import/index.tsx`
- Modify: `src/types/index.ts`
- Test: `src-tauri/src/services/import_export_service.rs`

**Interfaces:**
- Produces: `parse_import_rows(content: &str, data_type: &str) -> Result<ParsedImport, String>` internally.
- Preserves: `parse_import_csv(content: &str, data_type: &str) -> Result<ImportPreview, String>` for the preview command.
- Produces: a serializable preview containing counts and at most 20 display rows, while the confirm command accepts the original CSV content rather than preview rows.
- Produces: `confirm_import(content: String, data_type: String, account_id: String) -> Result<ImportResult, String>` at the Tauri boundary.

- [ ] **Step 1: Add a failing 25-row regression test**

Create a valid 25-row holdings CSV, parse it, and assert:

```rust
assert_eq!(parsed.preview.total_rows, 25);
assert_eq!(parsed.preview.valid_rows, 25);
assert_eq!(parsed.preview.preview_data.len(), 20);
assert_eq!(parsed.valid_rows.len(), 25);
```

Expected before implementation: the internal full-row collection does not exist.

- [ ] **Step 2: Add a failing invalid-row count test**

Use one row missing both `symbol` and `shares`; assert `total_rows == 1`, `valid_rows == 0`, and two field errors. This locks the distinction between invalid rows and error messages.

- [ ] **Step 3: Introduce an internal parsed-import model**

Keep `ImportPreview.preview_data` capped for UI display, but collect every valid canonical row in an internal `ParsedImport.valid_rows`. Count invalid row numbers in a set and compute `valid_rows = total_rows - invalid_row_count`.

- [ ] **Step 4: Stop sending preview rows back for confirmation**

Change `ImportPage` to call `confirm_import` with `rawCsvContent`, `dataType`, and `selectedAccountId`. Remove the frontend construction of `ImportData.rows` and delete the unused `column_mapping` round-trip from the confirmation contract.

- [ ] **Step 5: Reparse on confirmation**

The Rust confirm command must parse the original content again and pass all valid rows to the writer. It must reject unsupported `data_type` values instead of treating every non-holdings value as transactions.

- [ ] **Step 6: Verify lossless confirmation**

Add an in-memory database test that confirms the 25-row CSV and asserts 25 database rows were written. Run:

```bash
cd src-tauri
cargo test services::import_export_service -- --nocapture
```

Expected: all new parsing and 25-row tests pass.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/models/import_export.rs src-tauri/src/services/import_export_service.rs src-tauri/src/commands/import_export.rs src/pages/Import/index.tsx src/types/index.ts
git commit -m "fix: import every valid csv row"
```

### Task 3: Establish One Canonical Portfolio Mutation Path

**Files:**
- Create: `src-tauri/src/services/portfolio_mutation.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Modify: `src-tauri/src/commands/holdings.rs`
- Modify: `src-tauri/src/commands/transactions.rs`
- Modify: `src-tauri/src/services/import_export_service.rs`
- Test: `src-tauri/src/services/portfolio_mutation.rs`
- Test: `src-tauri/src/services/import_export_service.rs`

**Interfaces:**
- Produces: `CreateHoldingInput` and `CreateTransactionInput` typed inputs.
- Produces: `create_holding_in(conn: &rusqlite::Connection, input: &CreateHoldingInput) -> Result<Holding, String>`.
- Produces: `create_transaction_in(conn: &rusqlite::Connection, input: &CreateTransactionInput) -> Result<Transaction, String>`.
- Consumes: a connection already inside the caller-owned transaction or savepoint.

- [ ] **Step 1: Add equivalence tests around existing command behavior**

Cover BUY with commission, SELL with per-market cost adjustment on and off, PAY, cash deposit/withdrawal, a newly created stock holding, and an initial imported holding. Assert holdings, cash balance, transaction rows, and average cost.

- [ ] **Step 2: Add failing import parity tests**

For the same input, compare direct command state with generic-import state. Require identical holdings, cash balance, transaction type, commission handling, and cost basis. Add a test that an imported holding creates the same `OPEN` baseline as `create_holding`.

- [ ] **Step 3: Extract typed inputs and pure validation**

Move transaction type, market, currency, finite-number, share, cash-withdrawal, and average-cost validation into `portfolio_mutation.rs`. Replace the 12–15 positional command parameters inside the service with the two input structs.

- [ ] **Step 4: Extract canonical mutation functions**

Move the current create-holding and create-transaction database mutations into the new service. The service functions must not start or commit transactions; they operate within the caller's transaction.

- [ ] **Step 5: Make single-record commands thin transaction adapters**

Each Tauri command locks the database, starts one transaction, calls the canonical service, commits on success, and rolls back on error. Keep the existing command names and camelCase payloads so frontend callers do not change.

- [ ] **Step 6: Make each imported row atomic**

In `confirm_import`, create one savepoint per valid row, call the same canonical service, release on success, and roll back that savepoint on error. Preserve the existing partial-success result model, but guarantee that a failed row cannot update a holding without its matching transaction.

- [ ] **Step 7: Remove the duplicated direct SQL mutation implementation**

Delete the holding/transaction update formulas and direct INSERT blocks from `import_export_service.rs`. Keep only CSV conversion, row ordering, savepoint orchestration, and result collection.

- [ ] **Step 8: Verify data equivalence and atomicity**

Run:

```bash
cd src-tauri
cargo test portfolio_mutation -- --nocapture
cargo test import_export_service -- --nocapture
cargo test db::tests::tests::test_transaction_atomicity_on_failure -- --nocapture
```

Expected: direct and imported writes have identical state; a forced transaction INSERT failure leaves holdings and cash unchanged for that row.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/services/portfolio_mutation.rs src-tauri/src/services/mod.rs src-tauri/src/commands/holdings.rs src-tauri/src/commands/transactions.rs src-tauri/src/services/import_export_service.rs
git commit -m "refactor: unify portfolio mutation semantics"
```

### Task 4: Make Backup and Factory Reset Data-Safe

**Files:**
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Create: `src-tauri/src/services/backup_service.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Modify: `src-tauri/src/commands/backup.rs`
- Modify: `src-tauri/src/commands/reset.rs`
- Test: `src-tauri/src/services/backup_service.rs`
- Test: `src-tauri/src/commands/reset.rs`

**Interfaces:**
- Produces: `backup_database(source: &Database, destination: &Path) -> Result<(), String>`.
- Produces: `reset_database_state(conn: &mut rusqlite::Connection, now: &str) -> Result<(), String>`.
- Preserves: manual and seven-day automatic backup UI behavior.

- [ ] **Step 1: Add a failing live-backup test**

Create an on-disk source database, insert representative rows, keep the source connection open, run the backup service, open the destination, and assert `PRAGMA integrity_check` returns `ok` and key row counts match.

- [ ] **Step 2: Enable rusqlite online backup support**

Add the `backup` feature alongside `bundled` in `src-tauri/Cargo.toml` and update `Cargo.lock`.

- [ ] **Step 3: Implement one SQLite-aware backup function**

Lock the managed source connection, use rusqlite's online backup API to copy into a newly created destination database, close the backup handle, reopen the destination read-only, and require `PRAGMA integrity_check == 'ok'` before reporting success.

- [ ] **Step 4: Route manual and automatic backup through the service**

Remove both `std::fs::copy` calls. Keep filename, scheduling, change detection, user messages, and logging in the command layer.

- [ ] **Step 5: Write backup config atomically**

Serialize to a temporary file in the application data directory, sync it, and rename it over `backup_config.json`. Update backup metadata only after the verified database backup succeeds.

- [ ] **Step 6: Add factory-reset regression tests**

Start with `tools_enabled = 0`, a populated `cached_quote_refresh_time`, non-default quote settings, and user data. After reset, assert tools are enabled, the refresh timestamp is absent, config defaults match `Default`, and all business tables are empty.

- [ ] **Step 7: Extract database reset logic and remove drift**

Move the transaction-only reset work into `reset_database_state`. Include `cached_quote_refresh_time`, write and update `tools_enabled`, and reuse one exported `SYSTEM_CATEGORIES` definition shared with database initialization.

- [ ] **Step 8: Correct the mixed-state contract**

Run the database reset transaction before replacing `backup_config.json`. If the file update fails after the DB commit, return an explicit “database reset completed but backup preferences could not be reset” error; do not claim cross-resource atomicity.

- [ ] **Step 9: Verify**

Run:

```bash
cd src-tauri
cargo test backup_service -- --nocapture
cargo test commands::reset -- --nocapture
cargo test --lib
```

- [ ] **Step 10: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/services/backup_service.rs src-tauri/src/services/mod.rs src-tauri/src/commands/backup.rs src-tauri/src/commands/reset.rs
git commit -m "fix: make backup and reset data-safe"
```

### Task 5: Introduce Explicit Schema Migrations

**Files:**
- Create: `src-tauri/src/db/schema.rs`
- Create: `src-tauri/src/db/migrations.rs`
- Modify: `src-tauri/src/db/mod.rs`
- Modify: `src-tauri/src/db/tests.rs`
- Modify: `src-tauri/src/services/ai_config_service.rs`
- Modify: `src-tauri/src/services/quote_provider_service.rs`

**Interfaces:**
- Produces: `CURRENT_SCHEMA_VERSION: i64`.
- Produces: `run_migrations(conn: &mut Connection) -> rusqlite::Result<()>`.
- Produces: `column_exists(conn: &Connection, table: &str, column: &str) -> rusqlite::Result<bool>`.

- [ ] **Step 1: Add legacy-database fixtures**

Create in-memory schemas representing a fresh database and a pre-cookie/pre-tools/pre-chat-metadata database. Assert both reach the current columns, constraints, indices, and data repairs after migration.

- [ ] **Step 2: Add a migration-error visibility test**

Create an incompatible legacy table whose column type or constraint prevents migration. Assert initialization returns an error instead of falling back to defaults.

- [ ] **Step 3: Separate current schema from upgrades**

Move fresh `CREATE TABLE` and `CREATE INDEX` statements plus system seed data into `schema.rs`. Move legacy column additions, transaction constraint recreation, and OPEN data repairs into named functions in `migrations.rs`.

- [ ] **Step 4: Replace ignored ALTER errors with explicit introspection**

Call `column_exists` before each legacy column addition. Propagate every SQL error that is not the already-proven “column exists” case.

- [ ] **Step 5: Track schema version transactionally**

Read `PRAGMA user_version`, run ordered migrations in a transaction, and update `user_version` only in the same successful transaction. Reopening an already-current database must execute no migration writes.

- [ ] **Step 6: Stop masking config query failures**

In AI and quote-provider config services, return defaults only for `rusqlite::Error::QueryReturnedNoRows`; propagate schema, type, I/O, and corruption errors.

- [ ] **Step 7: Verify fresh, legacy, repeated, and failing migrations**

Run:

```bash
cd src-tauri
cargo test db::tests -- --nocapture
cargo test services::ai_config_service -- --nocapture
cargo test services::quote_provider_service -- --nocapture
```

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/db/schema.rs src-tauri/src/db/migrations.rs src-tauri/src/db/mod.rs src-tauri/src/db/tests.rs src-tauri/src/services/ai_config_service.rs src-tauri/src/services/quote_provider_service.rs
git commit -m "refactor: version sqlite schema migrations"
```

### Task 6: Aggregate the Performance Read Model

**Files:**
- Modify: `src-tauri/src/models/performance.rs`
- Modify: `src-tauri/src/services/performance_service.rs`
- Modify: `src-tauri/src/commands/performance.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/types/index.ts`
- Modify: `src/stores/performanceStore.ts`
- Test: `src-tauri/src/services/performance_service.rs`
- Test: `src/stores/performanceStore.test.ts`

**Interfaces:**
- Produces: `PerformanceReport { summary, drawdown, attribution, monthly_returns, holding_performances, risk_metrics }` in Rust and TypeScript.
- Produces: `get_performance_report(start_date, end_date, market, account_id, ranking_limit) -> Result<PerformanceReport, String>`.
- Removes: the unused `get_return_series` Tauri command after the aggregate interface is active.

- [ ] **Step 1: Add a report-equivalence Rust test**

Using one populated fixture, compare every field of the new report with the existing six service results. This preserves all investment metric definitions.

- [ ] **Step 2: Refactor calculators to accept one loaded context**

Keep `PerformanceCalculation::load` as the only daily-value/baseline/cash-flow loader. Extract `summary_from`, `drawdown_from`, `risk_from`, `monthly_from`, `attribution_from`, and `ranking_from` functions that accept `&PerformanceCalculation` plus their additional inputs.

- [ ] **Step 3: Add one aggregate service and command**

Load `PerformanceCalculation` once, construct the six report sections, and serialize one `PerformanceReport` through Tauri.

- [ ] **Step 4: Switch the frontend store to one invoke**

After `backfill_snapshots`, call only `get_performance_report`. Replace `Promise.allSettled` fallback-to-null behavior with one explicit report error while preserving the previous successful report until a refresh succeeds.

- [ ] **Step 5: Protect against stale responses**

Add a monotonically increasing request id in `performanceStore`. Only the latest request may write report data, loading, or error state. Add a deferred-promise Node test that resolves an older request after a newer request and asserts the newer report remains selected.

- [ ] **Step 6: Remove obsolete command surface**

Delete the frontend-unused `get_return_series` command and its registration. Keep provider-independent service helpers only if they have direct Rust consumers.

- [ ] **Step 7: Measure before and after**

Add debug timing around `get_performance_report` for one representative local database and record total command time. Verify by code inspection and test instrumentation that `PerformanceCalculation::load` runs once per report request.

- [ ] **Step 8: Verify**

Run:

```bash
node --test src/stores/performanceStore.test.ts
cd src-tauri
cargo test services::performance_service -- --nocapture
```

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/models/performance.rs src-tauri/src/services/performance_service.rs src-tauri/src/commands/performance.rs src-tauri/src/lib.rs src/types/index.ts src/stores/performanceStore.ts src/stores/performanceStore.test.ts
git commit -m "perf: load one performance report per request"
```

### Task 7: Remove Confirmed Runtime and Permission Waste

**Files:**
- Modify: `src-tauri/src/commands/quotes.rs`
- Modify: `src-tauri/src/commands/snapshots.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Modify: `src-tauri/capabilities/default.json`
- Modify: `package.json`
- Modify: `bun.lock`
- Test: `src-tauri/src/commands/quotes.rs`

**Interfaces:**
- Produces: one aggregate realized-PnL query for cleared holdings.
- Removes: `take_snapshot` and `get_portfolio_history` commands.
- Removes: unused Tauri shell plugin registration, Rust/JavaScript dependencies, and `shell:allow-open` permission.

- [ ] **Step 1: Add a multi-cleared-holding PnL test**

Create at least three cleared holdings with BUY/OPEN/SELL histories and assert the aggregate query returns the same realized PnL and buy cost for every holding.

- [ ] **Step 2: Replace the N+1 loop**

Run one SQL query grouped by `holding_id` for all cleared holdings, collect it into `HashMap<String, (f64, f64)>`, and remove per-holding `compute_realized_pnl` calls.

- [ ] **Step 3: Remove unused snapshot commands**

Delete `take_snapshot` and `get_portfolio_history`, their imports, and their `generate_handler!` registrations. Keep `backfill_snapshots` because the performance store uses it.

- [ ] **Step 4: Remove the unused shell plugin**

Delete `@tauri-apps/plugin-shell`, `tauri-plugin-shell`, `.plugin(tauri_plugin_shell::init())`, and `shell:allow-open`. Regenerate both lockfiles.

- [ ] **Step 5: Verify command and permission removal**

Run:

```bash
rg -n "take_snapshot|get_portfolio_history|plugin-shell|tauri_plugin_shell|shell:allow-open" src src-tauri package.json bun.lock
bun run check
```

Expected: no obsolete references and the full quality gate passes.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/commands/quotes.rs src-tauri/src/commands/snapshots.rs src-tauri/src/lib.rs src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/capabilities/default.json package.json bun.lock
git commit -m "refactor: remove unused runtime surface"
```

### Task 8: Add Route-Level Code Splitting

**Files:**
- Modify: `src/App.tsx`
- Test: `src/App.test.tsx` only if a React DOM test harness is introduced; otherwise verify through build output and a Tauri smoke test.

**Interfaces:**
- Produces: lazily loaded route components wrapped by one shared `Suspense` fallback.
- Preserves: all route paths and `MainLayout` behavior.

- [ ] **Step 1: Capture the current bundle baseline**

Run `bun run build` and record the current 4,182.24 kB JavaScript entry and 1,362.16 kB gzip size in the commit message or review notes.

- [ ] **Step 2: Convert page imports to `React.lazy`**

Keep `MainLayout`, the error boundary, and global quote-warning logic eager. Lazily import Dashboard, Statistics, Dividends, Performance, Accounts, Holdings, Transactions, Quarterly routes, Import, Options, Alerts, Review, Settings, and AI Assistant.

- [ ] **Step 3: Add one route fallback**

Wrap `Routes` in `Suspense` with the existing Ant Design `Spin` centered in the content area. Do not add a separate fallback to each route.

- [ ] **Step 4: Verify bundle splitting**

Run `bun run build`. Expected: multiple route chunks are emitted and the entry chunk is materially smaller than the recorded baseline; do not silence the warning by changing `chunkSizeWarningLimit`.

- [ ] **Step 5: Smoke-test navigation**

Run `bun run tauri dev` and open Dashboard, Holdings, Transactions, Performance, Review, Settings, and AI Assistant once. Confirm each lazy chunk loads, the error boundary still renders failures, and direct navigation to quarterly detail works.

- [ ] **Step 6: Commit**

```bash
git add src/App.tsx
git commit -m "perf: split frontend routes"
```

### Task 9: Consolidate Broker Import Workflows

**Files:**
- Create: `src/features/imports/types.ts`
- Create: `src/features/imports/csv.ts`
- Create: `src/features/imports/useImportWizard.ts`
- Create: `src/features/imports/ImportWizard.tsx`
- Create: `src/features/imports/resolveStockNames.ts`
- Create: `src/features/imports/brokers/ibTransactions.ts`
- Create: `src/features/imports/brokers/moomooTransactions.ts`
- Create: `src/features/imports/brokers/firstradeTransactions.ts`
- Create: `src/features/imports/brokers/ibHoldings.ts`
- Create: `src/features/imports/brokers/moomooHoldings.ts`
- Create: `src/features/imports/brokers/firstradeHoldings.ts`
- Modify: the eight CSV `Import*Modal.tsx` files under `src/pages/Holdings` and `src/pages/Transactions`
- Test: matching `*.test.ts` files under `src/features/imports`

**Interfaces:**
- Produces: `ImportAdapter<Row> { parse, normalize, columns, toCommandInput }`.
- Produces: `useImportWizard<Row>(adapter, account, onImported)` for upload, selection, editing, import status, and reset.
- Preserves: broker-specific column aliases, encoding fallbacks, symbol formatting, market detection, notes, and chronological transaction order.

- [ ] **Step 1: Move parsers behind characterization tests**

For IB, Moomoo, and Firstrade holdings and transactions, copy representative CSV fixtures from current accepted formats and assert the complete normalized rows. Keep the existing THS parser tests unchanged.

- [ ] **Step 2: Extract CSV primitives**

Move quote-aware `splitCsvLine`, numeric parsing that reports validation errors, encoding fallback, and shared market/currency helpers into `csv.ts`. Replace duplicated local copies only after their parser tests pass.

- [ ] **Step 3: Extract stock-name resolution**

Implement one resolver that first matches existing holdings, then invokes `lookup_stock_name_by_symbol` for unresolved unique symbols, and returns a symbol-to-name map. Preserve existing fallback to the symbol itself.

- [ ] **Step 4: Extract wizard state**

Move `step`, `fileList`, `rows`, `parseError`, `importing`, `importResult`, row selection/update, close reset, and chronological ordering into `useImportWizard`.

- [ ] **Step 5: Extract the shared wizard shell**

Implement one Upload → Preview → Result component. Accept adapter-provided title, hints, warnings, columns, row editor cells, and command input conversion.

- [ ] **Step 6: Convert transaction modals one at a time**

Convert THS first because it already has parser tests, then Firstrade, Moomoo, and IB. After each conversion, run that adapter's tests and `bun run build`.

- [ ] **Step 7: Convert holding modals one at a time**

Convert Firstrade, Moomoo, IB, then the CN generic holding modal. Preserve its GB18030 fallback and cash-category mapping.

Keep `ImportFromImageModal.tsx` outside the CSV adapter abstraction. It may reuse the shared result type and stock-name resolver in a separate commit only if its OCR recognition and lookup steps remain explicit.

- [ ] **Step 8: Delete duplicated helpers and state**

Run:

```bash
rg -n "function splitCsvLine|function parseNum|const \{ Dragger \} = Upload" src/pages/Holdings src/pages/Transactions
```

Expected: broker modal files no longer define the shared primitives; parser differences remain in adapter files.

- [ ] **Step 9: Verify**

Run `bun run check:frontend`, then manually import one fixture per broker into a disposable database and compare imported/skipped/error counts with the characterization tests.

- [ ] **Step 10: Commit**

```bash
git add src/features/imports src/pages/Holdings/ImportHoldingFromCsvModal.tsx src/pages/Holdings/ImportHoldingFromIbCsvModal.tsx src/pages/Holdings/ImportHoldingFromMoomooCsvModal.tsx src/pages/Holdings/ImportHoldingFromFirstradeCsvModal.tsx src/pages/Transactions/ImportFromIbCsvModal.tsx src/pages/Transactions/ImportFromMoomooCsvModal.tsx src/pages/Transactions/ImportFromFirstradeCsvModal.tsx src/pages/Transactions/ImportFromThsCsvModal.tsx
git commit -m "refactor: consolidate broker import workflows"
```

### Task 10: Split Hotspot Modules Along Existing Responsibilities

**Files:**
- Create: `src-tauri/src/services/quote_service/{mod.rs,cache.rs,persistence.rs,yahoo.rs,eastmoney.rs,xueqiu.rs,history.rs,financials.rs}`
- Delete after migration: `src-tauri/src/services/quote_service.rs`
- Create: `src/pages/AiAssistant/{SessionSidebar.tsx,ChatPanel.tsx,Composer.tsx,MessageRow.tsx,formatters.ts}`
- Modify: `src/pages/AiAssistant/index.tsx`
- Create: `src/stores/chat/{streamReducer.ts,persistence.ts,protocol.ts}`
- Modify: `src/stores/chatStore.ts`
- Out of scope for this roadmap: splitting `quarterly_service.rs` and `ocr.rs`; each requires its own approved follow-up spec after equivalent characterization coverage exists.

**Interfaces:**
- Preserves: current exported quote service functions through `services::quote` re-exports during migration.
- Preserves: `useChatStore` public selectors/actions and `AiAssistantPage` route export.
- Produces: modules with one provider, protocol, or UI responsibility each.

- [ ] **Step 1: Freeze public APIs with compile-time and behavior tests**

List every quote function imported outside `quote_service.rs` and every chat-store action used outside `chatStore.ts`. Add focused tests for parsing, fallback order, streaming updates, persistence, and session switching before moving code.

- [ ] **Step 2: Split quote code in dependency order**

Move cache and persistence first, provider implementations second, then history and financials. Keep fallback orchestration in `quote_service/mod.rs`. Run quote tests after each move.

- [ ] **Step 3: Replace global quote state with managed state only after the file split is stable**

Introduce a Tauri-managed `QuoteServiceState` containing credentials, automatic token state, warning state, and database access. Remove `APP_DB_PATH` and the fresh fallback connection only after all command, AI-tool, OCR, and startup call sites accept the managed state explicitly.

- [ ] **Step 4: Split AI assistant presentation components**

Move SessionSidebar, ChatPanel, Composer, MessageRow, ErrorCard, message metadata, and time formatters without changing props or state ownership. Keep route orchestration in `index.tsx`.

- [ ] **Step 5: Split chat-store pure logic**

Move stream event normalization/reduction, persistence conversion, and protocol types into focused files. Keep the Zustand store as the orchestration boundary.

- [ ] **Step 6: Review quarterly and OCR separately**

Create a new focused spec before changing either module. Quarterly currently has only three local service tests despite 2,420 production lines; OCR already has broad parser tests but also owns external process execution. Do not combine either split with quote or chat changes.

- [ ] **Step 7: Verify each commit**

Run the full `bun run check` after every module family, inspect `git diff --stat`, and reject moves that add adapters without reducing responsibility or duplication.

- [ ] **Step 8: Commit quote and AI splits separately**

```bash
git commit -m "refactor: split quote providers and persistence"
git commit -m "refactor: split ai assistant presentation and stream logic"
```

### Task 11: Final Verification and Scope Audit

**Files:**
- Verify: all files changed by Tasks 1–10
- Update: `README.md`
- Update: `README_EN.md`
- Update: `docs/RELEASE-NOTES.md` only for user-visible fixes and performance changes

**Interfaces:**
- Consumes: the complete roadmap implementation.
- Produces: fresh evidence for correctness, reproducibility, bundle shape, migration compatibility, backup restorability, and import parity.

- [ ] **Step 1: Run the full automated gate**

```bash
bun install --frozen-lockfile
bun run check
```

Expected: frontend tests/build, Rust fmt/tests, and strict Clippy all pass.

- [ ] **Step 2: Run data-safety scenarios**

Verify a 25+ row generic import, one invalid mixed import, broker imports, live backup plus restore into a temporary database, factory reset with non-default tools state, fresh database startup, and legacy database migration.

- [ ] **Step 3: Record performance evidence**

Run `bun run build` and record entry plus route chunk sizes. Record one representative performance-report timing and one 100-row transaction import timing before and after the relevant tasks.

- [ ] **Step 4: Audit command and dependency surface**

```bash
rg -n "take_snapshot|get_portfolio_history|get_return_series|plugin-shell|tauri_plugin_shell|shell:allow-open" src src-tauri package.json bun.lock
git diff --check
git status --short
```

Expected: no obsolete command/plugin references, no whitespace errors, and only approved files changed.

- [ ] **Step 5: Update development documentation**

Document the tracked Bun lockfile, `bun run check`, pull-request CI, lossless import behavior, and verified online backup behavior. Keep technical refactor details out of user-facing release notes unless they affect reliability or speed.

- [ ] **Step 6: Final review**

Confirm that no task changed investment formulas, market/currency semantics, quote refresh frequency, or symbol aggregation. Review each commit independently and revert any abstraction that only moves code without simplifying ownership.
