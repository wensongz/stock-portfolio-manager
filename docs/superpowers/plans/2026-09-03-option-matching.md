# Unified Option Matching Implementation Plan

**Goal:** Make every option workflow use one FIFO and split-allocation policy without write-on-read behavior.

**Architecture:** A pure service module returns conserved allocations and remainders. Command and review layers adapt domain records and retain their own persistence or financial calculations.

**Tech Stack:** Rust, rusqlite, chrono, Cargo tests.

---

### Task 1: Specify the shared matcher

**Files:**
- Create: `src-tauri/src/services/option_matching.rs`
- Modify: `src-tauri/src/services/mod.rs`

1. Add failing unit tests for exact-contract FIFO, split-window boundaries, deterministic ordering, partial closes, and close-quantity conservation.
2. Run `cargo test --manifest-path src-tauri/Cargo.toml services::option_matching` and confirm the new tests fail.
3. Implement the smallest normalized input, allocation output, and matching function that passes them.
4. Run the focused tests again.

### Task 2: Migrate review matching

**Files:**
- Modify: `src-tauri/src/services/option_review_service.rs`

1. Add a regression test proving review allocations match the engine for split and overlapping-open cases.
2. Replace local FIFO and split predicate logic with engine allocations while retaining campaign economics and quality reporting.
3. Run `cargo test --manifest-path src-tauri/Cargo.toml services::option_review_service`.

### Task 3: Migrate commands and remove write-on-read

**Files:**
- Modify: `src-tauri/src/commands/options.rs`

1. Add failing tests that one close cannot complete multiple opens, split rules match review, and `get_option_contracts_inner` leaves persisted rows unchanged.
2. Replace import validation, status recomputation, and contract projection matching with the engine.
3. Keep recomputation only on write paths and derive read results without updates.
4. Run `cargo test --manifest-path src-tauri/Cargo.toml commands::options` and the review suite.
5. Commit as `fix: unify option fifo matching`.
