# P0 Financial Read Integrity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Stop financial read models from fabricating exchange rates or converting database failures into empty/zero data.

**Architecture:** The exchange-rate service remains the sole fallback boundary, optional SQLite reads use `OptionalExtension`, and best-effort AI outputs represent unavailable currency conversions explicitly. Quarterly trend aggregates are loaded in one grouped query and decoded strictly.

**Tech Stack:** Rust 1.97, rusqlite 0.40, SQLite, serde_json, existing Tauri commands and AI tools.

**Spec:** `docs/superpowers/specs/2026-09-02-p0-financial-read-integrity-design.md`

## Global Constraints

- No production hardcoded FX fallback.
- Only `QueryReturnedNoRows` maps to `None`; other read/parse errors propagate.
- Best-effort AI paths use missing/null plus an explanation, never a synthetic financial value.
- Financial formulas and valid business zero values remain unchanged.
- Every behavior change follows a failing-test-first RED/GREEN cycle.

---

### Task 1: Correct optional SQLite reads

**Files:**
- Modify: `src-tauri/src/services/exchange_rate_service.rs`
- Modify: `src-tauri/src/services/quote_service/persistence.rs`
- Modify: `src-tauri/src/services/performance_service.rs`
- Test: the corresponding `#[cfg(test)]` modules.

**Interfaces:**
- Produces: strict `Result<Option<T>, String>` behavior for cache rows, refresh time and previous valuation.
- Consumes: `rusqlite::OptionalExtension`.

- [ ] **Step 1: Write failing tests** for absent rows, wrong SQLite column types and malformed prior-valuation dates.
- [ ] **Step 2: Run** each focused test and verify the existing code returns `None` instead of `Err`.
- [ ] **Step 3: Implement** `.optional().map_err(...)` and explicit date parsing after a successful row read.
- [ ] **Step 4: Re-run** focused tests and confirm pass.

### Task 2: Remove fabricated FX from strict endpoints

**Files:**
- Modify: `src-tauri/src/commands/dashboard.rs`
- Modify: `src-tauri/src/commands/statistics.rs`
- Modify: `src-tauri/src/services/ai_tools.rs`
- Modify: `src-tauri/src/services/quarterly/rebuild.rs`
- Test: adjacent command/service tests or extracted pure helpers.

**Interfaces:**
- Consumes: `get_cached_rates(...) -> Result<ExchangeRates, String>`.
- Produces: endpoint/tool errors that preserve the exchange-rate diagnostic.

- [ ] **Step 1: Write failing helper/endpoint tests** that inject an unavailable rate result and assert no summary is produced.
- [ ] **Step 2: Verify RED** with focused cargo tests.
- [ ] **Step 3: Replace** every `unwrap_or_else` exchange-rate constant with `?` or an explicit tool error branch.
- [ ] **Step 4: Re-run** focused tests and use `rg` to verify no production fallback literals remain.

### Task 3: Explicit degradation for AI context and market overview

**Files:**
- Modify: `src-tauri/src/services/ai_chat/context.rs`
- Modify: `src-tauri/src/services/market_overview_service.rs`
- Test: the same service modules.

**Interfaces:**
- Produces: context rendering with an unavailable-FX notice and `MarketOverview` nullable USD mover data/error reason.
- Consumes: optional valid rates and existing native-currency holding details.

- [ ] **Step 1: Write failing pure rendering tests** asserting unavailable FX never yields aggregate USD numbers and does not remove independent non-currency/index data.
- [ ] **Step 2: Verify RED** with the focused tests.
- [ ] **Step 3: Implement** optional-rate rendering and nullable market-overview currency fields with an availability reason.
- [ ] **Step 4: Re-run** focused tests and confirm serialization uses `null`, not zero.

### Task 4: Strict grouped quarterly trends and category lookup

**Files:**
- Modify: `src-tauri/src/services/quarterly/trends.rs`
- Modify: `src-tauri/src/commands/statistics.rs`
- Test: corresponding module tests.

**Interfaces:**
- Produces: two-query quarterly trend loading and strict optional category lookup.
- Consumes: snapshot ID-to-index map and `OptionalExtension`.

- [ ] **Step 1: Write failing tests** for multi-quarter category vectors, missing aggregate rows as zero, malformed aggregate fields as errors, missing category fallback, and malformed category rows as errors.
- [ ] **Step 2: Verify RED** against the N+1/swallowing implementation.
- [ ] **Step 3: Implement** one grouped trend query, strict row collection and optional category lookup.
- [ ] **Step 4: Re-run** focused tests and confirm outputs are unchanged for valid fixtures.

### Task 5: Full verification and commit

**Files:**
- Verify all modified files.

**Interfaces:**
- Consumes: Tasks 1–4.
- Produces: a repository-wide green quality gate.

- [ ] **Step 1: Run** `rg -n 'usd_cny: 7\\.2|usd_hkd: 7\\.8' src-tauri/src --glob '!**/*test*'` and inspect every remaining hit as a fixture or remove it.
- [ ] **Step 2: Run** `bun run check` and verify frontend tests/build, Rust format, 429 Rust tests and strict Clippy all exit zero.
- [ ] **Step 3: Run** `git diff --check` and review the complete diff against the spec.
- [ ] **Step 4: Commit** with `fix: surface financial read model failures`.
