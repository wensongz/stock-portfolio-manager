# Task 2 Report

Status: complete

Changed files:

- `src-tauri/src/services/mod.rs`
- `src-tauri/src/services/portfolio_alert_calculator.rs`

RED run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml portfolio_alert_calculator -- --nocapture
```

Expected red because the calculator module and its public API did not exist yet.

```text
error[E0432]: unresolved imports `super::calculate_portfolio_alert_snapshot`, `super::PortfolioAlertCalculation`, `super::PortfolioAlertCategoryInput`, `super::PortfolioAlertPositionInput`
 --> src/services/portfolio_alert_calculator.rs:8:9
  |
8 |         calculate_portfolio_alert_snapshot, PortfolioAlertCalculation,
  |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^^^^^^^^^ no `PortfolioAlertCalculation` in `services::portfolio_alert_calculator`
  |         |
  |         no `calculate_portfolio_alert_snapshot` in `services::portfolio_alert_calculator`
9 |         PortfolioAlertCategoryInput, PortfolioAlertPositionInput,
  |         ^^^^^^^^^^^^^^^^^^^^^^^^^^^  ^^^^^^^^^^^^^^^^^^^^^^^^^^^ no `PortfolioAlertPositionInput` in `services::portfolio_alert_calculator`
  |         |
  |         no `PortfolioAlertCategoryInput` in `services::portfolio_alert_calculator`
error: could not compile `stock-portfolio-manager` (lib test) due to 1 previous error; 1 warning emitted
```

GREEN focused run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml portfolio_alert_calculator -- --nocapture
```

Result:

```text
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 534 filtered out; finished in 0.00s
```

GREEN full backend run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Result:

```text
test result: ok. 532 passed; 0 failed; 13 ignored; 0 measured; 0 filtered out; finished in 1.21s
```

Edge-case coverage:

- Empty positions and zero-total inputs return `PortfolioAlertCalculation::Empty`.
- Negative and non-finite inputs are rejected with `Err`.
- Exact threshold comparisons stay strict `>` for both allocation and concentration.
- Zero-target categories stay normal at zero current value and become overweight when current value is positive.
- Cash contributes to allocation totals but is excluded from concentration alerts.
- Concentration groups are aggregated across accounts by normalized market and normalized symbol.
- Categories are ordered by settings `sort_order`, and deleted categories collapse into the virtual uncategorized row.
- Scope filtering is enforced for account scope.

Self-review:

- The calculator stays pure: no database, network, cache, or persistence access.
- The returned shape is typed as `Ready` or `Empty`, matching the brief.
- Comparisons are performed on raw values, with no rounding before evaluation.
- Concentration uses normalized symbol aggregation and excludes exact-threshold positions.

Concerns:

- Concentration ordering is deterministic but based on percent, then market, then symbol; if future UI expectations want a different tie-break, that would be a presentation change.
- The uncategorized row is always emitted as the virtual row, which matches the brief but may be worth hiding in the frontend if a zero row is not wanted visually.

Round 1 review fix:

Changed lines:

- `src-tauri/src/services/portfolio_alert_calculator.rs:46-73`
- `src-tauri/src/services/portfolio_alert_calculator.rs:650-779`

RED evidence:

```bash
cargo test --manifest-path src-tauri/Cargo.toml portfolio_alert_calculator -- --nocapture
```

```text
thread 'services::portfolio_alert_calculator::tests::target_percentages_must_sum_to_one_hundred_percent' ... panicked at src/services/portfolio_alert_calculator.rs:737:9:
assertion failed: result.is_err()
test services::portfolio_alert_calculator::tests::target_percentages_must_sum_to_one_hundred_percent ... FAILED
```

Why red was expected:

- The calculator did not yet reject target mixes whose percentages summed outside the `100.0 ± 0.01` tolerance.

GREEN evidence:

```bash
cargo test --manifest-path src-tauri/Cargo.toml portfolio_alert_calculator -- --nocapture
```

```text
test result: ok. 15 passed; 0 failed; 0 ignored; 0 measured; 534 filtered out; finished in 0.01s
```

What changed in this round:

- Added `TARGET_PERCENT_SUM_TOLERANCE` and rejected invalid target sums in `validate_inputs`.
- Added proof tests for market scope filtering and same-symbol cross-market concentration separation.
- Added the rebalance-sum check so the unrounded category deltas stay balanced at zero for a valid target mix.
