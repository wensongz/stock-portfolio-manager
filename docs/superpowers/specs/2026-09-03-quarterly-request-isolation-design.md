# Quarterly Request Isolation Design

## Problem

Quarterly list, detail, transaction, comparison, and trend requests share one loading/error pair. Concurrent work therefore hides or exposes unrelated spinners and errors, while late responses can overwrite data for a newer snapshot or comparison.

## Design

Split store request state by concern: list, detail bundle, comparison, trends, and mutations. Detail and quarterly transactions are treated as one snapshot-scoped bundle with a generation token. Selecting another snapshot immediately clears both values and invalidates the previous generation.

Comparison requests carry a normalized pair key and generation; trend requests carry their own generation. Only the newest matching request may commit data or errors. List and missing-quarter work no longer changes detail, comparison, or trend loading state.

Pages consume only the status belonging to the data they render. Existing domain payloads and backend commands remain unchanged.

## Verification

Store tests will resolve A and B requests in reverse order, check that stale failures cannot replace newer success, confirm snapshot switches clear transactions, and prove concurrent list/detail/comparison/trend loading states are independent.
