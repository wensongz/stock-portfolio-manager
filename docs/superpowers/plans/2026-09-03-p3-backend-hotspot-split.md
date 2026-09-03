# P3 Backend Hotspot Split Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Split the option command and performance service hotspots along their existing responsibilities without changing any public API or financial result.

**Architecture:** Keep `commands/options.rs` and `services/performance_service.rs` as compatibility facades. Move cohesive implementations into private child modules, expose only the symbols needed by the facade, and keep all repository callers on their current module paths.

**Tech Stack:** Rust 1.97.1, Tauri 2, rusqlite 0.40, csv, chrono

**Spec:** `docs/superpowers/specs/2026-09-03-p3-targeted-simplification-design.md`

## Global Constraints

- Do not modify Tauri command names, parameters, return types, database structure, or serialization shapes.
- Preserve option FIFO, split matching, status projection, simulation formulas, and CSV compatibility.
- Preserve TWR, drawdown, attribution, risk, ranking, and benchmark-return formulas.
- Do not split `commands/transactions.rs`; its production code is already focused.
- This is a behavior-preserving refactor, so use existing characterization tests rather than adding source-layout assertions.

---

### Task 1: Split Option Command Responsibilities

**Files:**
- Modify: `src-tauri/src/commands/options.rs`
- Create: `src-tauri/src/commands/options/csv.rs`
- Create: `src-tauri/src/commands/options/contracts.rs`
- Create: `src-tauri/src/commands/options/simulation.rs`
- Create: `src-tauri/src/commands/options/tests.rs`

**Interfaces:**
- Consumes: `services::option_matching::{match_options_fifo, MatchRecord, SplitRecord}` and the existing option models.
- Produces: the unchanged `commands::options::{import_options_csv, get_option_contracts, get_option_contracts_inner, simulate_sell_put, simulate_sell_call, delete_option_records, export_options_csv, parse_options_csv, StockPriceInput, ImportOptionsResult}` paths.

- [ ] **Step 1: Record the option behavior baseline**

Run:

```bash
CARGO_TARGET_DIR=/Users/wensongzhang/stock-portfolio-manager/src-tauri/target cargo test --manifest-path src-tauri/Cargo.toml commands::options::tests -- --nocapture
```

Expected: all current option command tests pass, including malformed-number rejection, atomic rollback, split matching, FIFO status projection, and CSV round trip.

- [ ] **Step 2: Establish the private module facade**

Replace the implementation imports at the top of `options.rs` with this boundary:

```rust
mod contracts;
mod csv;
mod simulation;

pub use contracts::get_option_contracts_inner;
pub use csv::ImportOptionsResult;
pub use simulation::StockPriceInput;

#[cfg(test)]
mod tests;
```

Keep each `#[tauri::command]` function in `options.rs` as a thin adapter calling the matching child-module function.

- [ ] **Step 3: Move CSV parsing, import, and export as one unit**

Move these exact items to `options/csv.rs`: `parse_option_symbol`, `ParsedOptionRow`, `parsed_row_match_record`, `import_options_csv_inner`, `export_options_csv_inner`, preview construction from `parse_options_csv`, `ImportOptionsResult`, `normalize_action`, `parse_decimal`, `parse_required_decimal`, `parse_quantity`, `is_close_code`, `parse_expiry_to_sortable`, and `get_field`.

Expose only the facade entry points:

```rust
pub(super) fn import_options_csv_inner(
    db: &Database,
    account_id: &str,
    csv_content: &str,
) -> Result<ImportOptionsResult, String>;

pub(super) fn export_options_csv_inner(
    db: &Database,
    account_id: &str,
) -> Result<String, String>;

pub(super) fn parse_options_csv_inner(
    csv_content: &str,
) -> Result<crate::models::import_export::ImportPreview, String>;
```

Call `contracts::recompute_option_statuses_in` from the import path; do not copy its SQL into `csv.rs`.

- [ ] **Step 4: Move contract loading and status projection**

Move `load_matching_inputs`, `load_split_records`, `recompute_option_statuses`, `recompute_option_statuses_in`, and `get_option_contracts_inner` to `options/contracts.rs`. Keep `get_option_contracts_inner` publicly re-exported because `services/ai_tools.rs` imports it; keep the remaining helpers `pub(super)` or private.

- [ ] **Step 5: Move simulations and share-lot loading**

Move the bodies of `simulate_sell_put` and `simulate_sell_call` to `options/simulation.rs` as:

```rust
pub(super) fn simulate_sell_put_inner(
    db: &Database,
    account_id: &str,
    stock_prices: Vec<StockPriceInput>,
) -> Result<Vec<SellPutSimulation>, String>;

pub(super) fn simulate_sell_call_inner(
    db: &Database,
    account_id: &str,
    stock_prices: Vec<StockPriceInput>,
) -> Result<Vec<SellCallSimulation>, String>;
```

Extract their duplicated `option_share_lots` query into one private `load_share_lots(&Database)` helper inside the same module.

- [ ] **Step 6: Move the option characterization tests**

Move the existing `#[cfg(test)] mod tests` body verbatim into `options/tests.rs`, change its first import to `use super::*;`, and import private child helpers explicitly where required:

```rust
use super::contracts::recompute_option_statuses;
use super::csv::{export_options_csv_inner, get_field, import_options_csv_inner, normalize_action};
```

Do not weaken or delete any assertion.

- [ ] **Step 7: Verify the option split**

Run the baseline command from Step 1, followed by:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
CARGO_TARGET_DIR=/Users/wensongzhang/stock-portfolio-manager/src-tauri/target cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
```

Expected: the same option tests pass and strict Clippy emits no diagnostics.

### Task 2: Split Performance Service Responsibilities

**Files:**
- Modify: `src-tauri/src/services/performance_service.rs`
- Create: `src-tauri/src/services/performance_service/calculation.rs`
- Create: `src-tauri/src/services/performance_service/attribution.rs`
- Create: `src-tauri/src/services/performance_service/ranking.rs`
- Create: `src-tauri/src/services/performance_service/benchmark.rs`
- Create: `src-tauri/src/services/performance_service/tests.rs`

**Interfaces:**
- Consumes: `Database`, `models::performance::*`, `exchange_rate_service`, and `http_client`.
- Produces: every existing `services::performance_service::*` function at the same path, including `PerformanceFilter`, `get_performance_report`, calculation helpers, section services, and benchmark services.

- [ ] **Step 1: Record the performance behavior baseline**

Run:

```bash
CARGO_TARGET_DIR=/Users/wensongzhang/stock-portfolio-manager/src-tauri/target cargo test --manifest-path src-tauri/Cargo.toml services::performance_service::tests -- --nocapture
```

Expected: all performance tests pass, including one-load report construction, TWR cash-flow adjustment, malformed data propagation, attribution, ranking, and benchmark conversion.

- [ ] **Step 2: Establish the performance facade**

Add this private module structure to `performance_service.rs`:

```rust
mod attribution;
mod benchmark;
mod calculation;
mod ranking;

pub use attribution::get_return_attribution;
pub use benchmark::{
    benchmark_to_return_series, cache_benchmark_prices, fetch_benchmark_history,
    read_cached_benchmark,
};
pub use calculation::{
    build_twr_return_series, calculate_sharpe_from_daily_returns,
    calculate_volatility, get_drawdown_analysis, get_monthly_returns,
    get_performance_summary, get_risk_metrics,
};
pub use ranking::get_holding_performance_ranking;

#[cfg(test)]
mod tests;
```

Keep `PerformanceFilter` and `get_performance_report` in the facade.

- [ ] **Step 3: Move canonical loading and calculations**

Move `parse_required_exchange_rates`, all daily-value and cash-flow loaders, `PerformanceCalculation`, its test load counter, `build_twr_return_series`, `calculate_max_drawdown`, `calculate_volatility`, `calculate_sharpe_from_daily_returns`, the summary/drawdown/risk/monthly public functions, and their `*_from` helpers to `calculation.rs`.

Expose the shared context and report builders to sibling modules:

```rust
pub(super) struct PerformanceCalculation {
    pub(super) daily_values: Vec<(NaiveDate, f64, f64)>,
    pub(super) baseline: Option<(NaiveDate, f64)>,
    pub(super) external_cash_flows: Vec<(NaiveDate, f64)>,
    pub(super) return_series: Vec<ReturnDataPoint>,
}

pub(super) fn performance_summary_from(
    calculation: &PerformanceCalculation,
    requested_start_date: NaiveDate,
    requested_end_date: NaiveDate,
) -> PerformanceSummary;
pub(super) fn drawdown_analysis_from(
    calculation: &PerformanceCalculation,
) -> DrawdownAnalysis;
pub(super) fn risk_metrics_from(
    calculation: &PerformanceCalculation,
) -> RiskMetrics;
pub(super) fn monthly_returns_from(
    calculation: &PerformanceCalculation,
) -> Vec<MonthlyReturn>;
```

Mark `PerformanceCalculation::{load, start_date, end_date, start_value, end_value, total_external_cash_flow, total_pnl, total_return, calendar_days, daily_returns}` as `pub(super)` because attribution, ranking, and the facade call them.

- [ ] **Step 4: Move attribution and ranking without changing SQL**

Move `get_return_attribution` plus `return_attribution_from` to `attribution.rs`. Move `get_holding_performance_ranking` plus `holding_performance_ranking_from` and their local row/aggregation types to `ranking.rs`. Preserve every query string, filter clause, FX conversion, sort order, and fallback value byte-for-byte except for import paths and visibility.

- [ ] **Step 5: Move benchmark persistence and fetching**

Move `CACHE_COVERAGE_THRESHOLD`, `cache_benchmark_prices`, `read_cached_benchmark`, `fetch_benchmark_history`, and `benchmark_to_return_series` to `benchmark.rs`. Keep the shared HTTP client and URL unchanged.

- [ ] **Step 6: Reconnect aggregate report construction**

Import the four section builders into the facade and keep one canonical load:

```rust
let calculation = calculation::PerformanceCalculation::load(
    db, start_date, end_date, filter,
)?;
let report = PerformanceReport {
    summary: calculation::performance_summary_from(&calculation, start_date, end_date),
    drawdown: calculation::drawdown_analysis_from(&calculation),
    attribution: attribution::return_attribution_from(db, &calculation, filter)?,
    monthly_returns: calculation::monthly_returns_from(&calculation),
    holding_performances: ranking::holding_performance_ranking_from(
        db, &calculation, ranking_sort_by, ranking_limit, filter,
    )?,
    risk_metrics: calculation::risk_metrics_from(&calculation),
};
```

- [ ] **Step 7: Move performance characterization tests**

Move the existing test module body to `performance_service/tests.rs`. Use `use super::*;` for the unchanged public facade and add this exact private-helper import:

```rust
use super::calculation::{
    calculate_max_drawdown, fetch_previous_day_value, parse_required_exchange_rates,
    performance_load_count, reset_performance_load_count,
};
```

Preserve all literal expectations.

- [ ] **Step 8: Verify and commit the backend split**

Run:

```bash
CARGO_TARGET_DIR=/Users/wensongzhang/stock-portfolio-manager/src-tauri/target cargo test --manifest-path src-tauri/Cargo.toml commands::options::tests -- --nocapture
CARGO_TARGET_DIR=/Users/wensongzhang/stock-portfolio-manager/src-tauri/target cargo test --manifest-path src-tauri/Cargo.toml services::performance_service::tests -- --nocapture
cargo fmt --manifest-path src-tauri/Cargo.toml --all -- --check
CARGO_TARGET_DIR=/Users/wensongzhang/stock-portfolio-manager/src-tauri/target cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
git diff --check
```

Expected: all targeted tests pass, strict Clippy is clean, and no callers outside the two facades require path changes.

Commit:

```bash
git add src-tauri/src/commands/options.rs src-tauri/src/commands/options src-tauri/src/services/performance_service.rs src-tauri/src/services/performance_service
git commit -m "refactor: split option and performance responsibilities"
```
