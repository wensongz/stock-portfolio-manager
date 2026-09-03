# Unified Option Matching Design

## Problem

Option import validation, persisted status recomputation, contract reads, and option review currently implement overlapping FIFO and stock-split matching rules. Their split date windows differ, reads mutate persisted status, and an unmatched close can be counted more than once by the command-side implementation.

## Design

Add `src-tauri/src/services/option_matching.rs` as the only quantity-allocation engine. Callers normalize records into stable identifiers, action, contract identity, quantity, and optional timestamps. The engine sorts deterministically, consumes same-contract closes FIFO first, then permits cross-contract allocation only when a split event is strictly after the open date and no later than both the close date and expiry. Each close quantity has one shared remainder and therefore cannot be reused.

The engine returns allocations plus remaining quantities; it does not calculate review economics or write the database. `option_review_service` will keep its campaign and return calculations but build them from engine allocations. `commands/options.rs` will use the same result for import validation, write-time status persistence, and read-time contract projection.

Persisted status remains for compatibility and is recomputed only after option writes. `get_option_contracts_inner` becomes read-only and derives the returned status and completing close details from the shared match result.

## Error Handling and Compatibility

Malformed dates remain review quality issues. Exact-contract matching can still use deterministic record order when dates are absent, while split matching always requires valid dates. Existing import atomicity remains unchanged: any insert or recomputation error rolls back the import transaction.

## Verification

Tests will cover shared FIFO allocation, close conservation, split events outside the open/close window, consistency between review and contract status, and proof that a contract read does not alter database rows.
