# Dead Code and Alert Hardening Implementation Plan

**Goal:** Remove unused vertical slices and make alert-check failures explicit and atomic.

**Architecture:** Delete dead UI/Tauri/service surfaces end to end. Preserve the AI alert use case with one transactional service operation.

**Tech Stack:** Rust, rusqlite, TypeScript, Cargo and Node tests.

---

### Task 1: Make alert updates atomic and observable

**Files:**
- Modify: `src-tauri/src/services/alert_service.rs`
- Modify: `src-tauri/src/services/ai_tools.rs`

1. Add a failing service test that forces an update error and verifies no alert status is committed.
2. Add a failing AI tool test that expects an explicit error result.
3. Load and update alerts within one transaction and propagate every failure.
4. Replace `unwrap_or_default` in the AI tool with error mapping.
5. Run focused alert and AI tool tests.

### Task 2: Remove dead chains

**Files:**
- Delete: `src/components/charts/LineChart.tsx`
- Modify: `src/stores/optionStore.ts`
- Modify: `src/stores/alertStore.ts`
- Modify: `src/stores/quarterlyStore.ts`
- Modify: `src/types/index.ts`
- Modify: `src-tauri/src/commands/options.rs`
- Modify: `src-tauri/src/commands/alerts.rs`
- Modify: `src-tauri/src/commands/quarterly.rs`
- Modify: `src-tauri/src/services/quarterly/notes.rs`
- Modify: `src-tauri/src/models/option.rs`
- Modify: `src-tauri/src/models/quarterly.rs`
- Modify: `src-tauri/src/lib.rs`

1. Remove expired-statistics, frontend alert-check, and quarterly-notes-history state and commands end to end.
2. Run reference searches for removed symbols and repair only genuine remaining consumers.
3. Run `bun run check:frontend`, Rust formatting, and focused Rust tests.
4. Commit as `refactor: remove unused data flows and harden alerts`.
