# SQLite v2 Indexes Implementation Plan

**Goal:** Add reliable indexes for the highest-frequency transaction and option lookup shapes.

**Architecture:** Fresh schema and sequential migration create the same idempotent indexes after legacy table repair.

**Tech Stack:** SQLite, rusqlite, Rust tests.

---

### Task 1: Write migration failures first

**Files:**
- Modify: `src-tauri/src/db/tests.rs`

1. Add tests that a v1 database migrates to v2 with all required indexes, reopens idempotently, and uses indexes for representative query plans.
2. Run the new tests and confirm they fail against schema version 1.

### Task 2: Implement v2

**Files:**
- Modify: `src-tauri/src/db/migrations.rs`
- Modify: `src-tauri/src/db/schema.rs`

1. Bump `CURRENT_SCHEMA_VERSION` to 2.
2. Add a sequential `migrate_v2` after legacy transaction repair.
3. Add matching fresh-schema index statements.
4. Run database tests, then all Rust library tests.
5. Commit as `perf: add transaction and option indexes`.
