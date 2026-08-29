# 股票复盘市场日历同步验证记录（2026-08-29）

状态：**DONE_WITH_CONCERNS**。自动化回归、真实日历发布、受保护用户表不变量和第二次相同刷新幂等性均通过；但不能宣称严格的全量 live acceptance 通过，原因见“已知限制与关注项”。

## 范围与安全边界

- 验证工作树：`codex/stock-review-market-calendar`，对比基线 `97e77a3`。
- 运行时数据库仅使用 `/Users/wensongzhang/Library/Application Support/com.portfolio.manager/portfolio.db`，未复制或提交数据库、Cookie、原始交易、证券明细、用户安全信息或密钥。
- 日历写入通过生产 `sync_market_calendars` / `sync_market_calendar_with_sources` 路径完成；报告质量检查使用 SQLite 强制只读连接和现有缓存。
- 首次写入前确认没有已运行的投资组合应用、Tauri dev 或 Vite 进程。
- `transactions`、`holdings`、`stock_review_annotations`、`stock_review_overrides` 的行数和内容摘要在每次日历刷新后均逐字节一致。

## 自动化验证矩阵

| 命令 | 结果 |
| --- | --- |
| `cd src-tauri && cargo fmt --check` | **FAIL（既有基线例外）**：仅报告未被本分支修改的 `commands/dividends.rs`、`commands/options.rs`、`commands/quotes.rs`、`commands/transactions.rs`。四个文件均不在 `git diff 97e77a3..HEAD --name-only` 中。 |
| `rustfmt --check --edition 2021 src-tauri/src/db/tests.rs src-tauri/src/models/stock_review.rs src-tauri/src/services/mod.rs src-tauri/src/services/quote_service.rs src-tauri/src/services/stock_action_builder.rs src-tauri/src/services/stock_campaign_builder.rs src-tauri/src/services/stock_review_calendar.rs src-tauri/src/services/stock_review_persistence.rs src-tauri/src/services/stock_review_quality.rs src-tauri/src/services/stock_review_service.rs` | PASS。 |
| `cd src-tauri && cargo test --lib stock_review_calendar::tests -- --nocapture` | PASS：24 passed，0 failed。 |
| `cd src-tauri && cargo test --lib stock_review_service::tests -- --nocapture` | PASS：70 passed，0 failed。 |
| `cd src-tauri && cargo test --lib` | PASS：594 passed，0 failed，8 ignored。 |
| `cd src-tauri && cargo check` | PASS。 |
| `node --test src/hooks/tablePageSize.test.ts src/pages/AiAssistant/prefill.test.ts src/pages/AiAssistant/sidebarPreference.test.ts src/pages/Options/expiredOptionsViewModel.test.ts src/pages/Quarterly/aggregateSnapshotHoldings.test.mjs src/pages/Review/optionReviewViewModel.test.ts src/pages/Review/reviewTabPreference.test.ts src/pages/Review/stockReviewDateBoundary.test.ts src/pages/Review/stockReviewViewModel.test.ts src/pages/Statistics/categoryHoldings.test.ts src/stores/chatStore.test.ts src/stores/optionReviewStore.test.ts src/stores/optionStore.test.ts src/stores/quoteErrors.test.ts src/stores/stockReviewStore.test.ts` | PASS：106 passed，0 failed，0 skipped。 |
| `npm run build` | PASS：TypeScript 与 Vite 生产构建成功。 |

生产构建保留了既有 Vite 警告：压缩后主 JS chunk 约 4,249.05 kB，超过 500 kB；未隐藏或改写该警告。

## 真实数据库基线与不变量

首次日历写入前：

| 表 | 行数 | 指定内容摘要 |
| --- | ---: | ---: |
| `transactions` | 551 | 38248.0 |
| `holdings` | 162 | 12118.0 |
| `stock_review_annotations` | 0 | 0.0 |
| `stock_review_overrides` | 0 | 0.0 |
| `stock_market_sessions` | 0 | — |
| `stock_market_calendar_coverage` | 0 | — |

第一次发布后、第二次相同刷新后以及只读报告检查后，前四个表仍分别为 `551 / 162 / 0 / 0`，内容摘要仍为 `38248.0 / 12118.0 / 0.0 / 0.0`。未发生用户拥有数据变更。

## 日历发布与幂等性

实际涉及的市场为 CN、HK、US。生产同步器第一次返回 `Published`，第二次使用完全相同的市场、起点和固定刷新时刻返回 `Reused`。

| 市场 | authority source | 完整范围 | revision | 自然日行数 | coverage / session 最大更新时间 |
| --- | --- | --- | --- | ---: | --- |
| CN | xueqiu | 2025-12-22..2026-08-29 | `exchange-holidays-v1-2025-2026:97e6cae9563b2628` | 251 | `2026-08-29T12:02:24.039728000Z` |
| HK | xueqiu | 2025-12-22..2026-08-29 | `exchange-holidays-v1-2025-2026:7dce97d92ae4aaa8` | 251 | `2026-08-29T12:02:33.873238000Z` |
| US | xueqiu | 2025-12-22..2026-08-28 | `exchange-holidays-v1-2025-2026:8887bdef147b0d62` | 250 | `2026-08-29T12:02:50.048910000Z` |

递归自然日完整性查询未返回任何缺失行。第二次刷新前后，revision、范围、session 行数、coverage `updated_at` 和 session `MAX(updated_at)` 的查询输出逐字节相同；总计保持 752 个 session rows 和 3 个 coverage rows。

## 报告与 UI 契约证据

- 精确加载文案存在于生产 UI：`正在同步交易日历并生成股票操作复盘…`。
- 强制只读、cache-only 的生产报告检查（YTD、全账户、全市场、USD）返回 authority markets `CN,HK,US`，`market_calendar_unavailable=0`。
- 实际行情覆盖为 81.1506%（不是 94.4%），报告产生 2,395 个独立行情缺口，前端契约单独展示 20 个并标明另有 2,375 个未展示；每项保留市场、证券和日期字段。验证记录未写入证券明细。
- `shadow_return_method=comparable_price_only`；`shadow_dividend_source_incomplete=1` 且 `shadow_degradedreturnmode=1`，证明日历修复未掩盖 price-only 降级。
- 浏览器表面可打开本地“股票操作复盘”并确认筛选器与刷新入口，但普通浏览器没有 Tauri `invoke`，不能控制原生 Tauri webview。原生窗口中的加载瞬态、展开 authority、展开缺口和警告视觉呈现未被直接观察；上述断言由真实只读后端输出、生产组件代码和已通过的前端测试共同覆盖。

## 占位符与调试遗留扫描

运行了 brief 的完整扫描：

```text
rg -n "TO[D]O|TB[D]|FIXM[E]|placeholde[r]|console\.log|dbg!" src src-tauri/src src-tauri/resources
```

全仓输出为既有表单 `placeholder`、聊天占位语义注释以及 `chatSessionStore.ts` 的既有 `console.log`。对 `97e77a3..HEAD` 变更生产/资源文件再次扫描，只命中 `src/types/index.ts` 两行 2026-07-21 已存在的 assistant-placeholder 注释；它们不在本分支 diff hunk 中。本次变更未新增占位符或调试遗留。

## 进程清理

已停止 Tauri、Vite 和两个临时验证 probe，关闭临时浏览器页。最终 `ps` 检查未发现 app/dev/probe 进程，`lsof -nP -iTCP:1420 -sTCP:LISTEN` 无输出。

## 已知限制与关注项

1. `cargo fmt --check` 的全 crate 检查因四个未触碰的既有文件失败；本分支所有变更 Rust 文件的 scoped rustfmt 检查通过。
2. 原生 Tauri UI 无法由允许的浏览器表面控制，因此上述四项原生交互没有直接视觉证据，状态为 DONE_WITH_CONCERNS。
3. Tauri 启动时的既有后台行情刷新在 `2026-08-29T11:58:27Z` 更新了派生缓存表 `cached_quotes`。受保护用户表完全不变，日历写入本身仅修改两个日历表，但严格的“整个进程只改变日历表”断言未满足；未尝试回滚或清理该派生缓存，因此不宣称严格 live acceptance 通过。
