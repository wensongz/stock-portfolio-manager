# Conservative Code Health Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reject invalid holding costs, remove the non-functional quote refresh setting and unused quote state, and make strict Rust linting pass without changing quote-fetch behavior.

**Architecture:** Keep the existing React/Zustand/Tauri boundaries intact. Add one backend value-validation boundary used by both holding commands, then mechanically remove frontend state and UI whose behavior is provably absent or unconsumed.

**Tech Stack:** React 19, TypeScript 7, Zustand 5, Tauri 2, Rust 1.97.1, rusqlite

**Spec:** `docs/superpowers/specs/2026-08-20-conservative-code-health-design.md`

## Global Constraints

- `symbol` is globally unique; do not change symbol-based aggregation, deduplication, or cache keys.
- Keep quote behavior as startup cache synchronization plus user-triggered refresh; do not add a timer.
- Do not upgrade dependencies, split modules, or change the database schema.
- Do not stage or modify the user's existing `src-tauri/Cargo.lock` change.

---

### Task 1: Validate Holding Average Cost

**Files:**
- Modify: `src-tauri/src/commands/holdings.rs`
- Modify: `src/pages/Holdings/index.tsx`

**Interfaces:**
- Consumes: holding command arguments `market: &str`, `symbol: &str`, `shares: f64`, and `avg_cost: f64`
- Produces: `validate_holding_values(market: &str, symbol: &str, shares: f64, avg_cost: f64) -> Result<(), String>`

- [ ] **Step 1: Write failing backend validation tests**

Append a test module to `src-tauri/src/commands/holdings.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::validate_holding_values;

    #[test]
    fn validate_holding_values_rejects_invalid_average_cost() {
        for avg_cost in [-0.01, f64::NAN, f64::INFINITY] {
            assert!(validate_holding_values("US", "AAPL", 1.0, avg_cost).is_err());
        }
    }

    #[test]
    fn validate_holding_values_accepts_zero_average_cost() {
        assert!(validate_holding_values("US", "AAPL", 1.0, 0.0).is_ok());
    }
}
```

- [ ] **Step 2: Run the focused test and verify RED**

Run: `cargo test commands::holdings::tests::validate_holding_values -- --nocapture`

Expected: compilation fails because `validate_holding_values` does not exist yet. This confirms the test requires the new validation boundary.

- [ ] **Step 3: Implement the minimal backend validation**

Replace `validate_holding_shares` with:

```rust
fn validate_holding_values(
    market: &str,
    symbol: &str,
    shares: f64,
    avg_cost: f64,
) -> Result<(), String> {
    if !shares.is_finite() || shares < 0.0 {
        return Err("Holding shares must be a non-negative number".to_string());
    }
    if !symbol.starts_with("$CASH-") && market != "US" && shares.fract().abs() > 1e-9 {
        return Err(
            "Only US holdings support fractional shares; CN and HK holdings must use whole shares"
                .to_string(),
        );
    }
    if !avg_cost.is_finite() || avg_cost < 0.0 {
        return Err("Holding average cost must be a non-negative number".to_string());
    }
    Ok(())
}
```

Update both command call sites to pass `avg_cost`:

```rust
validate_holding_values(&market, &symbol, shares, avg_cost)?;
```

- [ ] **Step 4: Add the matching form constraint**

In `src/pages/Holdings/index.tsx`, change the average-cost input to:

```tsx
<InputNumber min={0} precision={4} style={{ width: "100%" }} placeholder="买入均价" />
```

- [ ] **Step 5: Verify GREEN**

Run: `cargo test commands::holdings::tests::validate_holding_values -- --nocapture`

Expected: 2 focused tests pass.

Run: `npx tsc --noEmit --noUnusedLocals --noUnusedParameters`

Expected: exit code 0 with no diagnostics.

- [ ] **Step 6: Commit the validation change**

```bash
git add src-tauri/src/commands/holdings.rs src/pages/Holdings/index.tsx
git commit -m "fix: reject invalid holding average costs"
```

### Task 2: Remove Non-Functional Quote Refresh Settings

**Files:**
- Modify: `src/stores/quoteStore.ts`
- Modify: `src/pages/Settings/GeneralSettings.tsx`

**Interfaces:**
- Consumes: current manual-refresh and backend `quotes-refreshed` event behavior
- Produces: `startQuoteSync() -> () => void`; no refresh interval state or persisted setting

- [ ] **Step 1: Reconfirm the setting has no runtime consumer**

Run:

```bash
rg -n "refreshIntervalMs|setRefreshInterval|quote_refresh_interval_ms|startAutoRefresh|fetchQuotes|quotes:|warning:|error:" src/stores/quoteStore.ts src/pages/Settings/GeneralSettings.tsx src/pages/Holdings/index.tsx src/App.tsx
```

Expected: refresh interval references are confined to the store and settings page; `fetchQuotes` and the standalone `quotes`, `warning`, and `error` state have no external consumer.

- [ ] **Step 2: Remove dead quote-store state and rename synchronization**

In `src/stores/quoteStore.ts`:

- Remove `StockQuote` from the type import.
- Remove refresh interval constants and `loadRefreshInterval`.
- Remove `quotes`, `error`, `warning`, `refreshIntervalMs`, `fetchQuotes`, and `setRefreshInterval` from the interface and store implementation.
- Stop building the unused `quotes` object inside `fetchHoldingQuotes`.
- Keep `fetchHoldingQuotes`, `quoteWarning`, `loading`, and `lastUpdatedAt` behavior unchanged.
- Rename `startAutoRefresh` to `startQuoteSync` without changing its body.
- Simplify successful and failed state writes so they only target fields that remain.

The resulting interface must retain these signatures:

```ts
fetchHoldingQuotes: (refreshSymbols?: [string, string][]) => Promise<void>;
setQuoteWarning: (warning: string | null) => void;
startQuoteSync: () => () => void;
```

- [ ] **Step 3: Remove the misleading settings UI**

In `src/pages/Settings/GeneralSettings.tsx`:

- Delete `INTERVAL_OPTIONS`.
- Remove `quote_refresh_interval_ms` from `LOCAL_STORAGE_KEYS`.
- Remove the `useQuoteStore` import and hook call.
- Delete `handleIntervalChange`.
- Delete the `setRefreshInterval(5 * 60_000)` factory-reset write.
- Delete the entire “行情刷新设置” card.

- [ ] **Step 4: Update the synchronization caller and comments**

In `src/pages/Holdings/index.tsx`, use:

```tsx
const { startQuoteSync } = useQuoteStore.getState();
return startQuoteSync();
```

In `src/App.tsx`, remove comments that claim a deleted `fetchQuotes` path writes quote warnings.

- [ ] **Step 5: Verify removal and behavior-preserving compilation**

Run:

```bash
rg -n "refreshIntervalMs|setRefreshInterval|quote_refresh_interval_ms|startAutoRefresh|fetchQuotes" src
```

Expected: no matches.

Run: `npx tsc --noEmit --noUnusedLocals --noUnusedParameters`

Expected: exit code 0 with no diagnostics.

Run: `node --test src/pages/Quarterly/aggregateSnapshotHoldings.test.mjs`

Expected: the 2 existing tests pass, confirming symbol aggregation was not changed.

- [ ] **Step 6: Commit the quote-state cleanup**

```bash
git add src/stores/quoteStore.ts src/pages/Settings/GeneralSettings.tsx src/pages/Holdings/index.tsx src/App.tsx
git commit -m "refactor: remove inactive quote refresh state"
```

### Task 3: Clear the Strict Rust Lint Warning

**Files:**
- Modify: `src-tauri/src/commands/transactions.rs`

**Interfaces:**
- Consumes: the existing over-withdrawal `Result` assertion
- Produces: the same assertion through `Result::expect_err`

- [ ] **Step 1: Reproduce the strict Clippy failure**

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: failure at `src/commands/transactions.rs` for `clippy::err-expect`.

- [ ] **Step 2: Apply the minimal test-only simplification**

Replace:

```rust
let err = validate_cash_withdrawal(&conn, account_id, "$CASH-USD", 500.0)
    .err()
    .expect("over-withdrawal must be rejected");
```

with:

```rust
let err = validate_cash_withdrawal(&conn, account_id, "$CASH-USD", 500.0)
    .expect_err("over-withdrawal must be rejected");
```

- [ ] **Step 3: Verify the focused test and strict lint**

Run: `cargo test commands::transactions::tests::test_cash_withdraw_over_balance_rejected`

Expected: 1 test passes.

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: exit code 0 with no warnings.

- [ ] **Step 4: Commit the lint cleanup**

```bash
git add src-tauri/src/commands/transactions.rs
git commit -m "test: simplify withdrawal error assertion"
```

### Task 4: Full Verification and Scope Audit

**Files:**
- Verify: all files changed by Tasks 1–3

**Interfaces:**
- Consumes: the complete approved implementation
- Produces: fresh evidence for tests, build, lint, scope, and preservation of the user-owned lockfile change

- [ ] **Step 1: Run all frontend checks**

Run: `node --test src/pages/Quarterly/aggregateSnapshotHoldings.test.mjs`

Expected: 2 tests pass.

Run: `npx tsc --noEmit --noUnusedLocals --noUnusedParameters`

Expected: exit code 0 with no diagnostics.

Run: `npm run build`

Expected: exit code 0. The existing large-chunk advisory may remain because bundle splitting is outside scope.

- [ ] **Step 2: Run all backend checks**

Run: `cargo test`

Expected: all non-ignored tests pass; network integration tests remain ignored.

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: exit code 0 with no warnings.

- [ ] **Step 3: Audit the final diff and scope**

Run:

```bash
git diff --check
git status --short
git diff HEAD~3 -- src/pages/Quarterly/aggregateSnapshotHoldings.ts src/pages/Quarterly/aggregateSnapshotHoldings.test.mjs
git diff -- src-tauri/Cargo.lock
```

Expected:

- `git diff --check` succeeds.
- Quarterly aggregation files have no changes.
- `src-tauri/Cargo.lock` still contains only the pre-existing application version change and is not included in implementation commits.
- No files outside the approved design and plan are modified.

- [ ] **Step 4: Summarize verified changes**

Report the average-cost validation, removed inactive refresh setting/state, strict lint cleanup, exact verification outcomes, and the intentionally preserved `Cargo.lock` modification.
