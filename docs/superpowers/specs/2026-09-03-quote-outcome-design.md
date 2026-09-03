# Request-Scoped Quote Outcome Design

## Problem

Quote warnings are stored as one globally consumable string, so unrelated callers can steal each other's warnings. Cache-only reads write a fresh refresh timestamp, and frontend initialization races with active requests. Background refresh also splits data, warning, and timestamp across different events and commands.

## Design

Introduce a serialized quote command outcome containing `data`, `warning`, and `refreshedAt`. Provider fallback warnings will be collected within the request and returned with that request's quotes instead of being consumed from shared application state. Credential state remains shared, but warning state does not.

An actual successful provider refresh persists one timestamp and returns that exact value. A cache-only request reads the last persisted timestamp without modifying it. The background refresh emits one `quotes-refreshed` payload containing the complete outcome.

`quoteStore` becomes the sole owner of quote refresh state. It applies command results and background event payloads atomically. `App`, Dashboard, and Holdings stop polling or consuming warnings, and the module-level timestamp invocation is removed.

## Error Handling and Compatibility

Refresh failures remain rejected operations and preserve the last complete frontend state. A successful fallback can return data with a warning. Internal Tauri call sites are migrated together; no external API compatibility layer is required.

## Verification

Tests will prove that cache-only reads preserve the persisted timestamp, warnings stay attached to their originating request, frontend results are applied atomically, and background event payloads do not trigger a second cache fetch.
