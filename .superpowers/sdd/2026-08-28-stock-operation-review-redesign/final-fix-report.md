# Stock Operation Review Final Fix Report

Date: 2026-08-29
Branch: `codex/stock-operation-review-redesign`
Implementation commit: `49b8f75`

## Outcome

The four final-review findings are fixed in one implementation wave. The fix keeps the existing public Rust/TypeScript report contract intact: all new structures and fields are internal deterministic-input boundaries, and newly deserialized internal fields use `serde(default)` so an older cached shape degrades to unavailable rather than failing to decode or inventing a value. No dependency or database-schema change was added.

## 1. Authoritative Campaign position replay

Decision:

- Campaign cash flows remain cash authority only.
- A separate ordered `CampaignPositionEvent` stream is now share authority for `OPEN`, ordinary trades, transfers, and splits.
- Remaining quantity and remaining market value remain factual even when an imported `OPEN` has no reliable cost basis.
- Campaign return, P&L, invested capital, MAE, and MFE become unavailable when opening cost basis or required path inputs are unknown; zero is never substituted.
- Split quantity deltas are applied before same-day stock flows, matching the shadow and attribution engines.

RED evidence:

- The first focused test compile failed because `CampaignPositionEvent`, `CampaignPositionEventKind`, and `CampaignDetailInput.position_events` did not exist.
- After the first implementation pass, the split-excursion test failed with expected MAE `-20` but actual `+20`; cash flows were still changing shares as well as the new position stream. Removing that double application produced the intended path.

GREEN coverage:

- `synthetic_open_only_keeps_factual_quantity_but_not_invented_campaign_pnl`
- `synthetic_open_then_partial_sell_preserves_remaining_quantity_without_guessing_basis`
- `campaign_position_stream_applies_split_quantity_to_pnl_and_excursions`
- `live_split_materialization_preserves_shadow_value` now also proves `OPEN +10`, split `+10`, factual remaining shares `20`, and unavailable P&L/return.

## 2. Market-aware security identity and legacy split authority

Decision:

- Position and fallback keys use `(account_id, normalized_symbol, normalized_market)`.
- Risk concentration uses `(normalized_symbol, normalized_market)` and no longer collapses equal codes from different markets.
- The marketless legacy `stock_splits` table is not treated as market authority. A split applies only when global transaction and current-holding identities resolve the normalized code to exactly one market.
- A code observed in multiple markets causes the split to be skipped deterministically and emits `split_market_ambiguous` for a relevant report.
- The global split identity set is read both before and after async cache preparation; a change aborts candidate preparation instead of blessing a mixed revision.

RED evidence:

- Risk coverage initially failed to compile because `StockValueBase.market` did not exist.
- The opening/fallback regression initially returned one position instead of the literal two positions.
- The first split regression applied a marketless split despite a two-market identity. A second RED compile established that split loading needed an explicit global authority input; this specifically covered a market-filtered report that could otherwise hide the ambiguity.

GREEN coverage:

- `risk_concentration_keeps_equal_codes_in_different_markets_separate` proves weights `0.60` and HHI `0.52`, not a fabricated 100% position.
- `opening_and_legacy_fallback_identity_include_market` proves separate US/CN openings, fixed benchmark weights, and shadow valuation for the same account/code in two markets.
- `marketless_split_is_skipped_when_equal_code_has_multiple_market_authorities` proves filtered ambiguity remains skipped, while a single-market authority applies deterministically.

## 3. Account-filtered confirmed transfers

Decision:

- Query filtering no longer happens before logical transfer derivation.
- An internal derivation ledger includes referenced transfer legs and the referenced account/security histories needed to reconstruct each position path.
- After action/Campaign derivation, actions, fragments, Campaign account IDs, cash flows, position events, attribution, and risk inputs are projected back to the requested account/market.
- Confirmed transfer legs remain non-trades and cannot contribute turnover or fee drag.
- Invalid or missing references remain invalid; they are not promoted into transfer facts.

RED evidence:

- The first account-filtered test lost the destination `transfer_in` fragment and produced an invalid/negative path because the opposite leg had been filtered out.
- Including only the opposite referenced row was still insufficient: its pre-transfer opening history was absent. Expanding the internal ledger to the complete referenced account/security history resolved the position path.

GREEN coverage:

- `account_filtered_transfer_derives_with_both_legs_then_projects_only_local_fragment` covers source and destination queries plus a missing reference.
- `live_confirmed_transfer_is_excluded_from_turnover_and_stays_one_campaign` now proves both local directions, zero trade scoring, one local Campaign fragment, and no opposite-account action, position, cash-flow, or financial-value leakage.

## 4. Actual valuation coverage and terminal cutoff

Decision:

- Expected portfolio valuation dates are the union of explicit scoped exchange sessions, never inferred from snapshot or price rows.
- The TWR baseline is loaded on the exact authoritative pre-period session, so a later non-session snapshot cannot mask a valid baseline.
- The exact terminal session is mandatory.
- At least 95% session coverage is available; 80% through below 95% is degraded and may retain TWR, but drawdown is cleared; below 80% is unavailable and clears TWR and drawdown.
- Missing or invalid calendar authority makes actual result and drawdown unavailable.
- Shadow valuation dates still use the authoritative full session set, so a missing actual terminal does not erase an independently valid shadow return. Actual-vs-shadow value-add remains unavailable without both comparable sides.

RED evidence:

- Coverage tests first failed to compile because `expected_actual_dates` and `expected_baseline_date` did not exist.
- Service integration then failed to compile until the production constructor supplied those fields.
- The exact-cutoff regression initially failed to compile because `load_actual_values` could not accept an authoritative baseline date; the previous query selected the latest arbitrary pre-start snapshot.

GREEN coverage:

- `actual_valuation_coverage_requires_exact_baseline_and_terminal_observation`
- `actual_valuation_coverage_applies_95_and_80_percent_boundaries`
- `actual_snapshot_baseline_uses_the_exact_authoritative_cutoff`
- `nonempty_session_rows_without_coverage_metadata_are_not_authority`
- `missing_actual_terminal_does_not_suppress_an_independent_shadow_result`

These cover stale/missing baseline, missing terminal, sparse internal paths, exact 95% and 80% boundaries, below-80% suppression, a complete interval, missing calendar authority, exact baseline cutoff, and actual/shadow independent degradation.

## Changed implementation files

- `src-tauri/src/services/stock_review_metrics.rs`
- `src-tauri/src/services/stock_review_service.rs`

No public model, TypeScript type, frontend view, dependency manifest, migration, or database schema changed.

## Final verification evidence

All commands ran from the designated worktree after feature-file formatting:

- `rustfmt --edition 2021 src-tauri/src/services/stock_review_metrics.rs src-tauri/src/services/stock_review_service.rs` — PASS.
- `cargo test --manifest-path src-tauri/Cargo.toml --lib stock_ -- --nocapture` — PASS, 186 passed, 0 failed, 373 filtered.
- `cargo test --manifest-path src-tauri/Cargo.toml stock_review_metrics::tests -- --nocapture` — PASS, 36 passed, 0 failed.
- `cargo test --manifest-path src-tauri/Cargo.toml stock_review_service::tests -- --nocapture` — PASS, 55 passed, 0 failed.
- `cargo test --manifest-path src-tauri/Cargo.toml` — PASS, 551 passed, 0 failed, 8 ignored.
- Full Node inventory including the quarterly `.mjs` tests — PASS, 104 passed, 0 failed, 0 skipped.
- `cargo check --manifest-path src-tauri/Cargo.toml` — PASS.
- `cargo build --manifest-path src-tauri/Cargo.toml` — PASS.
- `npm run build` — PASS, TypeScript and Vite production build completed with 4,743 modules transformed.
- `git diff --check` — PASS.

The eight ignored Rust tests remain the repository's opt-in network quote integration tests. The frontend build emits the pre-existing large-chunk warning and no build error.

## Limitations retained deliberately

- `stock_splits` still has no market column. Ambiguous same-code records are therefore skipped and surfaced instead of guessed; resolving them requires future authoritative market metadata or a schema migration.
- A synthetic/imported `OPEN` does not establish cost basis merely because quantity is known. Campaign quantity/value remain visible, while return/P&L/excursions remain unavailable until reliable basis exists.
- Filtered daily holding snapshots still lack an authoritative account/market cash ledger, so filtered actual TWR and NAV-dependent ratios retain their established honest degradation.
- No live-provider or full interactive desktop acceptance run was added in this final fix wave; deterministic backend, frontend, check, and production-build coverage is green.
