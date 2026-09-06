# Import Batches Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development with independent frontend tasks and backend work, then integrated review.

**Goal:** 股票导入可去重、对账、失败重试和受保护撤销。
**Architecture:** Rust 批次服务持久化预览和结果，现有 mutation 服务负责账本写入，React 导入入口统一调用。账户状态快照保护撤销。
**Tech Stack:** Rust / rusqlite / React / TypeScript / Ant Design，无新依赖。
**Spec:** docs/superpowers/specs/2026-09-06-import-batches-design.md

## Global Constraints
- 不修改真实账户数据库；内存测试。
- 保留原始内容与解析版本；不把疑似重复当成确定重复。
- 期权保持既有独立流程。

## Tasks
- [x] Backend: add migration v6, models/import_batch.rs, services/import_batch/{mod,state,dedup,tests}.rs, commands/import_batches.rs and registration. Test idempotence, duplicate classifications, partial failure, retry, reconciliation, atomic guarded undo and migration preservation before implementation.
- [x] Frontend import entrypoints: add batchTypes.ts and batch adapter; change broker import wizard and OCR to staged batch API; preserve metadata and raw source; disable dismissal during mutation; test selection/retry/normalization against API contract.
- [x] Batch history/reconciliation: add ImportBatchPanel.tsx, ImportBatchHistory.tsx, integrate generic CSV preview and history on Import page. Explicit duplicate selection, expected-balance entry, conflicts, undo and retry must be user accessible after restart.
- [x] Integration: route legacy CSV confirmation via service, invalidate daily snapshots, update README, run node --test, bun run build, cargo test --lib, cargo fmt, cargo clippy. Review for data-loss and idempotency races.

## Execution ledger
- Prior review baseline: frontend 244 passed, Rust 606 passed / 13 ignored; build passed.
- Ruling: user approval of prior concrete recommendation authorizes implementation; no repeated permission gate.
- Ruling: account-wide conflict guard is deliberately conservative; it protects later changes and permits later batches to be undone in reverse order.

- Completed backend migration v6 and immutable batch input/row audit; per-row savepoints inside atomic submission; guarded account restore and daily snapshot invalidation.
- Completed broker/generic/OCR integrations, execution-ID metadata, persistent history, explicit suspected confirmation, balance inputs including broker-only symbols.
- Review fixes: newly suspected selection reset; Moomoo execution IDs retained per fill; nonfinite computed state rejected inside row savepoint; same original file row with modified payload rejected as conflict.
- Final verification: node --test 262 passed; cargo test --lib --offline 620 passed / 13 ignored; bun run build passed (existing >500kB advisory); cargo fmt --check and cargo clippy --all-targets --all-features --offline -- -D warnings passed; git diff --check passed.
- No real user account database was used. No full desktop GUI walkthrough was performed. Implementation retained on codex/import-batches; no merge or push.
