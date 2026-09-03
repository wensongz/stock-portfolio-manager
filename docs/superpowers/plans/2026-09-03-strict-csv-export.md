# Strict CSV Export Decoding Implementation Plan

**Goal:** Fail CSV export on invalid SQLite types instead of manufacturing zeros or empty fields.

**Architecture:** Typed row decoders strictly read required values and represent only schema-nullable values as options.

**Tech Stack:** Rust, rusqlite, csv, Cargo tests.

---

### Task 1: Expose silent corruption

**Files:**
- Modify: `src-tauri/src/services/import_export_service.rs`

1. Add failing tests that insert text into required numeric holding and transaction columns and assert export returns an error.
2. Add a valid nullable-label test to preserve deliberate blank output.
3. Run `cargo test --manifest-path src-tauri/Cargo.toml services::import_export_service` and confirm the corruption tests fail.

### Task 2: Implement typed decoding

**Files:**
- Modify: `src-tauri/src/services/import_export_service.rs`

1. Add typed holding and transaction export rows with `rusqlite::Result` decoders.
2. Propagate query, row, CSV, and database lock failures.
3. Preserve existing headers, filtering, and formatting for valid rows.
4. Run focused tests and all Rust library tests.
5. Commit as `fix: reject invalid database values during export`.

### Task 3: Final verification

1. Run `bun run check`.
2. Review `git diff --check` and the commit sequence.
3. Resolve only failures introduced by this branch and report any pre-existing warnings separately.
