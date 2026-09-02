# P1 Unused Runtime Surface Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 删除已确认没有运行时消费者的旧复盘、旧 command、前端 action、组件、类型和直接依赖，同时保留仍被股票操作复盘、AI、快照、技能克隆/导出使用的内部能力。

**Architecture:** 以公开边界为单位收敛代码：先用引用扫描锁定消费者，再删除前端 action、Tauri command 与注册，最后删除只被这些入口使用的 service/model 文件。绩效和技能的内部 service 函数继续保留，数据库字段与历史数据不变。

**Tech Stack:** React 19、TypeScript 7、Zustand 5、Tauri 2、Rust 1.97、Bun、Cargo、PostCSS。

**Spec:** `docs/superpowers/specs/2026-09-02-p1-simplification-and-read-model-design.md`

## Global Constraints

- 不删除 `commands/review.rs`，其中股票操作复盘 command 和查询规范化测试仍在使用。
- 不删除 `performance_service` 中与旧 command 同名的函数；AI、快照与 Rust 测试仍依赖它们。
- 不删除 `skill_service::get_skill`；克隆和导出仍依赖它。
- 不删除数据库中的 `decision_quality` 字段、迁移或历史数据。
- 不改变 `get_performance_report`、`get_benchmark_return_series`、`convertWithCachedRates` 的行为。
- 不顺带升级依赖；锁文件只能反映删除 `autoprefixer` 直接依赖所需的变化。
- 删除型重构不增加读取源码文本的脆弱单元测试；以编译、引用图、冻结锁文件安装和完整门禁验证。

---

### Task 1: 固化消费者边界与基线

**Files:**
- Inspect: `src-tauri/src/lib.rs`
- Inspect: `src-tauri/src/commands/review.rs`
- Inspect: `src-tauri/src/commands/performance.rs`
- Inspect: `src-tauri/src/commands/skills.rs`
- Inspect: `src-tauri/src/commands/exchange_rates.rs`
- Inspect: `src-tauri/src/services/ai_tools.rs`
- Inspect: `src-tauri/src/services/ai_chat/context.rs`
- Inspect: `src-tauri/src/services/snapshot_service.rs`
- Inspect: `src/stores/reviewStore.ts`
- Inspect: `src/stores/skillStore.ts`
- Inspect: `src/stores/exchangeRateStore.ts`

**Interfaces:**
- Produces: 一份执行日志，证明待删符号没有运行时消费者，并记录必须保留的内部 service 消费者。
- Consumes: 全仓 Rust/TypeScript 引用图。

- [ ] **Step 1: 运行公开入口扫描**：

  ```bash
  rg -n 'get_holding_review|update_decision_quality|get_decision_statistics|get_reviewed_symbols|get_performance_summary|get_return_attribution|get_monthly_returns|get_holding_performance_ranking|get_risk_metrics|get_drawdown_analysis|get_skill|convert_amount|convertAmount|NotesTimeline|OptionRecord|theme-tokens' src src-tauri package.json postcss.config.js
  ```

- [ ] **Step 2: 运行内部能力消费者扫描**，确认这些 service 函数在删除 command 后仍有调用方：

  ```bash
  rg -n 'performance_service::(get_performance_summary|get_return_attribution|get_monthly_returns|get_holding_performance_ranking|get_risk_metrics|get_drawdown_analysis)|skill_service::get_skill' src-tauri/src
  ```

- [ ] **Step 3: 记录当前质量门禁基线**：运行 `bun run check`，任何既有失败必须先单独记录，不能归因于本计划的删除。

### Task 2: 删除旧持仓复盘孤岛

**Files:**
- Delete: `src/stores/reviewStore.ts`
- Delete: `src-tauri/src/services/review_service.rs`
- Delete: `src-tauri/src/models/review.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Modify: `src-tauri/src/models/mod.rs`
- Modify: `src-tauri/src/commands/review.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/commands/review.rs` existing `#[cfg(test)]` module

**Interfaces:**
- Removes: `get_holding_review`, `update_decision_quality`, `get_decision_statistics`, `get_reviewed_symbols`。
- Preserves: `get_stock_operation_review` 与 `stock_operation_query(...)`。

- [ ] **Step 1: 删除四个旧 command 的注册**，同时保留以下注册：

  ```rust
  commands::review::get_stock_operation_review,
  ```

- [ ] **Step 2: 收窄 `commands/review.rs` 的导入**：

  ```rust
  use crate::models::stock_operation_review::{
      StockOperationReviewQuery, StockOperationReviewReport,
  };
  use crate::services::quote_service::QuoteServiceState;
  use crate::services::stock_operation_review_service;
  ```

  删除四个旧 command 函数，保留查询解析、股票操作复盘 command 和现有两个测试。

- [ ] **Step 3: 从模块树删除** `pub mod review_service;` 与 model 的 `pub mod review;`，然后删除三个孤立文件。

- [ ] **Step 4: 验证股票操作复盘边界**：

  ```bash
  cargo test --manifest-path src-tauri/Cargo.toml commands::review::tests
  cargo check --manifest-path src-tauri/Cargo.toml --all-targets
  ```

- [ ] **Step 5: 重新扫描旧复盘符号**；结果只能包含数据库字段/迁移中的 `decision_quality`，不能再有旧 model、service 或 command 引用。

### Task 3: 收敛绩效公开 command，保留内部计算能力

**Files:**
- Modify: `src-tauri/src/commands/performance.rs`
- Modify: `src-tauri/src/lib.rs`
- Verify: `src-tauri/src/services/performance_service.rs`
- Verify: `src-tauri/src/services/ai_tools.rs`
- Verify: `src-tauri/src/services/ai_chat/context.rs`
- Verify: `src-tauri/src/services/snapshot_service.rs`

**Interfaces:**
- Removes: 六个细粒度 Tauri command。
- Preserves: `get_performance_report`、`get_benchmark_return_series` 和全部内部 service 函数。

- [ ] **Step 1: 删除六个细粒度 command 的注册和 wrapper 函数**：

  ```text
  get_performance_summary
  get_return_attribution
  get_monthly_returns
  get_holding_performance_ranking
  get_risk_metrics
  get_drawdown_analysis
  ```

- [ ] **Step 2: 将 command model 导入缩减为实际返回类型**：

  ```rust
  use crate::models::performance::{PerformanceReport, ReturnDataPoint};
  ```

- [ ] **Step 3: 保留** `BENCHMARK_BASELINE_LOOKBACK_DAYS`、`parse_date`、`build_filter`、`get_performance_report` 和 `get_benchmark_return_series`；不得移动或改写其日期/基准算法。

- [ ] **Step 4: 运行引用扫描**，确认六个同名符号仅剩 `performance_service` 定义、内部消费者与测试，不再出现在 `generate_handler!` 或前端 invoke 中。

- [ ] **Step 5: 运行绩效定向测试和 Rust 检查**：

  ```bash
  cargo test --manifest-path src-tauri/Cargo.toml performance
  cargo check --manifest-path src-tauri/Cargo.toml --all-targets
  ```

### Task 4: 删除未使用的技能与汇率远程 action

**Files:**
- Modify: `src/stores/skillStore.ts`
- Modify: `src-tauri/src/commands/skills.rs`
- Modify: `src/stores/exchangeRateStore.ts`
- Modify: `src-tauri/src/commands/exchange_rates.rs`
- Modify: `src-tauri/src/lib.rs`
- Verify: `src-tauri/src/services/skill_service.rs`
- Test: existing frontend store tests and Rust service tests

**Interfaces:**
- Removes: Zustand `getSkill`、Tauri `get_skill`、Zustand `convertAmount`、Tauri `convert_amount`。
- Preserves: 技能列表/保存/克隆/导入导出以及缓存汇率换算。

- [ ] **Step 1: 从 `SkillState` 与 store creator 删除**：

  ```ts
  getSkill: (id: string) => Promise<Skill | null>;
  ```

  以及对应 `invoke<Skill>("get_skill", { id })` action。

- [ ] **Step 2: 删除 `commands::skills::get_skill` 注册和公开 wrapper**；保留导出中的内部调用：

  ```rust
  let skill = skill_service::get_skill(&app, &id)?;
  ```

- [ ] **Step 3: 从 `ExchangeRateState` 与 store creator 删除**：

  ```ts
  convertAmount: (amount: number, from: Currency, to: Currency) => Promise<number>;
  ```

  保留 `fetchRates`、`setBaseCurrency` 与 `convertWithCachedRates`。

- [ ] **Step 4: 删除 `convert_amount` 注册和 command 函数**，并将 import 收窄为：

  ```rust
  use crate::services::exchange_rate_service::{get_cached_rates, ExchangeRateCache};
  ```

- [ ] **Step 5: 运行** `bun test` 与 `cargo check --manifest-path src-tauri/Cargo.toml --all-targets`，确认所有调用方使用保留的边界。

### Task 5: 删除零散死代码与未使用直接依赖

**Files:**
- Delete: `src/pages/Quarterly/NotesTimeline.tsx`
- Delete: `src/styles/theme-tokens.ts`
- Modify: `src/types/index.ts`
- Modify: `package.json`
- Modify: `bun.lock`
- Verify: `postcss.config.js`

**Interfaces:**
- Removes: 未挂载季度组件、未导入主题 token、TypeScript `OptionRecord`、直接 `autoprefixer` 依赖。
- Preserves: Rust `OptionRecord` 与 `@tailwindcss/postcss` 自己解析出的传递依赖。

- [ ] **Step 1: 删除两个未引用文件**，并从 `src/types/index.ts` 删除：

  ```ts
  export interface OptionRecord {
    id: string;
    account_id: string;
    option_symbol: string;
    underlying: string;
    expiry_date: string;
    strike_price: number;
    option_type: "P" | "C";
    action: "SELL" | "BUY";
    code: string;
    quantity: number;
    price: number;
    amount: number;
    commission: number;
    fee: number;
    traded_at: string | null;
    settled_at: string | null;
    created_at: string;
  }
  ```

  Rust 同名结构不得改动。

- [ ] **Step 2: 从 `package.json` 的 `devDependencies` 删除唯一的直接项**：

  ```json
  "autoprefixer": "^10.5.4"
  ```

- [ ] **Step 3: 使用 Bun 正常重算锁文件**：

  ```bash
  bun install --lockfile-only
  bun install --frozen-lockfile
  ```

- [ ] **Step 4: 审查 `package.json` 与 `bun.lock` 的 diff**；只接受 `autoprefixer` 直接依赖及不再可达的锁项变化，不接受其他版本升级。

- [ ] **Step 5: 运行** `bun run build`，确认 PostCSS/Tailwind 构建仍成功。

### Task 6: 全量验证并提交

**Files:**
- Verify: all files changed in Tasks 1–5

**Interfaces:**
- Consumes: 已收敛的 Rust/TypeScript 公开边界。
- Produces: 一个可独立回退的死代码清理提交。

- [ ] **Step 1: 运行最终引用扫描**：

  ```bash
  rg -n 'get_holding_review|update_decision_quality|get_decision_statistics|get_reviewed_symbols|getSkill|convertAmount|NotesTimeline|theme-tokens' src src-tauri
  rg -n 'commands::performance::(get_performance_summary|get_return_attribution|get_monthly_returns|get_holding_performance_ranking|get_risk_metrics|get_drawdown_analysis)|commands::skills::get_skill|commands::exchange_rates::convert_amount' src-tauri/src/lib.rs
  ```

  两条命令均应无输出；若 `rg` 以 1 退出但无输出，视为通过。

- [ ] **Step 2: 运行内部消费者保护扫描**：

  ```bash
  rg -n 'performance_service::(get_performance_summary|get_return_attribution|get_monthly_returns|get_holding_performance_ranking|get_risk_metrics|get_drawdown_analysis)|skill_service::get_skill' src-tauri/src
  ```

  必须仍能看到 AI/快照/克隆或导出调用方。

- [ ] **Step 3: 运行完整质量门禁**：

  ```bash
  bun run check
  git diff --check
  ```

- [ ] **Step 4: 审查 `git diff --stat` 与完整 diff**，确认没有数据库迁移、业务公式或不相关格式化变化。

- [ ] **Step 5: 提交**：

  ```bash
  git add src src-tauri package.json bun.lock
  git commit -m "refactor: remove unused runtime surface"
  ```
