# Request-Scoped Quote Outcomes Implementation Plan

**Goal:** Return quote data, warnings, and truthful refresh timestamps as one request-scoped result.

**Architecture:** Quote fetches collect fallback warnings locally. Tauri commands and the background task emit one outcome shape, and the Zustand store commits it atomically.

**Tech Stack:** Rust/Tauri, TypeScript, Zustand, Node tests.

---

### Task 1: Make provider warnings request-local

**Files:**
- Modify: `src-tauri/src/services/quote_service/mod.rs`
- Modify: `src-tauri/src/services/quote_service/cache.rs`
- Modify: `src-tauri/src/services/quote_service/xueqiu.rs`
- Modify: `src-tauri/src/services/quote_service/tests.rs`

1. Add failing tests for independent concurrent warning outcomes and successful fallback with warning.
2. Introduce a request-local warning collector/outcome and migrate batch fetch paths.
3. Remove global warning clear/take/peek behavior while retaining shared credential state.
4. Run focused quote service tests.

### Task 2: Return truthful command outcomes

**Files:**
- Modify: `src-tauri/src/commands/quotes.rs`
- Modify: `src-tauri/src/lib.rs`

1. Add failing tests showing cache-only requests do not update `quote_last_refresh_time` and refreshes return the persisted time.
2. Return `{ data, warning, refreshedAt }` from quote commands.
3. Remove `take_quote_warning`; emit the complete background outcome once on `quotes-refreshed`.
4. Run `cargo test --manifest-path src-tauri/Cargo.toml commands::quotes services::quote_service`.

### Task 3: Make the frontend store the single owner

**Files:**
- Modify: `src/stores/quoteStore.ts`
- Modify: `src/App.tsx`
- Modify: `src/pages/Dashboard/index.tsx`
- Modify: `src/pages/Holdings/index.tsx`
- Create or modify: `src/stores/quoteStore.test.ts`

1. Add failing tests for atomic outcomes, cache timestamps, and direct background payload application.
2. Update command typing/store reducers and remove polling, duplicate listeners, and module-level initialization.
3. Update direct single-quote consumers to read their own warning outcome.
4. Run `bun run test` and `bun run build`.
5. Commit as `fix: scope quote refresh outcomes to requests`.
