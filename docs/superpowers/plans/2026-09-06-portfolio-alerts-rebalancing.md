# Investment Alerts and Portfolio Rebalancing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the left navigation's price-alert entry with an “投资提醒” workspace that preserves the existing price-alert behavior and adds independently configurable portfolio-allocation alerts, recoverable breach notifications, and one-click AI rebalancing advice.

**Architecture:** Keep allocation math and breach transitions deterministic in Rust, persist one configuration per scope in SQLite, and evaluate active configurations only from cached quotes and exchange rates after the existing refresh pipeline completes. The React page owns scope selection and editing, while the existing full-page AI assistant receives a trusted configuration ID, resolves the current rebalancing context on the backend, creates a fresh session, and auto-sends a constrained recommendation request.

**Tech Stack:** Rust, rusqlite, Tauri 2, Tokio, TypeScript, React 19, Zustand, Ant Design 6, ECharts, Node test runner, Cargo tests.

**Spec:** `docs/superpowers/specs/2026-09-06-portfolio-alerts-rebalancing-design.md`

## Global Constraints

- “投资类别” always means the user-defined categories from Settings; deleted categories cascade out of saved targets.
- Each overall, market, or account scope has its own configuration. Scope identity is unique and immutable after creation.
- Target percentages must total exactly 100% within a tolerance of `0.01` percentage points. The default relative-deviation threshold is 20%, and the default single-position concentration threshold is 20%.
- Category breach math is `abs(current_percent - target_percent) / target_percent * 100`, with a strict `>` comparison. Equality is normal. A zero target is normal only when the current percentage is zero; any positive holding breaches it.
- Cash participates in total value and category allocation, but never in single-position concentration. Uncategorized holdings use a virtual “未分类” category with target 0%.
- Overall and market scopes aggregate the same `market + symbol` across accounts before concentration checks.
- Missing required quotes or exchange rates produce `INCOMPLETE`. An incomplete run cannot create or recover breaches and must retain the prior valid snapshot as stale. Overall configurations persist the Settings base currency used when they are saved so background evaluation never needs browser local storage; market and account scopes use their native market currency.
- Persist only active breaches. `normal -> breach` notifies once, `breach -> breach` is silent, `breach -> normal` removes the active breach, and a later breach notifies again.
- The rebalancing amount uses the deterministic current snapshot and assumes no additional capital. AI may recommend new symbols, but must label them as candidates requiring verification and must stay within the selected scope's market.
- The AI action is disabled unless the current evaluation is valid and has an active allocation or concentration breach. It always opens the existing full-page assistant in a new session and auto-sends once.
- No task may add order placement, brokerage execution, or automatic trading.
- Never hold the SQLite mutex across an `.await`; load rows into owned values, release the lock, then perform async cache/service work. Database lock and serialization errors must propagate without `unwrap` in production paths.

## Canonical Contracts

Use these serialized values and field names across Rust, Tauri, and TypeScript:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortfolioAlertScopeKind {
    Overall,
    Market,
    Account,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PortfolioAlertScope {
    pub kind: PortfolioAlertScopeKind,
    pub market: Option<String>,
    pub account_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortfolioAlertDataStatus {
    Ready,
    Empty,
    Incomplete,
    InvalidConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortfolioAlertBreachKind {
    CategoryDeviation,
    Concentration,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AllocationDirection {
    Overweight,
    Underweight,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PortfolioAlertBreachDirection {
    Overweight,
    Underweight,
    AboveLimit,
}
```

The stable scope key and breach keys are:

```text
overall
market:CN
market:US
market:HK
account:<account_id>

category:<category_id>
category:uncategorized
security:<market>:<normalized_symbol>
```

## Target File Map

```text
src-tauri/src/
├── commands/
│   ├── alerts.rs                         # existing price-alert commands remain
│   ├── portfolio_alerts.rs               # configuration and evaluation commands
│   ├── ai.rs                             # DB-aware trusted scope validation
│   └── reset.rs                          # clears new tables on factory reset
├── db/
│   ├── migrations.rs                     # schema version 4
│   ├── schema.rs                         # three portfolio-alert tables
│   └── tests.rs                          # migration and cascade coverage
├── models/
│   ├── mod.rs
│   └── portfolio_alert.rs                # canonical API/domain types
├── services/
│   ├── alert_service.rs                  # existing price-alert service remains
│   ├── portfolio_alert_calculator.rs     # pure allocation/concentration math
│   ├── portfolio_alert_service.rs        # persistence, scope loading, transitions
│   ├── portfolio_read_service.rs         # cache-only completeness metadata
│   ├── ai_chat_service.rs                # trusted prefill validation and derived scope
│   ├── ai_tools.rs                       # rebalance context and market-bound tools
│   └── skill_service.rs                  # built-in skill registration
├── skills/
│   └── portfolio-rebalance.md            # rebalancing output contract
└── lib.rs                                # commands and post-refresh evaluation

src/
├── components/Layout/MainLayout.tsx      # menu label becomes 投资提醒
├── pages/Alerts/
│   ├── index.tsx                         # two-tab page shell
│   ├── alertsCopy.ts                     # tested tab/menu copy
│   ├── PriceAlertsTab.tsx                # extracted existing page behavior
│   ├── PortfolioAlertsTab.tsx            # scope/config/status UI
│   ├── portfolioAlertViewModel.ts        # validation and chart/row derivation
│   └── portfolioAlertViewModel.test.ts
├── pages/AiAssistant/
│   ├── index.tsx                         # consumes one atomic prefill request
│   ├── prefill.ts
│   ├── prefill.test.ts
│   ├── portfolioRebalancePrefill.ts
│   ├── portfolioRebalancePrefill.test.ts
│   ├── aiPrefillAutoSend.ts
│   ├── aiPrefillAutoSend.test.ts
│   └── ChatPanel.tsx                     # guarded one-shot auto-send
├── stores/
│   ├── portfolioAlertStore.ts
│   └── portfolioAlertStore.test.ts
└── types/
    ├── index.ts                          # exports portfolio-alert types
    └── portfolioAlert.ts                 # TypeScript mirror of Rust contract
```

---

### Task 1: Add schema version 4 and canonical portfolio-alert models

**Files:**
- Modify: `src-tauri/src/db/schema.rs`
- Modify: `src-tauri/src/db/migrations.rs`
- Modify: `src-tauri/src/db/tests.rs`
- Modify: `src-tauri/src/commands/reset.rs`
- Create: `src-tauri/src/models/portfolio_alert.rs`
- Modify: `src-tauri/src/models/mod.rs`

- [ ] **Step 1: Write failing migration tests**

Add tests that create a version-3 database, run migrations, and assert the new tables, uniqueness rules, and category cascade behavior:

```rust
#[test]
fn migration_v4_adds_portfolio_alert_tables_without_touching_price_alerts() {
    let db = version_three_database_with_price_alert();

    run_migrations(&db.conn).unwrap();

    assert_eq!(schema_version(&db.conn), 4);
    for table in [
        "portfolio_alert_configs",
        "portfolio_alert_targets",
        "portfolio_alert_breaches",
    ] {
        assert!(table_exists(&db.conn, table));
    }
    assert_eq!(price_alert_count(&db.conn), 1);
}

#[test]
fn portfolio_alert_scope_is_unique_and_deleted_category_targets_cascade() {
    let db = migrated_database();
    seed_account_and_category(&db.conn, "acct-1", "cat-growth");
    insert_market_config(&db.conn, "config-1", "US");
    insert_target(&db.conn, "config-1", "cat-growth", 60.0);

    let duplicate = insert_market_config(&db.conn, "config-2", "US");
    assert!(duplicate.is_err());

    db.conn.execute("DELETE FROM categories WHERE id = 'cat-growth'", []).unwrap();
    assert_eq!(target_count(&db.conn, "config-1"), 0);
}

#[test]
fn factory_reset_clears_all_portfolio_alert_rows() {
    let mut db = migrated_database_with_portfolio_config_target_and_breach();
    reset_database_state(&mut db.conn, "2026-09-06T10:00:00Z").unwrap();

    assert_eq!(row_count(&db.conn, "portfolio_alert_breaches"), 0);
    assert_eq!(row_count(&db.conn, "portfolio_alert_targets"), 0);
    assert_eq!(row_count(&db.conn, "portfolio_alert_configs"), 0);
}
```

- [ ] **Step 2: Run the migration tests and confirm the intended failure**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml db::tests::migration_v4 -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml db::tests::portfolio_alert_scope -- --nocapture
```

Expected: failure because schema version 4 and the portfolio-alert tables do not exist.

- [ ] **Step 3: Add the v4 schema in one idempotent helper**

Create these tables in `schema.rs` and call the same helper from both current-schema creation and `migrate_v4`:

```sql
CREATE TABLE IF NOT EXISTS portfolio_alert_configs (
    id TEXT PRIMARY KEY,
    scope_key TEXT NOT NULL UNIQUE,
    scope_kind TEXT NOT NULL CHECK (scope_kind IN ('OVERALL', 'MARKET', 'ACCOUNT')),
    market TEXT,
    account_id TEXT REFERENCES accounts(id) ON DELETE CASCADE,
    base_currency TEXT NOT NULL CHECK (base_currency IN ('USD', 'CNY', 'HKD')),
    deviation_threshold REAL NOT NULL DEFAULT 20 CHECK (deviation_threshold >= 0 AND deviation_threshold <= 100),
    concentration_threshold REAL NOT NULL DEFAULT 20 CHECK (concentration_threshold > 0 AND concentration_threshold <= 100),
    is_active INTEGER NOT NULL DEFAULT 1 CHECK (is_active IN (0, 1)),
    last_snapshot_json TEXT,
    last_evaluated_at TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK (
        (scope_kind = 'OVERALL' AND market IS NULL AND account_id IS NULL) OR
        (scope_kind = 'MARKET' AND market IN ('CN', 'US', 'HK') AND account_id IS NULL) OR
        (scope_kind = 'ACCOUNT' AND market IS NULL AND account_id IS NOT NULL)
    )
);

CREATE TABLE IF NOT EXISTS portfolio_alert_targets (
    config_id TEXT NOT NULL REFERENCES portfolio_alert_configs(id) ON DELETE CASCADE,
    category_id TEXT NOT NULL REFERENCES categories(id) ON DELETE CASCADE,
    target_percent REAL NOT NULL CHECK (target_percent >= 0 AND target_percent <= 100),
    PRIMARY KEY (config_id, category_id)
);

CREATE TABLE IF NOT EXISTS portfolio_alert_breaches (
    config_id TEXT NOT NULL REFERENCES portfolio_alert_configs(id) ON DELETE CASCADE,
    breach_key TEXT NOT NULL,
    breach_kind TEXT NOT NULL CHECK (breach_kind IN ('CATEGORY_DEVIATION', 'CONCENTRATION')),
    direction TEXT NOT NULL CHECK (direction IN ('OVERWEIGHT', 'UNDERWEIGHT', 'ABOVE_LIMIT')),
    first_triggered_at TEXT NOT NULL,
    last_seen_at TEXT NOT NULL,
    PRIMARY KEY (config_id, breach_key)
);
```

Set `CURRENT_SCHEMA_VERSION` to `4`, keep the migration transactional, and add indexes on `portfolio_alert_configs(is_active)` and `portfolio_alert_breaches(config_id)`.

Add the three tables to `reset_database_state` in child-to-parent order—breaches, targets, configs—before accounts and categories are deleted.

- [ ] **Step 4: Add the Rust model module**

Define the canonical enums plus these structures with `#[serde(rename_all = "camelCase")]` fields:

```rust
pub struct PortfolioAlertTarget {
    pub category_id: String,
    pub target_percent: f64,
}

pub struct PortfolioAlertConfig {
    pub id: String,
    pub scope: PortfolioAlertScope,
    pub base_currency: String,
    pub deviation_threshold: f64,
    pub concentration_threshold: f64,
    pub is_active: bool,
    pub targets: Vec<PortfolioAlertTarget>,
    pub last_snapshot: Option<PortfolioAlertSnapshot>,
    pub last_evaluated_at: Option<String>,
}

pub struct SavePortfolioAlertConfigInput {
    pub id: Option<String>,
    pub scope: PortfolioAlertScope,
    pub base_currency: String,
    pub deviation_threshold: f64,
    pub concentration_threshold: f64,
    pub is_active: bool,
    pub targets: Vec<PortfolioAlertTarget>,
}

pub struct CategoryAllocation {
    pub category_id: Option<String>,
    pub category_name: String,
    pub category_color: String,
    pub category_icon: String,
    pub target_percent: f64,
    pub current_percent: f64,
    pub relative_deviation_percent: Option<f64>,
    pub current_market_value: f64,
    pub target_market_value: f64,
    pub rebalance_amount: f64,
    pub direction: Option<AllocationDirection>,
}

pub struct ConcentrationAlert {
    pub market: String,
    pub symbol: String,
    pub normalized_symbol: String,
    pub name: String,
    pub category_id: Option<String>,
    pub market_value: f64,
    pub position_percent: f64,
    pub threshold_percent: f64,
}

pub struct PortfolioAlertSnapshot {
    pub config_id: String,
    pub scope: PortfolioAlertScope,
    pub base_currency: String,
    pub evaluated_at: String,
    pub total_market_value: f64,
    pub categories: Vec<CategoryAllocation>,
    pub concentrations: Vec<ConcentrationAlert>,
}

pub struct MissingPortfolioAlertData {
    pub market: Option<String>,
    pub symbol: Option<String>,
    pub currency: Option<String>,
    pub reason: String,
}

pub struct PortfolioAlertBreach {
    pub config_id: String,
    pub breach_key: String,
    pub breach_kind: PortfolioAlertBreachKind,
    pub direction: PortfolioAlertBreachDirection,
    pub first_triggered_at: String,
    pub last_seen_at: String,
}

pub struct PortfolioAlertNotification {
    pub config_id: String,
    pub scope: PortfolioAlertScope,
    pub breach: PortfolioAlertBreach,
    pub message: String,
    pub triggered_at: String,
}

pub struct PortfolioAlertEvaluation {
    pub status: PortfolioAlertDataStatus,
    pub snapshot: Option<PortfolioAlertSnapshot>,
    pub stale: bool,
    pub missing_data: Vec<MissingPortfolioAlertData>,
    pub active_breaches: Vec<PortfolioAlertBreach>,
    pub newly_triggered: Vec<PortfolioAlertBreach>,
}

pub struct PortfolioAlertView {
    pub config: Option<PortfolioAlertConfig>,
    pub evaluation: Option<PortfolioAlertEvaluation>,
}
```

Add `PortfolioAlertBreach` with the stable key, kind, non-null direction, and first/last timestamps. Derive the numeric category or concentration detail from `last_snapshot_json`; do not duplicate that detail in the breach table.

Add a serialization contract test so Tauri and TypeScript receive the agreed casing:

```rust
#[test]
fn portfolio_alert_contract_serializes_camel_case_fields_and_uppercase_enums() {
    let scope = PortfolioAlertScope {
        kind: PortfolioAlertScopeKind::Market,
        market: Some("US".to_string()),
        account_id: None,
    };
    assert_eq!(
        serde_json::to_value(scope).unwrap(),
        json!({ "kind": "MARKET", "market": "US", "accountId": null })
    );
    assert_eq!(
        serde_json::to_value(PortfolioAlertBreachDirection::AboveLimit).unwrap(),
        json!("ABOVE_LIMIT")
    );
}
```

- [ ] **Step 5: Run migration and model tests**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml db::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml commands::reset::tests -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml models::portfolio_alert -- --nocapture
```

Expected: all pass.

- [ ] **Step 6: Commit the schema boundary**

```bash
git add src-tauri/src/db/schema.rs src-tauri/src/db/migrations.rs src-tauri/src/db/tests.rs src-tauri/src/commands/reset.rs src-tauri/src/models/mod.rs src-tauri/src/models/portfolio_alert.rs
git commit -m "feat: add portfolio alert schema"
```

---

### Task 2: Implement deterministic allocation and concentration calculations

**Files:**
- Create: `src-tauri/src/services/portfolio_alert_calculator.rs`
- Modify: `src-tauri/src/services/mod.rs`

- [ ] **Step 1: Define calculator-only input types and write failing tests**

Keep persistence and I/O out of this module:

```rust
pub struct PortfolioAlertCategoryInput {
    pub id: String,
    pub name: String,
    pub color: String,
    pub icon: String,
    pub sort_order: i64,
}

pub struct PortfolioAlertPositionInput {
    pub account_id: String,
    pub market: String,
    pub symbol: String,
    pub name: String,
    pub category_id: Option<String>,
    pub category_name: String,
    pub category_color: String,
    pub market_value: f64,
    pub is_cash: bool,
}

pub enum PortfolioAlertCalculation {
    Ready(PortfolioAlertSnapshot),
    Empty,
}

pub fn calculate_portfolio_alert_snapshot(
    config: &PortfolioAlertConfig,
    categories: &[PortfolioAlertCategoryInput],
    positions: &[PortfolioAlertPositionInput],
    base_currency: &str,
    evaluated_at: &str,
) -> Result<PortfolioAlertCalculation, String>;
```

Add exact tests for the strict threshold, zero target, cash behavior, uncategorized holdings, and cross-account aggregation:

```rust
#[test]
fn allocation_uses_relative_deviation_and_strict_greater_than() {
    let config = config_with_targets(20.0, [("growth", 50.0), ("cash", 50.0)]);
    let at_boundary = positions([("growth", 60.0, false), ("cash", 40.0, true)]);
    let beyond = positions([("growth", 60.01, false), ("cash", 39.99, true)]);

    let boundary = calculate(&config, &at_boundary);
    assert_eq!(allocation(&boundary, "growth").direction, None);

    let breached = calculate(&config, &beyond);
    assert_eq!(allocation(&breached, "growth").direction, Some(AllocationDirection::Overweight));
}

#[test]
fn positive_value_against_zero_target_is_overweight() {
    let config = config_with_targets(20.0, [("growth", 100.0)]);
    let snapshot = calculate(&config, &uncategorized_position(1.0));
    let row = uncategorized_allocation(&snapshot);

    assert_eq!(row.target_percent, 0.0);
    assert_eq!(row.relative_deviation_percent, None);
    assert_eq!(row.direction, Some(AllocationDirection::Overweight));
}

#[test]
fn cash_affects_allocation_but_is_excluded_from_concentration() {
    let config = config_with_concentration(20.0);
    let snapshot = calculate(&config, &positions([
        ("cash", 60.0, true),
        ("growth", 40.0, false),
    ]));

    assert_eq!(allocation(&snapshot, "cash").current_percent, 60.0);
    assert_eq!(snapshot.concentrations.len(), 1);
    assert_eq!(snapshot.concentrations[0].position_percent, 40.0);
}

#[test]
fn concentration_aggregates_same_market_and_symbol_across_accounts() {
    let config = config_with_concentration(20.0);
    let snapshot = calculate(&config, &same_symbol_in_two_accounts("US", "AAPL", 12.0, 13.0, 100.0));

    assert_eq!(snapshot.concentrations[0].symbol, "AAPL");
    assert_eq!(snapshot.concentrations[0].market_value, 25.0);
    assert_eq!(snapshot.concentrations[0].position_percent, 25.0);
}
```

Also assert:

- empty positions or a non-positive total return `PortfolioAlertCalculation::Empty`;
- a zero-target category with zero current value is normal;
- a positive target with zero current value has 100% relative deviation and becomes underweight only when the configured threshold is below 100%;
- negative or non-finite values are rejected;
- `rebalance_amount = target_percent / 100 * total_market_value - current_market_value`;
- concentration uses a strict `>` comparison and excludes exact-threshold positions;
- categories deleted from Settings do not reappear except the virtual uncategorized row.

- [ ] **Step 2: Run the focused calculator tests and confirm failure**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml portfolio_alert_calculator -- --nocapture
```

Expected: compile failure because the calculator module does not exist.

- [ ] **Step 3: Implement the smallest pure calculator**

Filter positions by `config.scope`, sum total and category values, aggregate non-cash concentration by `(market, normalized_symbol)`, apply the special zero-target rule, and sort category output by the Settings category order followed by “未分类”. Do not round before comparisons; round only for display in React.

- [ ] **Step 4: Run focused tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml portfolio_alert_calculator -- --nocapture
```

Expected: all calculator tests pass.

- [ ] **Step 5: Commit the calculation core**

```bash
git add src-tauri/src/services/mod.rs src-tauri/src/services/portfolio_alert_calculator.rs
git commit -m "feat: calculate portfolio allocation alerts"
```

---

### Task 3: Persist and validate independent scope configurations

**Files:**
- Create: `src-tauri/src/services/portfolio_alert_service.rs`
- Modify: `src-tauri/src/services/mod.rs`

- [ ] **Step 1: Write failing service tests for scope identity and atomic saves**

Place database-backed tests under `portfolio_alert_service::tests`:

```rust
#[test]
fn save_config_replaces_targets_atomically_and_preserves_scope_identity() {
    let db = configured_db();
    seed_categories(&db, ["growth", "bonds"]);
    let first = save_portfolio_alert_config(
        &db,
        input(overall_scope(), 20.0, 20.0, [("growth", 60.0), ("bonds", 40.0)]),
    ).unwrap();

    let updated = save_portfolio_alert_config(
        &db,
        input_with_id(first.id.clone(), overall_scope(), 15.0, 25.0, [("growth", 50.0), ("bonds", 50.0)]),
    ).unwrap();

    assert_eq!(updated.id, first.id);
    assert_eq!(updated.targets, targets([("growth", 50.0), ("bonds", 50.0)]));
    assert_eq!(config_count(&db), 1);
}

#[test]
fn save_config_rejects_invalid_totals_and_mismatched_ids() {
    let db = configured_db();
    seed_categories(&db, ["growth", "bonds"]);

    assert!(save_portfolio_alert_config(
        &db,
        input(overall_scope(), 20.0, 20.0, [("growth", 70.0), ("bonds", 20.0)]),
    ).unwrap_err().contains("100"));

    let saved = save_portfolio_alert_config(&db, valid_market_input("US")).unwrap();
    let error = save_portfolio_alert_config(
        &db,
        input_with_id(saved.id, market_scope("CN"), 20.0, 20.0, valid_targets()),
    ).unwrap_err();
    assert!(error.contains("scope"));
}

#[test]
fn overall_market_and_each_account_keep_independent_configs() {
    let db = configured_db();
    seed_account(&db, "acct-us", "US");
    let configs = [
        save(&db, overall_scope()),
        save(&db, market_scope("US")),
        save(&db, account_scope("acct-us")),
    ];

    assert_eq!(configs.iter().map(|c| &c.id).collect::<HashSet<_>>().len(), 3);
}
```

Add validation coverage for duplicate categories, unknown categories, non-finite thresholds, deviation outside `0..=100`, concentration outside `0 < value <= 100`, base currency outside `USD|CNY|HKD`, market values outside `CN|US|HK`, account scopes with unknown accounts, account deletion cascading its configuration/targets/breaches, and total tolerance (`99.99` and `100.01` accepted; values outside rejected). Validate market/account base currency against `CN -> CNY`, `US -> USD`, and `HK -> HKD`; overall accepts the saved Settings base currency.

- [ ] **Step 2: Run the service tests and confirm failure**

```bash
cargo test --manifest-path src-tauri/Cargo.toml portfolio_alert_service::tests -- --nocapture
```

Expected: compile failure because persistence functions are missing.

- [ ] **Step 3: Implement validation and transactional persistence**

Expose these exact service functions:

```rust
pub fn scope_key(scope: &PortfolioAlertScope) -> Result<String, String>;

pub fn get_portfolio_alert_config_by_scope(
    db: &Database,
    scope: &PortfolioAlertScope,
) -> Result<Option<PortfolioAlertConfig>, String>;

pub fn get_portfolio_alert_config_by_id(
    db: &Database,
    config_id: &str,
) -> Result<PortfolioAlertConfig, String>;

pub fn save_portfolio_alert_config(
    db: &Database,
    input: SavePortfolioAlertConfigInput,
) -> Result<PortfolioAlertConfig, String>;

pub fn set_portfolio_alert_active(
    db: &Database,
    config_id: &str,
    is_active: bool,
) -> Result<PortfolioAlertConfig, String>;
```

Use one SQLite transaction to upsert the configuration, delete the old target rows, insert the validated replacement targets, and commit. Resolve account existence and Settings category existence inside that transaction. Do not mutate an existing configuration's scope.

- [ ] **Step 4: Clear breach state inside configuration mutations**

When an existing configuration is saved, delete all of its active breach rows inside the same transaction before committing the new targets. When disabling a configuration, set `is_active = 0` and delete its active breach rows in one transaction. Enabling only changes the flag here; Task 4's command layer performs the immediate current-data evaluation after the mutation commits.

- [ ] **Step 5: Run service tests and a backend compile check**

```bash
cargo test --manifest-path src-tauri/Cargo.toml portfolio_alert_service::tests -- --nocapture
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: both pass.

- [ ] **Step 6: Commit configuration persistence**

```bash
git add src-tauri/src/services/mod.rs src-tauri/src/services/portfolio_alert_service.rs
git commit -m "feat: persist portfolio alert configurations"
```

---

### Task 4: Evaluate cached portfolio data and persist recoverable breach transitions

**Files:**
- Modify: `src-tauri/src/services/portfolio_read_service.rs`
- Modify: `src-tauri/src/services/portfolio_alert_service.rs`
- Create: `src-tauri/src/commands/portfolio_alerts.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write failing cache-completeness and transition tests**

Add these service tests with a temporary database and in-memory quote cache:

```rust
#[tokio::test]
async fn ready_evaluation_persists_snapshot_and_notifies_only_on_new_transition() {
    let fixture = alert_fixture_with_complete_quotes();
    let first = evaluate_portfolio_alert(
        &fixture.db,
        &fixture.quote_cache,
        Some(&fixture.rates),
        &fixture.config_id,
        "2026-09-06T10:00:00Z",
    ).await.unwrap();
    assert_eq!(first.status, PortfolioAlertDataStatus::Ready);
    assert_eq!(first.newly_triggered.len(), 1);
    assert_eq!(active_breach_count(&fixture.db), 1);

    let second = evaluate_portfolio_alert(
        &fixture.db,
        &fixture.quote_cache,
        Some(&fixture.rates),
        &fixture.config_id,
        "2026-09-06T10:05:00Z",
    ).await.unwrap();
    assert!(second.newly_triggered.is_empty());
    assert_eq!(active_breach_count(&fixture.db), 1);
}

#[tokio::test]
async fn recovery_removes_active_row_and_later_breach_notifies_again() {
    let fixture = breached_fixture();
    evaluate(&fixture).await;
    fixture.quote_cache.set_price("US", "AAPL", 50.0);
    let recovered = evaluate(&fixture).await;
    assert!(recovered.active_breaches.is_empty());
    assert_eq!(active_breach_count(&fixture.db), 0);

    fixture.quote_cache.set_price("US", "AAPL", 100.0);
    let rebreach = evaluate(&fixture).await;
    assert_eq!(rebreach.newly_triggered.len(), 1);
}

#[tokio::test]
async fn incomplete_quotes_keep_last_snapshot_and_do_not_change_breaches() {
    let fixture = evaluated_fixture();
    let prior = load_last_snapshot(&fixture.db);
    fixture.quote_cache.remove("US", "AAPL");

    let result = evaluate(&fixture).await;

    assert_eq!(result.status, PortfolioAlertDataStatus::Incomplete);
    assert!(result.stale);
    assert_eq!(result.snapshot, Some(prior));
    assert_eq!(active_breach_keys(&fixture.db), fixture.prior_breach_keys);
    assert!(result.newly_triggered.is_empty());
}
```

Add cases for missing FX in an overall scope, no holdings or non-positive total value (`EMPTY`), invalid config after category deletion (`INVALID_CONFIG`), overall/market/account filtering, cash at one unit of its native currency, and same-symbol aggregation. Market and account scopes whose holdings are already in the scope's native currency must stay `READY` without an FX cache.

Add state-machine cases proving that saving a changed config clears old breaches before immediate evaluation, disabling clears breaches, re-enabling evaluates current data, and an injected breach-write failure rolls back the new snapshot, `last_evaluated_at`, inserts, updates, and deletes together.

- [ ] **Step 2: Run the focused tests and confirm failure**

```bash
cargo test --manifest-path src-tauri/Cargo.toml portfolio_alert_service::tests::ready_evaluation -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml portfolio_alert_service::tests::incomplete_quotes -- --nocapture
```

Expected: compile failure because evaluation is not implemented.

- [ ] **Step 3: Expose cache completeness from the read model**

Extend `PortfolioReadModel` without changing existing dashboard semantics:

```rust
pub struct PortfolioReadModel {
    holdings: Vec<HoldingDetail>,
    missing_quote_keys: HashSet<(String, String)>,
    category_ids_by_holding: HashMap<String, Option<String>>,
    quote_warning: Option<String>,
    quotes_refreshed: bool,
}

impl PortfolioReadModel {
    pub fn missing_quote_keys(&self) -> &HashSet<(String, String)>;
    pub fn category_id_for_holding(&self, holding_id: &str) -> Option<&str>;
}
```

Build quote maps by `(market, symbol)`, not symbol alone, and add a focused regression for the same symbol text in two markets resolving to different cached prices. Preserve the read model's existing dashboard behavior. When the alert service builds calculator inputs, treat `$CASH-*` shares as native-currency cash value at price 1, exclude them from `missing_quote_keys`, and exclude them from concentration.

- [ ] **Step 4: Implement scope loading and evaluation**

Expose:

```rust
pub async fn evaluate_portfolio_alert(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    config_id: &str,
    evaluated_at: &str,
) -> Result<PortfolioAlertEvaluation, String>;
```

Implementation order:

1. Load the configuration, current Settings categories, and `PortfolioReadModel` with `QuoteReadMode::CacheOnly`.
2. Filter holdings to the scope before checking completeness and populate `normalized_symbol` with the existing stock-symbol normalization helper.
3. Derive market/account currency as `CN -> CNY`, `US -> USD`, and `HK -> HKD`; use the persisted Settings base currency for overall. Convert only values whose holding currency differs from that base.
4. Return `EMPTY` for no scoped holdings.
5. Return `INCOMPLETE` with the prior snapshot and unchanged breach rows if any scoped non-cash quote is missing or any required FX rate is missing, non-finite, or non-positive.
6. Return `INVALID_CONFIG` with unchanged breach rows if surviving targets no longer total within the allowed tolerance.
7. Calculate a current snapshot and proposed breach map.
8. In one SQLite transaction, insert newly breached keys, update `last_seen_at` and current direction for persistent keys, delete recovered keys, and save `last_snapshot_json` plus `last_evaluated_at`.
9. Return current active breaches and only the newly inserted rows in `newly_triggered`.

- [ ] **Step 5: Add mutation-aware Tauri commands and register them**

```rust
#[tauri::command]
pub async fn get_portfolio_alert_view(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    exchange_rate_cache: State<'_, ExchangeRateCache>,
    scope: PortfolioAlertScope,
) -> Result<PortfolioAlertView, String>;

#[tauri::command]
pub async fn save_portfolio_alert_config(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    exchange_rate_cache: State<'_, ExchangeRateCache>,
    input: SavePortfolioAlertConfigInput,
) -> Result<PortfolioAlertView, String>;

#[tauri::command]
pub async fn set_portfolio_alert_active(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    exchange_rate_cache: State<'_, ExchangeRateCache>,
    config_id: String,
    is_active: bool,
) -> Result<PortfolioAlertView, String>;

#[tauri::command]
pub async fn evaluate_portfolio_alert(
    db: State<'_, Database>,
    quote_cache: State<'_, QuoteCache>,
    exchange_rate_cache: State<'_, ExchangeRateCache>,
    config_id: String,
) -> Result<PortfolioAlertEvaluation, String>;
```

Use `ExchangeRateCache::get_stale()` and then `load_exchange_rates_from_db`; do not call the network-fetching `get_cached_rates`. `get_portfolio_alert_view` returns an unconfigured view or evaluates an active config when the tab opens. Save clears old breach rows in its configuration transaction and immediately evaluates the committed config. Disable clears active breaches and returns without evaluation; enable immediately evaluates. Register all four commands in `commands/mod.rs` and `generate_handler!` without modifying the existing price-alert commands.

- [ ] **Step 6: Run focused and regression tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml portfolio_alert -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml portfolio_read_service -- --nocapture
```

Expected: all pass.

- [ ] **Step 7: Commit evaluation and transition logic**

```bash
git add src-tauri/src/services/portfolio_read_service.rs src-tauri/src/services/portfolio_alert_service.rs src-tauri/src/commands/mod.rs src-tauri/src/commands/portfolio_alerts.rs src-tauri/src/lib.rs
git commit -m "feat: evaluate recoverable portfolio alerts"
```

---

### Task 5: Evaluate active configurations after quote refresh and emit new-breach events

**Files:**
- Modify: `src-tauri/src/services/portfolio_alert_service.rs`
- Modify: `src-tauri/src/commands/portfolio_alerts.rs`
- Modify: `src-tauri/src/commands/quotes.rs`
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Write failing batch-evaluation tests**

```rust
#[tokio::test]
async fn evaluate_all_active_skips_disabled_configs_and_collects_only_new_breaches() {
    let fixture = multi_config_fixture();
    fixture.disable("config-hk");

    let notifications = evaluate_all_active_portfolio_alerts(
        &fixture.db,
        &fixture.quote_cache,
        Some(&fixture.rates),
        "2026-09-06T10:00:00Z",
    ).await.unwrap();

    assert_eq!(notifications.iter().map(|n| n.config_id.as_str()).collect::<Vec<_>>(), vec!["config-us"]);
    assert!(!was_evaluated(&fixture.db, "config-hk"));
}
```

Also assert that one incomplete configuration does not prevent other active configurations from evaluating and that a second unchanged batch returns no notifications.

- [ ] **Step 2: Run the test and confirm failure**

```bash
cargo test --manifest-path src-tauri/Cargo.toml evaluate_all_active -- --nocapture
```

Expected: compile failure because the batch function does not exist.

- [ ] **Step 3: Implement the batch function**

```rust
pub async fn evaluate_all_active_portfolio_alerts(
    db: &Database,
    quote_cache: &QuoteCache,
    exchange_rates: Option<&ExchangeRates>,
    evaluated_at: &str,
) -> Result<Vec<PortfolioAlertNotification>, String>;
```

Load active IDs once, evaluate them sequentially against the same quote/rate snapshot, log per-config errors, and continue. Return one notification payload per newly inserted breach with config ID, scope, breach key, kind, concise Chinese message, and timestamp. Missing rates affect only scopes requiring currency conversion.

- [ ] **Step 4: Wire the existing refresh pipeline**

Add `pub(crate) async fn evaluate_and_emit_portfolio_alerts(...)` in `commands/portfolio_alerts.rs`. It reads only stale in-memory or persisted rates, calls the batch function, logs errors, and emits `portfolio-alert-triggered` once per returned item. Call it in both places that complete a holding-quote sync: the public `get_holding_quotes` command after a successful non-empty/full refresh request (not a `Some(vec![])` cache-only read) and the startup/background quote-refresh task after quotes have been persisted.

```rust
let rates = rate_cache
    .get_stale()
    .or_else(|| exchange_rate_service::load_exchange_rates_from_db(&db).ok().flatten());
let notifications = portfolio_alert_service::evaluate_all_active_portfolio_alerts(
    &db,
    &quote_cache,
    rates.as_ref(),
    &Utc::now().to_rfc3339(),
).await;

if let Ok(items) = notifications {
    for item in items {
        let _ = app_handle.emit("portfolio-alert-triggered", item);
    }
}
```

Keep the existing `quotes-refreshed` event. Evaluation errors are logged and must not fail the quote refresh. Add the required `AppHandle` and `ExchangeRateCache` state parameters to the public command only; keep `get_holding_quotes_inner` reusable for callers and tests.

- [ ] **Step 5: Run backend tests and compile**

```bash
cargo test --manifest-path src-tauri/Cargo.toml evaluate_all_active -- --nocapture
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: both pass.

- [ ] **Step 6: Commit refresh integration**

```bash
git add src-tauri/src/services/portfolio_alert_service.rs src-tauri/src/commands/portfolio_alerts.rs src-tauri/src/commands/quotes.rs src-tauri/src/lib.rs
git commit -m "feat: trigger portfolio alerts after quote refresh"
```

---

### Task 6: Create the “投资提醒” page shell and preserve price-alert behavior

**Files:**
- Create: `src/pages/Alerts/alertsCopy.ts`
- Create: `src/pages/Alerts/alertsCopy.test.ts`
- Create: `src/pages/Alerts/PriceAlertsTab.tsx`
- Modify: `src/pages/Alerts/index.tsx`
- Modify: `src/components/Layout/MainLayout.tsx`

- [ ] **Step 1: Add a failing copy contract test**

```typescript
import assert from 'node:assert/strict';
import test from 'node:test';
import { ALERTS_MENU_LABEL, INVESTMENT_ALERT_TABS } from './alertsCopy.ts';

test('investment alerts exposes portfolio and price tabs in product order', () => {
  assert.equal(ALERTS_MENU_LABEL, '投资提醒');
  assert.deepEqual(INVESTMENT_ALERT_TABS, [
    { key: 'portfolio', label: '组合提醒' },
    { key: 'price', label: '价格提醒' },
  ]);
});
```

- [ ] **Step 2: Run the test and confirm failure**

```bash
node --test src/pages/Alerts/alertsCopy.test.ts
```

Expected: module-not-found failure.

- [ ] **Step 3: Extract the current page into `PriceAlertsTab`**

Move the existing price-alert list, create/edit form, delete behavior, holding selector, loading/error handling, and store usage without changing its command names or data shapes. Export a default `PriceAlertsTab` component. No existing price-alert functionality is removed.

- [ ] **Step 4: Build the tab shell from the tested constants**

`Alerts/index.tsx` should render a page title “投资提醒” and Ant Design tabs. Default to `portfolio`; lazily render `PortfolioAlertsTab` after Task 8, and use a small typed placeholder component until then so this commit builds. Change only the existing menu item's label in `MainLayout.tsx`; preserve its icon, path `/alerts`, permissions, and selection logic.

- [ ] **Step 5: Run copy, price-alert, and build regressions**

```bash
node --test src/pages/Alerts/alertsCopy.test.ts
node --test src/pages/Alerts/holdingOptions.test.ts
bun run build
```

Expected: all pass and `/alerts` still builds with the current price-alert UI under its tab.

- [ ] **Step 6: Commit the page shell**

```bash
git add src/pages/Alerts/alertsCopy.ts src/pages/Alerts/alertsCopy.test.ts src/pages/Alerts/PriceAlertsTab.tsx src/pages/Alerts/index.tsx src/components/Layout/MainLayout.tsx
git commit -m "feat: add investment alerts tab shell"
```

---

### Task 7: Add the typed frontend store with request isolation

**Files:**
- Create: `src/types/portfolioAlert.ts`
- Modify: `src/types/index.ts`
- Create: `src/stores/portfolioAlertStore.ts`
- Create: `src/stores/portfolioAlertStore.test.ts`

- [ ] **Step 1: Mirror the backend contract in TypeScript**

Use literal unions rather than untyped strings:

```typescript
export type PortfolioAlertScope =
  | { kind: 'OVERALL'; market: null; accountId: null }
  | { kind: 'MARKET'; market: 'CN' | 'US' | 'HK'; accountId: null }
  | { kind: 'ACCOUNT'; market: null; accountId: string };

export type PortfolioAlertDataStatus = 'READY' | 'EMPTY' | 'INCOMPLETE' | 'INVALID_CONFIG';

export interface SavePortfolioAlertConfigInput {
  id: string | null;
  scope: PortfolioAlertScope;
  baseCurrency: 'USD' | 'CNY' | 'HKD';
  deviationThreshold: number;
  concentrationThreshold: number;
  isActive: boolean;
  targets: Array<{ categoryId: string; targetPercent: number }>;
}
```

Mirror every canonical Rust response field, including stale status, missing-data reasons, category rows, concentration rows, active breaches, and newly triggered breaches.

- [ ] **Step 2: Write failing store tests**

Inject the Tauri invoker so Node tests do not need a desktop runtime:

```typescript
test('scope key separates overall, market, and account state', () => {
  assert.equal(portfolioAlertScopeKey(overallScope()), 'overall');
  assert.equal(portfolioAlertScopeKey(marketScope('US')), 'market:US');
  assert.equal(portfolioAlertScopeKey(accountScope('acct-1')), 'account:acct-1');
});

test('a stale scope response cannot overwrite the newest request', async () => {
  const us = deferred<PortfolioAlertView>();
  const cn = deferred<PortfolioAlertView>();
  const store = createPortfolioAlertStore(commandRouter({ US: us.promise, CN: cn.promise }));

  const first = store.getState().loadScope(marketScope('US'));
  const second = store.getState().loadScope(marketScope('CN'));
  cn.resolve(viewFor('CN'));
  await second;
  us.resolve(viewFor('US'));
  await first;

  assert.equal(store.getState().selectedScopeKey, 'market:CN');
  assert.equal(selectCurrentPortfolioAlertView(store.getState())?.config?.scope.market, 'CN');
});

test('save uses the mutation command response without a second evaluation call', async () => {
  const calls: InvokeCall[] = [];
  const store = createPortfolioAlertStore(recordingInvoker(calls));
  await store.getState().saveConfig(validDraft());

  assert.deepEqual(calls.map((call) => call.command), ['save_portfolio_alert_config']);
});

test('newly triggered breaches are queued once for the UI to consume', async () => {
  const store = createPortfolioAlertStore(invokerReturning(readyViewWithNewBreach('category:growth')));
  await store.getState().loadScope(overallScope());
  assert.deepEqual(store.getState().pendingNotifications.map((item) => item.breachKey), ['category:growth']);

  const consumed = store.getState().takePendingNotifications();
  assert.equal(consumed.length, 1);
  assert.deepEqual(store.getState().pendingNotifications, []);
});
```

Also cover activation toggles and independent per-scope loading/error maps.

- [ ] **Step 3: Run focused tests and confirm failure**

```bash
node --test src/stores/portfolioAlertStore.test.ts
```

Expected: module-not-found failure.

- [ ] **Step 4: Implement the vanilla store and React hook**

Use Zustand's vanilla store for tests and export a bound React hook for components:

```typescript
export interface PortfolioAlertStoreState {
  selectedScopeKey: string;
  viewsByScope: Record<string, PortfolioAlertView | undefined>;
  loadingByScope: Record<string, boolean>;
  errorsByScope: Record<string, string | undefined>;
  pendingNotifications: PortfolioAlertBreach[];
  selectScope(scope: PortfolioAlertScope): void;
  loadScope(scope: PortfolioAlertScope): Promise<void>;
  saveConfig(input: SavePortfolioAlertConfigInput): Promise<void>;
  setActive(configId: string, scope: PortfolioAlertScope, isActive: boolean): Promise<void>;
  evaluate(configId: string, scope: PortfolioAlertScope): Promise<void>;
  takePendingNotifications(): PortfolioAlertBreach[];
}
```

Use a monotonically increasing request revision per scope. Export `selectCurrentPortfolioAlertView(state)` to derive the visible view from `selectedScopeKey`; no response may change the selected scope. `loadScope` calls `get_portfolio_alert_view`; save and activation commands already return the post-mutation view and must not trigger a redundant second evaluation. Enqueue each command response's `newlyTriggered` rows once, keyed by `configId + breachKey + firstTriggeredAt`, and clear them atomically through `takePendingNotifications`.

- [ ] **Step 5: Run store tests and TypeScript build**

```bash
node --test src/stores/portfolioAlertStore.test.ts
bun run build
```

Expected: both pass.

- [ ] **Step 6: Commit the frontend data layer**

```bash
git add src/types/portfolioAlert.ts src/types/index.ts src/stores/portfolioAlertStore.ts src/stores/portfolioAlertStore.test.ts
git commit -m "feat: add portfolio alert frontend store"
```

---

### Task 8: Build the portfolio-alert configuration, chart, and breach UI

**Files:**
- Create: `src/pages/Alerts/portfolioAlertViewModel.ts`
- Create: `src/pages/Alerts/portfolioAlertViewModel.test.ts`
- Create: `src/pages/Alerts/PortfolioAlertsTab.tsx`
- Modify: `src/pages/Alerts/index.tsx`
- Modify: `src/stores/portfolioAlertStore.ts`

- [ ] **Step 1: Write failing pure view-model tests**

```typescript
test('scope options contain overall, three markets, then every account', () => {
  const options = buildPortfolioAlertScopeOptions([
    { id: 'acct-us', name: '美股主账户', market: 'US' },
    { id: 'acct-hk', name: '港股账户', market: 'HK' },
  ]);

  assert.deepEqual(options.map((item) => item.label), [
    '整体组合', 'A股组合', '美股组合', '港股组合', '美股主账户', '港股账户',
  ]);
});

test('a deleted account selection falls back to overall', () => {
  const options = buildPortfolioAlertScopeOptions([]);
  assert.deepEqual(
    resolvePortfolioAlertScope(accountScope('deleted-account'), options),
    overallScope(),
  );
});

test('draft validation requires target total within one basis point of 100', () => {
  assert.equal(validatePortfolioAlertDraft(draft([60, 39.98])).valid, false);
  assert.equal(validatePortfolioAlertDraft(draft([60, 39.99])).valid, true);
  assert.equal(validatePortfolioAlertDraft(draft([60, 40.01])).valid, true);
  assert.equal(validatePortfolioAlertDraft(draft([60, 40.02])).valid, false);
});

test('chart rows include current and target data plus stale status', () => {
  const model = buildPortfolioAlertDisplayModel(incompleteViewWithLastSnapshot());
  assert.equal(model.stale, true);
  assert.equal(model.statusLabel, '数据不完整');
  assert.deepEqual(model.pieData.map((row) => row.name), ['成长', '现金', '未分类']);
  assert.equal(model.canAskAi, false);
});

test('AI is enabled only for a ready evaluation with active breaches', () => {
  assert.equal(buildPortfolioAlertDisplayModel(readyNormalView()).canAskAi, false);
  assert.equal(buildPortfolioAlertDisplayModel(readyBreachedView()).canAskAi, true);
});
```

Add display cases for empty state, invalid configuration, exact-threshold normal rows, overweight/underweight labels, and concentration rows.

- [ ] **Step 2: Run the view-model tests and confirm failure**

```bash
node --test src/pages/Alerts/portfolioAlertViewModel.test.ts
```

Expected: module-not-found failure.

- [ ] **Step 3: Implement pure scope, validation, and display helpers**

Expose:

```typescript
export function buildPortfolioAlertScopeOptions(accounts: Account[]): ScopeOption[];
export function resolvePortfolioAlertScope(scope: PortfolioAlertScope, options: ScopeOption[]): PortfolioAlertScope;
export function validatePortfolioAlertDraft(draft: PortfolioAlertDraft): DraftValidation;
export function buildPortfolioAlertDisplayModel(view?: PortfolioAlertView): PortfolioAlertDisplayModel;
```

The display model supplies unrounded numeric chart values, formatted labels, row status colors, total target/current values, stale banner copy, missing-data descriptions, concentration warnings, and `canAskAi`.

- [ ] **Step 4: Build `PortfolioAlertsTab`**

Implement the approved layout:

- scope selector for overall, A-share, US, HK, and every account;
- active switch plus edit/save action;
- category target editor sourced from the existing category store/API;
- one shared deviation threshold and one concentration threshold, both defaulting to 20;
- visible target total with save disabled until validation succeeds;
- current allocation pie chart using the existing ECharts registration pattern;
- target/current/deviation/rebalance-amount table;
- concentration alert list;
- `EMPTY`, `INCOMPLETE`, `INVALID_CONFIG`, and stale-snapshot states;
- disabled AI button with an explanatory tooltip unless `canAskAi` is true.

Merge every freshly loaded Settings category into the draft and display model with a default target of 0%; never expose “未分类” as an editable target. Show category icon, color, and Settings sort order. Mark changed targets or thresholds as unsaved and confirm before discarding them during a scope switch.

For an overall draft, copy `baseCurrency` from `useExchangeRateStore`; for market and account drafts derive the native currency from the selected market. Include it in the save payload and display it beside all amount columns.

Observe `useQuoteStore`'s `lastUpdatedAt` and call `evaluate` for the visible active configuration whenever a newer quote snapshot arrives, regardless of whether that snapshot came from the startup event or a manual refresh command. Subscribe to `portfolio-alert-triggered`, show one Ant Design notification for the emitted new breach, and refresh only the matching visible scope. Remove the Tauri event listener on unmount.

Consume `pendingNotifications` after load/save/enable/manual evaluation and show the same notification presentation for command-returned transitions. Use `configId + breachKey + firstTriggeredAt` as the UI dedupe key so React Strict Mode and an adjacent refresh event cannot display the same transition twice.

- [ ] **Step 5: Replace the shell placeholder and verify UI compilation**

Import `PortfolioAlertsTab` into `Alerts/index.tsx`; keep `portfolio` as the default tab and leave `PriceAlertsTab` unchanged.

Run:

```bash
node --test src/pages/Alerts/portfolioAlertViewModel.test.ts
node --test src/stores/portfolioAlertStore.test.ts
bun run build
```

Expected: all pass.

- [ ] **Step 6: Commit the portfolio-alert UI**

```bash
git add src/pages/Alerts/portfolioAlertViewModel.ts src/pages/Alerts/portfolioAlertViewModel.test.ts src/pages/Alerts/PortfolioAlertsTab.tsx src/pages/Alerts/index.tsx src/stores/portfolioAlertStore.ts
git commit -m "feat: add portfolio allocation alert workspace"
```

---

### Task 9: Add trusted AI rebalancing context and a built-in rebalancing skill

**Files:**
- Modify: `src-tauri/src/services/ai_tools.rs`
- Modify: `src-tauri/src/services/ai_chat_service.rs`
- Modify: `src-tauri/src/services/skill_service.rs`
- Create: `src-tauri/src/skills/portfolio-rebalance.md`
- Modify: `src-tauri/src/commands/ai.rs`
- Modify: `src-tauri/src/services/portfolio_alert_service.rs`

- [ ] **Step 1: Write failing trusted-prefill tests**

```rust
#[test]
fn rebalance_prefill_accepts_only_an_exact_config_id_payload() {
    let valid = PrefilledToolContext {
        name: "get_rebalance_context".to_string(),
        arguments: json!({ "config_id": "config-us" }),
    };
    assert!(validated_prefilled_tool(&valid).is_ok());

    for invalid in [
        json!({ "name": "get_rebalance_context", "arguments": {} }),
        json!({ "name": "get_rebalance_context", "arguments": { "config_id": "config-us", "market": "CN" } }),
        json!({ "name": "get_rebalance_context", "arguments": { "config_id": 1 } }),
    ] {
        let context: PrefilledToolContext = serde_json::from_value(invalid).unwrap();
        assert!(validated_prefilled_tool(&context).is_err());
    }
}

#[tokio::test]
async fn rebalance_context_is_resolved_from_the_saved_config_and_current_snapshot() {
    let fixture = ready_breached_fixture("config-us");
    let result = execute_get_rebalance_context(&fixture.context, "config-us").await.unwrap();

    assert_eq!(result["scope"]["market"], "US");
    assert_eq!(result["assumptions"]["additionalCapital"], 0);
    assert_eq!(result["activeBreaches"].as_array().unwrap().len(), 1);
    assert!(result["deterministicActions"][0]["amount"].as_f64().unwrap() != 0.0);
}
```

Also reject nonexistent configs, inactive configs, `INCOMPLETE`/`EMPTY`/`INVALID_CONFIG` evaluations, stale snapshots, and ready configs with no active breach.

- [ ] **Step 2: Write failing market-bound tool tests**

```rust
#[test]
fn account_rebalance_scope_carries_the_accounts_market() {
    let db = db_with_account_and_config("acct-us", "US", "config-acct-us");
    let scope = validated_portfolio_scope(&db, Some(&prefill("config-acct-us"))).unwrap().unwrap();
    assert_eq!(scope.account_id.as_deref(), Some("acct-us"));
    assert_eq!(scope.market.as_deref(), Some("US"));
}

#[tokio::test]
async fn restricted_rebalance_rejects_a_new_symbol_from_another_market() {
    let context = ai_context_with_scope(Some("US"), Some("acct-us"));
    let result = execute_stock_quote(&context, json!({ "symbol": "600519", "market": "CN" })).await;
    assert!(result.unwrap_err().contains("US"));
}
```

Add equivalent coverage for history, fundamentals, technical indicators, financials, and stock-search result filtering.

- [ ] **Step 3: Run the AI tests and confirm failure**

```bash
cargo test --manifest-path src-tauri/Cargo.toml prefilled_tool_tests::rebalance -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml restricted_rebalance -- --nocapture
```

Expected: failure because the new tool, derived scope, and built-in skill are absent.

- [ ] **Step 4: Implement `get_rebalance_context` as a trusted tool**

Add the tool definition and executor. The caller supplies only `config_id`. Extract a `preview_portfolio_alert` service function from the evaluator that uses current cache-only inputs but performs no database writes. The tool must run that preview, require `READY`, require at least one preview breach that is still present in the active-breach table, derive scope from the saved configuration, and return:

```json
{
  "configId": "config-us",
  "scope": { "kind": "MARKET", "market": "US", "accountId": null },
  "baseCurrency": "USD",
  "totalMarketValue": 100000,
  "thresholds": { "relativeDeviationPercent": 20, "concentrationPercent": 20 },
  "allocations": [],
  "positions": [],
  "activeBreaches": [],
  "deterministicActions": [],
  "assumptions": { "additionalCapital": 0, "automaticTrading": false }
}
```

`deterministicActions` uses each category's `rebalance_amount`; positive means buy and negative means sell. Include enough current-position detail for the model to select existing or candidate symbols, but never include credentials or provider configuration. Read rates only from `ExchangeRateCache::get_stale()` or SQLite so this trusted prefill cannot start a network refresh.

- [ ] **Step 5: Derive and enforce AI scope on the backend**

Change `validated_portfolio_scope` in `ai_chat_service.rs` to receive `&Database`, and update `commands/ai.rs` to pass it so `config_id` resolves to a trusted market/account pair before streaming begins. Add `get_rebalance_context` to `PortfolioScope::allows_tool` and the scope-filtered tool definitions. For account scopes, populate both the account ID and its market. Add one shared guard in `ai_tools.rs` used by quote, history, fundamentals, technical, and financial tools. Filter search results to the allowed market before returning them. Overall scope remains unrestricted; market and account scopes reject cross-market requests.

- [ ] **Step 6: Register the built-in `portfolio-rebalance` skill**

Use these instructions in the skill body:

```text
你是投资组合再平衡助手。先读取可信的 get_rebalance_context 结果，再给出以恢复目标配置为目的的建议。
默认不追加资金，不得建议自动下单。优先用卖出超配类别所得资金买入低配类别。
分别说明应买入/卖出的类别、候选标的、约金额、调整后类别占比和集中度影响。
可推荐当前未持有的标的，但必须标注“候选标的，交易前需核验”，并使用工具核验代码、市场、最新行情和基础资料。
候选标的必须注明建议归入的用户投资类别，并明确这是基于用户分类体系的假设。
所有新标的必须属于当前组合范围允许的市场；账户组合同时受该账户市场约束。
整体组合跨市场建议必须说明建议在哪个账户执行；需要跨账户转账或换汇时列为前置条件，不能假设资金可无成本移动。
只有集中度违规且类别正常时，优先在同一类别内替换超限标的，避免制造新的类别偏离。
若数据不完整、配置无效、没有有效违规或工具核验失败，明确说明并停止给出金额级交易建议。
按“再平衡结论、系统计算的类别缺口、具体标的和约计金额、调整后预计配置与集中度、执行顺序/风险/待核验事项”的顺序回答。
注明行情时间，并声明税费、佣金、滑点、汇率变化和最小交易单位未精确计入；建议是分析结果，不是订单。
```

Create the Markdown file with normal skill frontmatter (`name`, `description`, `trigger`, `enabled`) and register it in `BUILTIN_SKILLS` in `skill_service.rs`. Add parsing and trigger tests, including a check that generic “组合分析” does not accidentally activate the rebalancing skill.

- [ ] **Step 7: Run AI and backend regression tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml ai_chat -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml skill_service -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml portfolio_alert -- --nocapture
```

Expected: all pass.

- [ ] **Step 8: Commit the trusted AI backend**

```bash
git add src-tauri/src/services/ai_tools.rs src-tauri/src/services/ai_chat_service.rs src-tauri/src/services/skill_service.rs src-tauri/src/skills/portfolio-rebalance.md src-tauri/src/commands/ai.rs src-tauri/src/services/portfolio_alert_service.rs
git commit -m "feat: add trusted portfolio rebalancing context"
```

---

### Task 10: Navigate to a new AI session and auto-send the rebalancing request once

**Files:**
- Modify: `src/pages/AiAssistant/prefill.ts`
- Modify: `src/pages/AiAssistant/prefill.test.ts`
- Create: `src/pages/AiAssistant/portfolioRebalancePrefill.ts`
- Create: `src/pages/AiAssistant/portfolioRebalancePrefill.test.ts`
- Create: `src/pages/AiAssistant/aiPrefillAutoSend.ts`
- Create: `src/pages/AiAssistant/aiPrefillAutoSend.test.ts`
- Modify: `src/pages/AiAssistant/index.tsx`
- Modify: `src/pages/AiAssistant/ChatPanel.tsx`
- Modify: `src/pages/Alerts/PortfolioAlertsTab.tsx`

- [ ] **Step 1: Write failing route-state validation tests**

```typescript
test('portfolio rebalance prefill requires the trusted tool, skill, and auto-send', () => {
  const request = readAiPrefillRequest({
    prefillPrompt: '请根据当前违规生成再平衡建议。',
    prefillActiveSkill: 'portfolio-rebalance',
    prefillAutoSend: true,
    prefillToolName: 'get_rebalance_context',
    prefillToolArguments: { config_id: 'config-us' },
  });

  assert.deepEqual(request, {
    prompt: '请根据当前违规生成再平衡建议。',
    activeSkill: 'portfolio-rebalance',
    autoSend: true,
    toolContext: {
      name: 'get_rebalance_context',
      arguments: { config_id: 'config-us' },
    },
  });
});

test('auto-send is rejected for arbitrary skills and malformed tool arguments', () => {
  assert.equal(readAiPrefillRequest({
    prefillPrompt: 'send me',
    prefillActiveSkill: 'stock-review',
    prefillAutoSend: true,
  }), null);
  assert.equal(readAiPrefillRequest({
    prefillPrompt: 'send me',
    prefillActiveSkill: 'portfolio-rebalance',
    prefillAutoSend: true,
    prefillToolName: 'get_rebalance_context',
    prefillToolArguments: { market: 'US' },
  }), null);
});

test('rebalance navigation always clears the current session', () => {
  const action = buildPortfolioRebalanceNavigation('config-us');
  assert.equal(action.sessionId, null);
  assert.equal(action.state.prefillAutoSend, true);
  assert.equal(action.state.prefillToolName, 'get_rebalance_context');
  assert.equal(action.state.prefillToolArguments.config_id, 'config-us');
});

test('auto-send stages trusted context and creates one new session without using the old session', async () => {
  const calls: string[] = [];
  const request = validRebalanceRequest('config-us');

  await runAiPrefillAutoSend(request, {
    stageSkill: (skill) => calls.push(`skill:${skill}`),
    stageTool: (tool) => calls.push(`tool:${tool.name}`),
    createSession: async () => { calls.push('create'); return 'session-new'; },
    sendMessage: async (prompt, sessionId) => calls.push(`send:${sessionId}:${prompt}`),
    touchSession: async (sessionId) => calls.push(`touch:${sessionId}`),
    renameSession: async (sessionId) => calls.push(`rename:${sessionId}`),
  });

  assert.deepEqual(calls.slice(0, 4), [
    'skill:portfolio-rebalance',
    'tool:get_rebalance_context',
    'create',
    'send:session-new:请根据当前违规生成再平衡建议。',
  ]);
  assert.equal(calls.some((call) => call.includes('session-old')), false);
  assert.equal(calls.filter((call) => call === 'create').length, 1);
});
```

Keep existing stock-review and portfolio-overview prefills valid with `autoSend: false`.

- [ ] **Step 2: Run prefill tests and confirm failure**

```bash
node --test src/pages/AiAssistant/prefill.test.ts src/pages/AiAssistant/portfolioRebalancePrefill.test.ts src/pages/AiAssistant/aiPrefillAutoSend.test.ts
```

Expected: failure because rebalancing auto-send is not yet accepted.

- [ ] **Step 3: Parse one atomic prefill request**

Replace separately parsed route fields with:

```typescript
export interface AiPrefillRequest {
  prompt: string;
  activeSkill: string | null;
  autoSend: boolean;
  toolContext: AiToolContext | null;
}

export function readAiPrefillRequest(state: unknown): AiPrefillRequest | null;
```

Read the existing route keys `prefillPrompt`, `prefillActiveSkill`, `prefillAutoSend`, `prefillToolName`, and `prefillToolArguments`, then return the atomic internal shape above. Allow `autoSend: true` only when all of these are true: skill is `portfolio-rebalance`, tool name is `get_rebalance_context`, arguments contain exactly one non-empty string `config_id`, and prompt is non-empty. Preserve legacy helper exports and their false-auto-send behavior for the current stock-review and Munger portfolio entry points. Extend `readPersistedAiToolContext` to reconstruct `get_rebalance_context` with the rebalancing skill when a saved AI session is reopened.

- [ ] **Step 4: Add a one-shot send-decision helper and tests**

```typescript
export function shouldAutoSendPrefill(input: {
  request: AiPrefillRequest | null;
  consumed: boolean;
  configured: boolean;
  sending: boolean;
}): boolean {
  return Boolean(
    input.request?.autoSend &&
    !input.consumed &&
    input.configured &&
    !input.sending
  );
}
```

Test that missing configuration waits without consuming, rerenders while sending do not duplicate, and consumed requests never resend.

Implement `runAiPrefillAutoSend(request, dependencies)` in `aiPrefillAutoSend.ts` using injected staging, session, sending, touch, and rename functions. The helper must stage the skill and tool before creating the session, always use the newly returned session ID, and call the existing `sendMessage` path exactly once. It must propagate a send failure without retrying. The component remains responsible for setting its consumed ref immediately before calling the helper.

- [ ] **Step 5: Integrate guarded auto-send in `ChatPanel`**

Pass the atomic request from `AiAssistant/index.tsx` to `ChatPanel`. The page may clear the current session and route state, but it must stop staging the skill/tool separately; `ChatPanel` consumes the whole request in order. Clear the route state with `navigate(..., { replace: true, state: null })` after capturing it. For a non-auto-send request, retain current behavior by staging its skill/tool and seeding the composer. For an auto-send request:

1. wait until AI configuration is loaded and usable;
2. set the trusted tool context and active skill before dispatch;
3. use the existing send path so session creation, message persistence, streaming, cancellation, and title generation remain unchanged;
4. set a `useRef` consumed guard immediately before dispatch;
5. set the existing `expectingSessionCreation` ref before awaiting session creation so the null-to-new-session render cannot wipe the in-flight message;
6. create a session because navigation set the current session to `null`;
7. never retry automatically after a send failure.

- [ ] **Step 6: Wire the portfolio alert button**

`buildPortfolioRebalanceNavigation(configId)` returns the `/ai-assistant` path and route state. In `PortfolioAlertsTab`, call `setCurrentSession(null)` before navigation, then navigate with that state. The prompt should request a detailed no-new-capital plan with category buy/sell direction, symbols, approximate amounts, and resulting allocation; trusted values remain in the tool result, not interpolated into the route prompt.

- [ ] **Step 7: Run frontend tests and build**

```bash
node --test src/pages/AiAssistant/prefill.test.ts src/pages/AiAssistant/portfolioRebalancePrefill.test.ts src/pages/AiAssistant/aiPrefillAutoSend.test.ts
node --test src/pages/Alerts/portfolioAlertViewModel.test.ts
bun run build
```

Expected: all pass and TypeScript accepts the new `ChatPanel` contract.

- [ ] **Step 8: Commit the AI navigation flow**

```bash
git add src/pages/AiAssistant/prefill.ts src/pages/AiAssistant/prefill.test.ts src/pages/AiAssistant/portfolioRebalancePrefill.ts src/pages/AiAssistant/portfolioRebalancePrefill.test.ts src/pages/AiAssistant/aiPrefillAutoSend.ts src/pages/AiAssistant/aiPrefillAutoSend.test.ts src/pages/AiAssistant/index.tsx src/pages/AiAssistant/ChatPanel.tsx src/pages/Alerts/PortfolioAlertsTab.tsx
git commit -m "feat: auto-send portfolio rebalancing advice"
```

---

### Task 11: Run end-to-end regression checks and update product documentation

**Files:**
- Modify: `README.md`

- [ ] **Step 1: Update user-facing feature documentation**

Replace references that describe `/alerts` as price-only with “投资提醒”, list the “组合提醒 / 价格提醒” tabs, explain independent scopes and thresholds, and state that AI advice assumes no additional capital and does not place trades.

- [ ] **Step 2: Run formatting and static checks**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
bun run check
```

Expected: both pass without modifying files.

- [ ] **Step 3: Run the complete test suites**

```bash
bun run test
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all frontend and backend tests pass.

- [ ] **Step 4: Run the production build**

```bash
bun run build
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: both pass.

- [ ] **Step 5: Perform a desktop smoke test**

Run the existing Tauri development command and verify this sequence with seeded holdings:

1. Sidebar shows “投资提醒” and opens `/alerts`.
2. “价格提醒” retains create, edit, toggle, and delete behavior.
3. Overall, CN, US, HK, and two account scopes save different target allocations.
4. A target total outside tolerance cannot save; a valid total saves and evaluates.
5. A breached category and concentrated symbol appear with the current pie chart and approximate rebalancing amounts.
6. Repeated quote refresh does not repeat the same notification; recovery clears it; rebreach notifies again.
7. Removing a required cached quote shows the stale snapshot and disables AI advice.
8. “AI 调仓建议” opens the full-page assistant, starts a new session, and sends exactly one message.
9. The AI result assumes zero new capital, includes category and symbol amounts, labels new symbols as candidates, and does not query a symbol outside the selected market.
10. No order-placement action is present.

- [ ] **Step 6: Inspect the final diff for accidental scope expansion**

```bash
git status --short
git diff --stat
git diff --check
```

Expected: only intended feature and documentation files are changed; `.superpowers/` remains untracked and is not staged.

- [ ] **Step 7: Commit documentation and any verification-only corrections**

```bash
git add README.md
git commit -m "docs: document investment alerts workflow"
```
