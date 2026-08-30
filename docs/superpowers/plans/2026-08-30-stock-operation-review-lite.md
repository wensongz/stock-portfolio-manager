# Stock Operation Review Lite Implementation Plan

> **For Codex:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Use superpowers:test-driven-development for every behavior change and superpowers:verification-before-completion before reporting success.

**Goal:** Replace the stock-operation-review page and AI entry point with a lightweight, endpoint-based evaluation of each buy, add, reduce, and close operation, including estimated position impact and automatic market-benchmark comparison, without invoking the legacy performance/shadow/Campaign pipeline.

**Architecture:** Add an independent Rust report model, pure calculator, orchestration service, and Tauri command. The service reuses only low-level transaction replay, cached/fetched close prices, benchmark prices, snapshots, and FX conversion. A new frontend store and view model consume the lightweight report; the existing complex stock-review engine and commands remain present but dormant for rollback. Missing market, benchmark, FX, or NAV data is represented on the affected field and never becomes a report-wide availability state.

**Tech Stack:** Rust, Tauri 2, rusqlite, chrono, serde, TypeScript 7, React 19, Zustand 5, Ant Design 6, Node test runner, Cargo tests.

**Design source:** `docs/superpowers/specs/2026-08-30-stock-operation-review-lite-design.md`

**Working constraint:** Work directly on the current `main` checkout. Do not commit, rebase, reset, or discard existing uncommitted changes. Stop after verified working-tree changes so the user can run a manual acceptance test before deciding whether to commit.

---

## Task 1: Define the lightweight backend contract and pure endpoint formulas

**Files:**
- Create: `src-tauri/src/models/stock_operation_review.rs`
- Modify: `src-tauri/src/models/mod.rs`
- Create: `src-tauri/src/services/stock_operation_review_calculator.rs`
- Modify: `src-tauri/src/services/mod.rs`

- [ ] **Step 1: Write failing calculator tests for all four action directions**

Add unit tests in `stock_operation_review_calculator.rs` for:

```rust
#[test]
fn buy_and_add_gain_when_the_stock_rises() {
    let buy = input("open", 100.0, 10.0, 12.0, 5.0);
    assert_eq!(calculate_endpoint_effect(&buy).price_effect_local, Some(195.0));
    assert_eq!(calculate_endpoint_effect(&buy).price_effect_percent, Some(0.195));
}

#[test]
fn reduce_and_close_gain_when_the_stock_falls() {
    let sell = input("close", 100.0, 10.0, 8.0, 5.0);
    assert_eq!(calculate_endpoint_effect(&sell).price_effect_local, Some(195.0));
}

#[test]
fn sell_after_which_the_stock_rises_is_an_opportunity_loss() {
    let sell = input("reduce", 100.0, 10.0, 12.0, 5.0);
    assert_eq!(calculate_endpoint_effect(&sell).price_effect_local, Some(-205.0));
}
```

Also test zero notional, missing endpoint price, non-finite inputs, fee deduction, and the direction-adjusted benchmark formula.

- [ ] **Step 2: Run the targeted test and confirm it fails because the module does not exist**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review_calculator
```

Expected: FAIL with an unresolved module/type/function error.

- [ ] **Step 3: Add the serialized report model**

Define these public types in `models/stock_operation_review.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockOperationReviewQuery {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub account_id: Option<String>,
    pub market: Option<String>,
    pub base_currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockOperationReviewReport {
    pub query: StockOperationReviewQuery,
    pub summary: StockOperationReviewSummary,
    pub securities: Vec<StockOperationSecuritySummary>,
    pub actions: Vec<StockOperationEffect>,
    pub data_quality: StockOperationDataQuality,
    pub generated_at: String,
    pub algorithm_version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockOperationEffect {
    pub action_id: String,
    pub transaction_ids: Vec<String>,
    pub account_id: String,
    pub account_name: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub action_type: String,
    pub trade_date: NaiveDate,
    pub quantity: f64,
    pub trade_price: f64,
    pub trade_notional_local: f64,
    pub fee_local: f64,
    pub currency: String,
    pub shares_before: f64,
    pub shares_after: f64,
    pub prior_nav_date: Option<NaiveDate>,
    pub prior_nav_base: Option<f64>,
    pub weight_before: Option<f64>,
    pub weight_after: Option<f64>,
    pub weight_change: Option<f64>,
    pub operation_size_ratio: Option<f64>,
    pub evaluation_date: Option<NaiveDate>,
    pub end_price: Option<f64>,
    pub price_effect_local: Option<f64>,
    pub price_effect_base: Option<f64>,
    pub price_effect_percent: Option<f64>,
    pub benchmark_symbol: Option<String>,
    pub benchmark_start_date: Option<NaiveDate>,
    pub benchmark_end_date: Option<NaiveDate>,
    pub benchmark_return: Option<f64>,
    pub directional_excess_return: Option<f64>,
    pub fact_labels: Vec<String>,
    pub issues: Vec<StockOperationFieldIssue>,
}
```

Add summary types with explicit nullable fields and counts:

```rust
pub struct StockOperationReviewSummary {
    pub total: StockOperationGroupSummary,
    pub buys: StockOperationGroupSummary,
    pub sells: StockOperationGroupSummary,
    pub position_impact: StockPositionImpactSummary,
}

pub struct StockOperationGroupSummary {
    pub action_count: usize,
    pub positive_count: usize,
    pub negative_count: usize,
    pub missing_effect_count: usize,
    pub price_effect_base: Option<f64>,
    pub positive_notional_ratio: Option<f64>,
    pub weighted_excess_return: Option<f64>,
}

pub struct StockPositionImpactSummary {
    pub invested_amount_base: Option<f64>,
    pub recovered_amount_base: Option<f64>,
    pub largest_absolute_weight_change: Option<f64>,
    pub total_fees_base: Option<f64>,
    pub missing_weight_count: usize,
}
```

`StockOperationDataQuality` must contain only counts and short notes: `action_count`, `missing_end_price_count`, `missing_benchmark_count`, `missing_fx_count`, `missing_weight_count`, and `notes`. `StockOperationFieldIssue` must contain `code`, `field`, and `message`; do not add a report-wide `status` or `availability` field.

- [ ] **Step 4: Implement pure formulas and aggregation**

Implement:

```rust
pub fn calculate_endpoint_effect(input: &EndpointEffectInput) -> EndpointEffectOutput;
pub fn calculate_directional_excess(
    action_type: &str,
    stock_return: f64,
    benchmark_return: f64,
) -> Option<f64>;
pub fn summarize_actions(actions: &[StockOperationEffect]) -> StockOperationReviewSummary;
pub fn summarize_securities(actions: &[StockOperationEffect]) -> Vec<StockOperationSecuritySummary>;
```

Rules:

- `open`/`add`: `q * (end - trade) - fee`.
- `reduce`/`close`: `q * (trade - end) - fee`.
- Percentage denominator is absolute local notional.
- Group base-currency sums are `None` when any otherwise-price-evaluable member lacks base conversion; never present a partial amount as a complete total.
- Positive-notional ratio and weighted excess use absolute base notional and are `None` when required conversion is incomplete.
- Security summaries group by account, normalized market, and normalized symbol, while preserving a display name/code.
- Sorting is deterministic: available base effect descending, unavailable last, then market/symbol/account.

- [ ] **Step 5: Run the calculator tests and confirm they pass**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review_calculator
```

Expected: PASS.

- [ ] **Step 6: Checkpoint without committing**

Run `git diff --check` and inspect only the files from this task. Do not commit.

---

## Task 2: Build actions from the complete ledger and scope only the displayed operations

**Files:**
- Create: `src-tauri/src/services/stock_operation_review_service.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/stock_operation_review_service.rs`

- [ ] **Step 1: Write failing service tests for replay and query scoping**

Use an in-memory `Database` and insert transactions that cover:

- Opening position before `start_date`, then add/reduce/close inside the range.
- Multiple fills on the same account/symbol/day/direction.
- `PAY`, `$CASH-*`, synthetic `OPEN`, split records, and confirmed transfers.
- Account and market filters.
- Same symbol in two accounts.

Assertions must prove that complete history establishes correct `shares_before`/`shares_after`, while only action dates inside the query are returned.

- [ ] **Step 2: Run the service tests and confirm they fail**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review_service
```

Expected: FAIL because the service API is not implemented.

- [ ] **Step 3: Implement query validation and database loaders**

Add:

```rust
pub fn validate_query(query: &StockOperationReviewQuery) -> Result<(), String>;
pub async fn get_stock_operation_review(
    db: &Database,
    query: StockOperationReviewQuery,
) -> Result<StockOperationReviewReport, String>;
```

Validation rules:

- `start_date <= end_date`.
- `base_currency` is one of `USD`, `CNY`, `HKD`.
- `market`, when present, is `US`, `CN`, or `HK`.
- A selected `account_id` must exist; this is one of the few request-level failures.

The transaction query must load all rows through `end_date`, ordered by `traded_at`, `created_at`, and `id`. Load active overrides with `stock_review_persistence::list_overrides`; pass those and the complete transaction vector to `stock_action_builder::build_stock_actions`.

Project the builder output into lightweight action seeds, then filter seeds by action date, selected account, and selected market. Do not pass the report start date into the replay. Map account names from `accounts` and stock names from the source transaction IDs.

- [ ] **Step 4: Preserve operation-builder semantics without importing legacy metrics**

Explicitly reuse only:

```rust
stock_action_builder::build_stock_actions(...)
```

Do not call any of:

```rust
stock_review_service::get_stock_review_report
stock_review_calendar::*
shadow_portfolio_engine::*
rebalance_attribution::*
stock_campaign_builder::*
```

Convert only `open`, `add`, `reduce`, and `close`. Existing synthetic `OPEN`, cash, PAY, split, and confirmed-transfer behavior must remain governed by the builder. Builder issues relevant to an action may become field issues or quality notes, but cannot create a report-wide unavailable state.

- [ ] **Step 5: Run the service tests and confirm they pass**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review_service
```

Expected: PASS for replay, grouping, scoping, account validation, and exclusions.

- [ ] **Step 6: Checkpoint without committing**

Run `git diff --check`. Do not commit.

---

## Task 3: Resolve endpoint stock prices and automatic benchmarks without a calendar dependency

**Files:**
- Modify: `src-tauri/src/services/stock_operation_review_service.rs`
- Reuse: `src-tauri/src/services/stock_review_market_data.rs`
- Reuse: `src-tauri/src/services/performance_service.rs`
- Test: `src-tauri/src/services/stock_operation_review_service.rs`

- [ ] **Step 1: Write failing endpoint-resolution tests**

Create pure fixtures for stock and benchmark observations and cover:

- The endpoint is the last real stock close in `[action_date, query.end_date]`.
- A newly listed stock with a pre-listing action date uses its first/last real post-listing close and does not demand a quote before listing.
- A weekend/holiday end date uses the last actual observation before the query end.
- No post-action stock close yields only `end_price`, `evaluation_date`, and price-effect fields as `None` plus `missing_end_price`.
- Benchmark start is the last point on/before the action date within seven calendar days.
- Benchmark end is the last point on/before the stock evaluation date within seven calendar days.
- Stale or missing benchmark points hide only benchmark fields.
- CN/HK/US map to `000300.SS`, `^HSI`, and `^GSPC` respectively.

- [ ] **Step 2: Run the targeted tests and confirm they fail**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review_service endpoint
```

Expected: FAIL before endpoint resolution exists.

- [ ] **Step 3: Implement stock endpoint loading through the existing quote/cache path**

For each unique `(normalized_symbol, normalized_market)`, find the earliest in-scope action date and call:

```rust
ensure_stock_price_cache(
    db,
    symbol,
    market,
    earliest_action_date,
    query.end_date,
    None, // deliberately no exchange-session calendar
    provider,
).await
```

`expected_sessions = None` is required: it fills leading/trailing cache ranges and uses the same quote-provider path as other analysis pages without making the lightweight report depend on an authoritative calendar. If refresh fails, reload `load_stock_price_series` and continue with cached points. Never turn a network failure into a request failure.

For each action, choose `max(point.date)` where `action_date <= point.date <= end_date` and `close` is finite and positive. Record the actual date.

- [ ] **Step 4: Implement automatic benchmark loading with cached fallback**

Use `stock_review_market_data::default_benchmark_symbol`. For each required benchmark:

1. Read `performance_service::read_cached_benchmark` from `earliest_action_date - 7 days` through `end_date`.
2. Attempt `performance_service::fetch_benchmark_history` for the same range.
3. If the fetch fails, retain the cached vector.
4. Sort and deduplicate by date.

Resolve start/end points with the seven-calendar-day staleness rule. Calculate stock return from `trade_price` to `end_price`, benchmark return from the two resolved benchmark endpoints, then use `calculate_directional_excess`.

- [ ] **Step 5: Add fact labels based only on observed outcomes**

Generate deterministic labels such as:

- `买入后上涨` / `买入后下跌`.
- `卖出后下跌` / `卖出后继续上涨`.
- `买入后跑赢基准` / `买入后跑输基准`.
- `卖出方向跑赢基准` / `卖出方向跑输基准`.
- `期末行情不足` / `基准数据不足`.

Do not emit “正确操作”, “错误操作”, or any statement about investment thesis quality.

- [ ] **Step 6: Run endpoint tests and confirm they pass**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review_service endpoint
```

Expected: PASS, including the newly listed stock fixture.

- [ ] **Step 7: Checkpoint without committing**

Run `git diff --check`. Do not commit.

---

## Task 4: Add endpoint FX conversion and estimated position weights with field-level degradation

**Files:**
- Modify: `src-tauri/src/services/stock_operation_review_service.rs`
- Modify: `src-tauri/src/services/stock_operation_review_calculator.rs`
- Test: `src-tauri/src/services/stock_operation_review_service.rs`

- [ ] **Step 1: Write failing tests for all-account and selected-account NAV**

Fixtures must cover:

- All accounts: use the latest `daily_portfolio_values` row strictly before the action and convert its USD `total_value` to the report base currency.
- Selected account: sum all that account's `daily_holding_snapshots` rows, including explicit `$CASH-*`, on the latest valid date strictly before the action.
- A market filter limits actions but does not filter the NAV denominator.
- Missing explicit cash for a selected account hides weights only.
- Missing NAV hides `prior_nav_base`, weights, and `operation_size_ratio`, while shares, local amount, local price effect, and benchmark comparison remain.
- Missing trade-date FX hides weights/base notional only.
- Missing evaluation-date FX hides `price_effect_base` and aggregate base amount only.
- Base currency equal to transaction currency uses rate `1.0` and requires no FX row.

- [ ] **Step 2: Run the weight tests and confirm they fail**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review_service weight
```

Expected: FAIL before the NAV/FX resolver is implemented.

- [ ] **Step 3: Implement prior NAV resolution**

Add internal resolvers:

```rust
fn resolve_prior_nav(
    db: &Database,
    account_id: Option<&str>,
    action_date: NaiveDate,
    base_currency: &str,
) -> Result<Option<ResolvedNav>, String>;

fn resolve_fx_on_or_before(
    db: &Database,
    currency: &str,
    base_currency: &str,
    date: NaiveDate,
    max_age_days: i64,
) -> Result<Option<ResolvedFx>, String>;
```

Rules:

- NAV date is the latest valid snapshot strictly before the action; there is no exact-session requirement.
- Selected-account NAV must include stock and an explicit cash row and must not apply the market filter.
- For endpoint amount conversion, use the last FX observation on/before the evaluation date within seven calendar days.
- For a weight estimate, use the FX embedded in the chosen prior NAV date so numerator and denominator share a valuation basis.
- Reject non-finite or non-positive NAV and FX values as missing data.

- [ ] **Step 4: Calculate and label position impact**

Use:

```rust
weight_before = shares_before * trade_price * trade_fx / prior_nav_base;
weight_after = shares_after * trade_price * trade_fx / prior_nav_base;
weight_change = weight_after - weight_before;
operation_size_ratio = abs(trade_notional_local * trade_fx) / prior_nav_base;
```

All four are estimates. Add `大幅提高仓位` or `大幅降低仓位` only when `abs(weight_change) >= 0.05`; add `权重数据不足` when the estimate is absent.

Convert fees, invested/recovered notional, and price effect to base currency consistently. Summary totals become `None` if a required member conversion is missing; counts still remain correct.

- [ ] **Step 5: Add data-quality counts without a global status**

Build `StockOperationDataQuality` from action fields. Notes must be short Chinese explanations such as “2 项操作缺少基准端点，仅隐藏相对基准字段”. Do not use severity `blocking`, `unavailable`, `degraded`, or legacy issue codes such as `market_calendar_unavailable`.

- [ ] **Step 6: Run the service and calculator tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review_service
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review_calculator
```

Expected: PASS.

- [ ] **Step 7: Checkpoint without committing**

Run `git diff --check`. Do not commit.

---

## Task 5: Expose the lightweight Tauri command and switch the AI read tool

**Files:**
- Modify: `src-tauri/src/commands/review.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/services/ai_tools.rs`
- Modify: `src-tauri/src/skills/stock-review.md`
- Test: `src-tauri/src/commands/review.rs`
- Test: `src-tauri/src/services/ai_tools.rs`

- [ ] **Step 1: Write failing command/query tests**

Add tests that parse camelCase Tauri arguments into `StockOperationReviewQuery`, normalize market/base currency, reject invalid dates/currency/account, and prove that the new command has no `benchmark_symbol` or Campaign argument.

- [ ] **Step 2: Run targeted tests and confirm they fail**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib commands::review
cargo test --manifest-path src-tauri/Cargo.toml --lib services::ai_tools::tests::stock_review
```

Expected: FAIL until command and tool schema are switched.

- [ ] **Step 3: Add and register the new command**

Add:

```rust
#[tauri::command(rename_all = "camelCase")]
pub async fn get_stock_operation_review(
    start_date: String,
    end_date: String,
    account_id: Option<String>,
    market: Option<String>,
    base_currency: String,
    db: State<'_, Database>,
) -> Result<StockOperationReviewReport, String>;
```

Register `commands::review::get_stock_operation_review` in `src-tauri/src/lib.rs`. Do not call snapshot backfill from this command: the service consumes existing snapshots opportunistically and missing weight data is field-level.

Keep all existing legacy commands registered for rollback, but the new frontend must call only this command.

- [ ] **Step 4: Change `get_stock_review` AI tool to consume the new report**

Keep the tool name `get_stock_review` for saved prompts and user familiarity, but change its schema to:

```json
{
  "start_date": "YYYY-MM-DD",
  "end_date": "YYYY-MM-DD",
  "base_currency": "USD|CNY|HKD",
  "account_id": "optional",
  "market": "optional US|CN|HK",
  "symbol": "optional post-filter"
}
```

Remove `benchmark_symbol` and `campaign_id`. Parse the lightweight query, call `stock_operation_review_service::get_stock_operation_review`, and when `symbol` is supplied retain matching actions/securities and recalculate the lightweight summaries from that subset rather than returning stale whole-report summary numbers.

Tool output must serialize the same report shape/numbers as the page. The AI is allowed to interpret, not recalculate endpoint effects.

- [ ] **Step 5: Rewrite the bundled stock-review skill instructions**

The skill must ask the AI to analyze:

1. Largest positive and negative price effects.
2. Buy/add versus reduce/close effectiveness.
3. Large estimated weight changes with weak endpoint results.
4. Direction-adjusted benchmark winners and laggards.
5. At most three missing investment-thesis/target-weight/sell-reason questions.

It must state that endpoint effects are hindsight price comparisons, exclude unallocated dividends, are not TWR attribution, and cannot alone label a decision right or wrong. Remove Campaign, 60/120-session, shadow-portfolio, drawdown, and complex data-quality instructions.

- [ ] **Step 6: Run command and AI tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib commands::review
cargo test --manifest-path src-tauri/Cargo.toml --lib services::ai_tools
```

Expected: PASS.

- [ ] **Step 7: Checkpoint without committing**

Run `git diff --check`. Do not commit.

---

## Task 6: Add lightweight frontend types, filters, view model, and race-safe store

**Files:**
- Modify: `src/types/index.ts`
- Create: `src/stores/stockOperationReviewStore.ts`
- Create: `src/stores/stockOperationReviewStore.test.ts`
- Create: `src/pages/Review/stockOperationReviewViewModel.ts`
- Create: `src/pages/Review/stockOperationReviewViewModel.test.ts`
- Modify: `src/pages/Review/StockReviewFilters.tsx`
- Modify: `src/pages/AiAssistant/prefill.ts`
- Modify: `src/pages/AiAssistant/prefill.test.ts`

- [ ] **Step 1: Write failing view-model tests**

Test:

- Default QTD/previous-quarter/YTD/one-year/custom dates.
- Saved filters contain only account, period, dates, market, and base currency.
- Old local-storage objects containing `benchmarkSymbol` migrate by ignoring that field.
- Summary card view models distinguish complete values from missing fields without a global status.
- Ranking puts unavailable effects last and supports effect, notional, benchmark, and weight-change sort keys.
- Currency/percent/weight formatters return `—` for `null` without conflating it with zero.
- AI prefill uses `get_stock_review` without benchmark or Campaign arguments.

- [ ] **Step 2: Write failing store tests**

Mock `invoke` and prove:

- The command is exactly `get_stock_operation_review`.
- Arguments are `{ startDate, endDate, accountId, market, baseCurrency }`.
- Latest request wins when responses resolve out of order.
- A refresh error keeps the last successful report and exposes a non-blocking error.
- A first-load error exposes an error state.

- [ ] **Step 3: Run the frontend tests and confirm they fail**

Run:

```bash
node --test src/pages/Review/stockOperationReviewViewModel.test.ts src/stores/stockOperationReviewStore.test.ts src/pages/AiAssistant/prefill.test.ts
```

Expected: FAIL because the lightweight modules/types do not exist.

- [ ] **Step 4: Add TypeScript interfaces matching Rust exactly**

Add `StockOperationReviewFilters`, `StockOperationReviewQuery`, `StockOperationReviewReport`, `StockOperationReviewSummary`, `StockOperationGroupSummary`, `StockPositionImpactSummary`, `StockOperationEffect`, `StockOperationSecuritySummary`, `StockOperationDataQuality`, and `StockOperationFieldIssue`.

Keep the legacy `StockReview*` interfaces until the legacy engine is removed. Do not coerce the new report into the old type.

- [ ] **Step 5: Implement the isolated store and view model**

The new Zustand store exposes only:

```ts
interface StockOperationReviewState {
  report: StockOperationReviewReport | null;
  loading: boolean;
  error: string | null;
  loadReport(filters: StockOperationReviewFilters): Promise<void>;
  clearError(): void;
}
```

Use a monotonic request token so an old response cannot overwrite a newer filter selection. Retain the last successful report on refresh failure.

The new view model owns filter storage under a versioned key such as `stock-operation-review-filters-v1`. It builds AI prefill from the lightweight filter shape.

- [ ] **Step 6: Simplify `StockReviewFilters`**

Change the component prop type to `StockOperationReviewFilters`. Retain account, period, custom date range, market, and base currency. Remove benchmark mode/code state and inputs entirely. Keep refresh and AI buttons.

- [ ] **Step 7: Run frontend tests and confirm they pass**

Run:

```bash
node --test src/pages/Review/stockOperationReviewViewModel.test.ts src/stores/stockOperationReviewStore.test.ts src/pages/AiAssistant/prefill.test.ts
```

Expected: PASS.

- [ ] **Step 8: Checkpoint without committing**

Run `git diff --check`. Do not commit.

---

## Task 7: Replace the stock-review page with four cards, ranking, action table, and compact quality notes

**Files:**
- Create: `src/pages/Review/StockOperationReviewSummaryCards.tsx`
- Create: `src/pages/Review/StockOperationSecurityTable.tsx`
- Create: `src/pages/Review/StockOperationActionsTable.tsx`
- Create: `src/pages/Review/StockOperationReviewQuality.tsx`
- Modify: `src/pages/Review/StockReviewTab.tsx`
- Test: `src/pages/Review/stockOperationReviewViewModel.test.ts`

- [ ] **Step 1: Add failing presentation tests to the pure view model**

Avoid adding a browser test framework. Test pure projection functions for:

- Four card titles and metrics.
- Buy/add and reduce/close grouping.
- Positive/negative/missing counts.
- Partial field notes.
- Fact-label rendering inputs.
- Correct Chinese wording for sell gains (`避损`) versus losses (`机会损失`).
- Empty action set.

- [ ] **Step 2: Run the view-model tests and confirm they fail**

Run:

```bash
node --test src/pages/Review/stockOperationReviewViewModel.test.ts
```

Expected: FAIL before presentation projections exist.

- [ ] **Step 3: Implement the four summary cards**

Render:

1. `操作总效果`: base-currency endpoint effect, weighted relative benchmark, positive/negative/missing counts.
2. `买入与加仓`: endpoint effect, positive-notional ratio, weighted relative benchmark.
3. `减仓与清仓`: avoid-loss/opportunity-loss effect, positive-notional ratio, weighted relative benchmark.
4. `仓位影响`: invested amount, recovered amount, largest estimated absolute weight change, fees, missing-weight count.

Each value renders independently. A null field gets `—` plus a short local explanation; cards never display `不可用`, `降级`, or a global status badge.

- [ ] **Step 4: Implement the stock ranking table**

Columns: stock/account, action counts, net shares, buy/sell amount, base effect, weighted relative benchmark, largest estimated weight change, and positive/negative/missing counts. Default sort is base effect descending, with missing effects last. Provide the other design-specified sortable columns.

- [ ] **Step 5: Implement the operation detail table**

Columns/groups:

- Trade: date, account, stock, action type, quantity, weighted trade price, notional, fee.
- Position: shares before/after, estimated weights before/after/change.
- Endpoint: actual evaluation date, endpoint price, local/base effect, effect percent.
- Benchmark: automatic symbol, benchmark return, directional excess.
- Evidence: fact labels and field-level issue tooltip.

Use a horizontal scroll container and existing table page-size conventions. Never hide an entire row because one field is missing.

- [ ] **Step 6: Implement the compact quality component**

Show a neutral sentence such as “共分析 12 项操作；1 项缺少期末价，2 项缺少基准，3 项缺少权重估算”. Expand only when notes exist. Do not render the old yellow report-wide availability alert or the long methodology list.

- [ ] **Step 7: Rewrite `StockReviewTab` to use only the lightweight path**

The page must import `useStockOperationReviewStore` and the new components. Remove all runtime imports/calls for:

- `useStockReviewStore`.
- `PortfolioComparisonChart`.
- `RebalanceAttributionPanel`.
- `RiskStructurePanel`.
- `StockReviewSummaryCards`.
- `StockCampaignDrawer` and Campaign handlers.
- `LegacyStockReviewPanel`.
- Annotation and override mutations.

Loading copy becomes “正在生成股票操作效果复盘…”. Empty copy becomes “所选区间没有可评价的股票买卖操作”. AI navigation uses the lightweight prefill.

- [ ] **Step 8: Run presentation tests and TypeScript build**

Run:

```bash
node --test src/pages/Review/stockOperationReviewViewModel.test.ts src/stores/stockOperationReviewStore.test.ts
npm run build
```

Expected: PASS, with no TypeScript references from the new page to the legacy report type/store.

- [ ] **Step 9: Checkpoint without committing**

Run `git diff --check`. Do not commit.

---

## Task 8: Prove the lightweight page does not invoke the legacy pipeline and complete regression verification

**Files:**
- Modify: `src/stores/stockOperationReviewStore.test.ts`
- Modify: `src/pages/Review/stockOperationReviewViewModel.test.ts`
- Modify as needed from test findings only

- [ ] **Step 1: Add a frontend contract test for the command boundary**

Assert the new store never invokes any legacy command name:

```ts
assert.notEqual(command, "get_stock_review_report");
assert.notEqual(command, "get_stock_campaign_detail");
assert.notEqual(command, "confirm_stock_review_override");
assert.equal(command, "get_stock_operation_review");
```

Also search the compiled page source imports to ensure `StockReviewTab.tsx` contains none of the removed legacy component/store names.

- [ ] **Step 2: Run Rust formatting and targeted tests**

Run:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review
cargo test --manifest-path src-tauri/Cargo.toml --lib services::ai_tools
```

Expected: PASS.

- [ ] **Step 3: Run the complete Rust test suite**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib
```

Expected: PASS. Existing ignored tests may remain ignored; no new ignored tests.

- [ ] **Step 4: Run the complete frontend test suite**

Run the repository's existing Node test list plus the two new files:

```bash
node --test src/hooks/tablePageSize.test.ts src/pages/AiAssistant/prefill.test.ts src/pages/AiAssistant/sidebarPreference.test.ts src/pages/Options/expiredOptionsViewModel.test.ts src/pages/Quarterly/aggregateSnapshotHoldings.test.mjs src/pages/Review/optionReviewViewModel.test.ts src/pages/Review/reviewTabPreference.test.ts src/pages/Review/stockReviewDateBoundary.test.ts src/pages/Review/stockReviewViewModel.test.ts src/pages/Review/stockOperationReviewViewModel.test.ts src/pages/Statistics/categoryHoldings.test.ts src/stores/chatStore.test.ts src/stores/optionReviewStore.test.ts src/stores/optionStore.test.ts src/stores/quoteErrors.test.ts src/stores/stockReviewStore.test.ts src/stores/stockOperationReviewStore.test.ts
```

Expected: PASS.

- [ ] **Step 5: Run build and lint-level verification**

Run:

```bash
npm run build
cargo clippy --manifest-path src-tauri/Cargo.toml --lib -- -D warnings
git diff --check
```

If strict clippy is blocked only by documented pre-existing warnings in untouched code, rerun ordinary `cargo clippy --manifest-path src-tauri/Cargo.toml --lib`, record the exact warnings, and do not broaden this feature into unrelated cleanup.

- [ ] **Step 6: Perform a source-level acceptance audit**

Run:

```bash
rg -n "useStockReviewStore|get_stock_review_report|StockCampaignDrawer|PortfolioComparisonChart|RebalanceAttributionPanel|RiskStructurePanel|LegacyStockReviewPanel" src/pages/Review/StockReviewTab.tsx src/stores/stockOperationReviewStore.ts
rg -n "stock_review_calendar|shadow_portfolio_engine|rebalance_attribution|stock_campaign_builder|get_stock_review_report" src-tauri/src/services/stock_operation_review_service.rs src-tauri/src/commands/review.rs
```

Expected: the first search has no matches; the new service has no legacy calculation calls. The command file may still contain the separately retained old command, so inspect any command-file match and confirm it is not called by `get_stock_operation_review`.

- [ ] **Step 7: Hand off for manual acceptance without committing**

Report:

- Exact changed files.
- Exact test/build results.
- Any remaining field-level data gaps observed in fixtures.
- That no commit was created.
- Manual checks: open the stock-operation-review page, verify four cards/ranking/table, switch account/period/market/base currency, confirm no global “不可用”, and compare one buy plus one sell calculation by hand.

Do not commit until the user confirms the manual test is correct.
