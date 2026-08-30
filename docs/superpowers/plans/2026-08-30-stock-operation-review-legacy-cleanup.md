# 股票操作复盘旧引擎退役实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 删除股票操作复盘旧复杂引擎，让轻量报告只按原始交易记录计算，同时保留截至期末的操作效果、仓位影响和市场基准比较。

**Architecture:** 先引入不接受纠错参数的原始交易动作构建器，并把轻量服务切换到独立的最小行情模块；随后移除 AI 写入口、旧数据库模式、旧后端依赖闭包和旧前端依赖闭包。现有轻量 Tauri 接口和序列化报告保持不变，已有旧数据库表不读取、不写入、不迁移、不自动删除。

**Tech Stack:** Rust 2021、Tauri 2、rusqlite、Tokio、React 19、TypeScript 7、Zustand、Node test runner、Vite。

**Spec:** `docs/superpowers/specs/2026-08-30-stock-operation-review-legacy-cleanup-design.md`

## Global Constraints

- 股票操作复盘的交易事实只能来自 `transactions` 原始记录。
- 不得读取或写入 `stock_review_overrides`、`stock_review_annotations`、`stock_market_sessions`、`stock_market_calendar_coverage`。
- 已有数据库中的旧表保持原样；不得添加 `DROP TABLE` 或隐式数据删除。
- AI 只能读取轻量报告，不得保存注释、确认纠错或回写计算输入。
- 保持 `get_stock_operation_review` 与 AI `get_stock_review` 的现有轻量报告契约。
- 保留 `stock_daily_prices`，不再为股票操作复盘建立交易日历或完整收益曲线。
- 不新增第三方依赖，不改变绩效分析、期权操作复盘和持仓复盘行为。
- 每项任务先写失败测试、确认失败，再做最小实现并提交。

## File Map

### 新建

- `src-tauri/src/services/stock_operation_builder.rs`：从原始交易记录重放持仓并生成轻量操作事实，同时提供股票和市场身份规范化函数。
- `src-tauri/src/services/stock_operation_market_data.rs`：只负责 `stock_daily_prices` 的端点行情读写和默认市场基准映射。

### 保留并修改

- `src-tauri/src/services/stock_operation_review_service.rs`：改为消费原始动作构建器和最小行情模块，删除纠错持久化依赖。
- `src-tauri/src/services/stock_operation_review_calculator.rs`：改用轻量身份规范化函数。
- `src-tauri/src/services/ai_tools.rs`：保留只读 `get_stock_review`，删除保存注释能力。
- `src-tauri/src/services/ai_chat_service.rs`：删除只为股票复盘写工具存在的上下文测试依赖。
- `src-tauri/src/services/skill_service.rs`：删除旧复杂复盘指标策略和纠错提问策略，保留技能注册与路由。
- `src-tauri/src/skills/stock-review.md`：明确原始交易唯一事实来源和 AI 只读边界。
- `src-tauri/src/commands/review.rs`、`src-tauri/src/lib.rs`：只注册轻量股票操作复盘命令。
- `src-tauri/src/db/mod.rs`、`src-tauri/src/db/tests.rs`、`src-tauri/src/commands/reset.rs`：新数据库只维护行情缓存，旧表保持惰性兼容。
- `src/pages/Review/stockOperationReviewViewModel.ts`：内聚日期范围逻辑，不再导入旧视图模型。
- `src/pages/Review/stockOperationReviewViewModel.test.ts`：承接闰年和筛选日期边界测试。
- `src/pages/AiAssistant/index.tsx`、`src/components/ai/ToolCallCard.tsx`、`src/pages/AiAssistant/prefill.test.ts`：删除股票复盘写工具文案和夹具。
- `src/types/index.ts`：只保留轻量股票操作复盘类型。
- `docs/superpowers/specs/2026-08-30-stock-operation-review-lite-design.md`：删除“旧引擎暂时保留”的过渡说明。

### 删除

- `src-tauri/src/models/stock_review.rs`
- `src-tauri/src/services/rebalance_attribution.rs`
- `src-tauri/src/services/shadow_portfolio_engine.rs`
- `src-tauri/src/services/stock_action_builder.rs`
- `src-tauri/src/services/stock_campaign_builder.rs`
- `src-tauri/src/services/stock_review_calendar.rs`
- `src-tauri/src/services/stock_review_market_data.rs`
- `src-tauri/src/services/stock_review_metrics.rs`
- `src-tauri/src/services/stock_review_persistence.rs`
- `src-tauri/src/services/stock_review_quality.rs`
- `src-tauri/src/services/stock_review_service.rs`
- `src/pages/Review/LegacyStockReviewPanel.tsx`
- `src/pages/Review/PortfolioComparisonChart.tsx`
- `src/pages/Review/RebalanceAttributionPanel.tsx`
- `src/pages/Review/RiskStructurePanel.tsx`
- `src/pages/Review/StockActionsTable.tsx`
- `src/pages/Review/StockCampaignDrawer.tsx`
- `src/pages/Review/StockReviewDataQuality.tsx`
- `src/pages/Review/StockReviewSummaryCards.tsx`
- `src/pages/Review/stockReviewViewModel.ts`
- `src/pages/Review/stockReviewViewModel.test.ts`
- `src/pages/Review/stockReviewDateBoundary.test.ts`
- `src/stores/stockReviewStore.ts`
- `src/stores/stockReviewStore.test.ts`
- 已被轻量版取代的 2026-08-28 复杂复盘和 2026-08-29 交易日历设计、计划及验证文档。

---

### Task 1: 建立原始交易动作构建器

**Files:**
- Create: `src-tauri/src/services/stock_operation_builder.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/stock_operation_builder.rs` 内嵌测试模块

**Interfaces:**
- Consumes: `crate::models::Transaction`。
- Produces: `normalize_stock_symbol(&str) -> Option<String>`、`normalize_stock_market(&str) -> Option<String>`、`stock_securities_equal(&str, &str, &str, &str) -> bool`、`build_raw_stock_operations(&[Transaction]) -> Vec<RawStockOperation>`。
- Produces: `RawStockOperation` 字段为 `action_id`、`transaction_ids`、`account_id`、`symbol`、`name`、`market`、`action_type`、`traded_at`、`trade_date`、`quantity`、`trade_price`、`trade_notional_local`、`fee_local`、`currency`、`shares_before`、`shares_after`。

- [ ] **Step 1: 写原始交易重放的失败测试**

在新模块测试中构造期初 `OPEN`、两笔同日 `BUY`、一笔 `PAY`、一笔 `SELL`，断言只生成加仓和减仓操作：

```rust
fn transaction(
    id: &str,
    transaction_type: &str,
    shares: f64,
    price: f64,
    traded_at: &str,
) -> Transaction {
    Transaction {
        id: id.to_string(),
        holding_id: None,
        account_id: "account-1".to_string(),
        symbol: "AAPL".to_string(),
        name: "Apple".to_string(),
        market: "US".to_string(),
        transaction_type: transaction_type.to_string(),
        shares,
        price,
        total_amount: shares * price,
        commission: 1.0,
        currency: "USD".to_string(),
        traded_at: traded_at.to_string(),
        notes: None,
        created_at: traded_at.to_string(),
    }
}

#[test]
fn raw_replay_groups_same_day_fills_and_ignores_non_trade_rows() {
    let rows = vec![
        transaction("opening", "OPEN", 100.0, 10.0, "2026-06-30"),
        transaction("buy-1", "BUY", 20.0, 11.0, "2026-07-02T10:00:00Z"),
        transaction("buy-2", "BUY", 30.0, 12.0, "2026-07-02T11:00:00Z"),
        transaction("dividend", "PAY", 1.0, 5.0, "2026-07-03"),
        transaction("sell", "SELL", 50.0, 13.0, "2026-07-10T10:00:00Z"),
    ];

    let actions = build_raw_stock_operations(&rows);
    assert_eq!(actions.len(), 2);
    assert_eq!(actions[0].action_type, "add");
    assert_eq!(
        actions[0].transaction_ids,
        vec!["buy-1".to_string(), "buy-2".to_string()],
    );
    assert_eq!(actions[0].quantity, 50.0);
    assert!((actions[0].trade_price - 11.6).abs() < 1e-12);
    assert_eq!((actions[0].shares_before, actions[0].shares_after), (100.0, 150.0));
    assert_eq!(actions[1].action_type, "reduce");
    assert_eq!((actions[1].shares_before, actions[1].shares_after), (150.0, 100.0));
}
```

再加入测试确认现金代码、空日期和无前置持仓的卖出不产生推断操作；大小写不同的同一股票代码和市场仍属于同一持仓。

- [ ] **Step 2: 运行测试并确认缺少新模块或函数而失败**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_builder -- --nocapture
```

Expected: FAIL，提示 `stock_operation_builder` 或 `build_raw_stock_operations` 尚不存在。

- [ ] **Step 3: 实现不接受纠错参数的最小构建器**

核心接口必须保持如下形状：

```rust
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct RawStockOperation {
    pub action_id: String,
    pub transaction_ids: Vec<String>,
    pub account_id: String,
    pub symbol: String,
    pub name: String,
    pub market: String,
    pub action_type: String,
    pub traded_at: String,
    pub trade_date: NaiveDate,
    pub quantity: f64,
    pub trade_price: f64,
    pub trade_notional_local: f64,
    pub fee_local: f64,
    pub currency: String,
    pub shares_before: f64,
    pub shares_after: f64,
}

pub(crate) fn build_raw_stock_operations(
    transactions: &[Transaction],
) -> Vec<RawStockOperation> {
    // 排序键只能使用 traded_at、created_at、id；不得接收 override 或 AI 输入。
    // OPEN 只建立期初股数，BUY/SELL 才生成操作，PAY/现金代码不生成操作。
    // 同账户、规范化股票、规范化市场、同日、同方向的成交合并。
}
```

金额加权均价使用 `sum(shares * price) / sum(shares)`，成交金额优先汇总原始 `total_amount` 的绝对值，费用汇总原始 `commission`。卖出导致股数为负或缺少前置多头持仓时不推断操作。

- [ ] **Step 4: 运行构建器测试并确认通过**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_builder -- --nocapture
```

Expected: PASS，覆盖四种动作、同日合并、期初重放、非买卖排除和规范化身份。

- [ ] **Step 5: 提交原始动作构建器**

```bash
git add src-tauri/src/services/stock_operation_builder.rs src-tauri/src/services/mod.rs
git commit -m "refactor(review): build operations from raw trades"
```

---

### Task 2: 将轻量服务切换到原始交易唯一事实来源

**Files:**
- Modify: `src-tauri/src/services/stock_operation_review_service.rs`
- Modify: `src-tauri/src/services/stock_operation_review_calculator.rs`
- Test: `src-tauri/src/services/stock_operation_review_service.rs` 内嵌测试模块

**Interfaces:**
- Consumes: Task 1 的 `build_raw_stock_operations` 与身份规范化函数。
- Produces: `project_action_seeds(&[Transaction], &HashMap<String, String>, &StockOperationReviewQuery) -> Vec<StockOperationEffect>`；不再接受 `StockReviewOverride`。
- Preserves: `get_stock_operation_review`、`get_stock_operation_review_with_refresh`、`StockOperationReviewReport` 序列化契约。

- [ ] **Step 1: 把服务测试改成无纠错参数，并增加旧纠错不影响结果的失败测试**

先把调用改为：

```rust
let actions = project_action_seeds(&transactions, &names, &query());
```

增加集成测试，在内存数据库中手工建立旧表并写入 `non_trade` 覆盖，再调用轻量服务：

```rust
fn seeded_operation_db() -> Database {
    let db = Database::new(":memory:").unwrap();
    let conn = db.conn.lock().unwrap();
    conn.execute(
        "INSERT INTO accounts (id, name, market, created_at, updated_at)
         VALUES ('account-1', '主账户', 'US', '2026-01-01', '2026-01-01')",
        [],
    ).unwrap();
    conn.execute(
        "INSERT INTO transactions
            (id, account_id, symbol, name, market, transaction_type, shares,
             price, total_amount, commission, currency, traded_at, created_at)
         VALUES
            ('buy', 'account-1', 'AAPL', 'Apple', 'US', 'BUY', 10,
             100, 1000, 1, 'USD', '2026-07-03T10:00:00Z', '2026-07-03T10:00:00Z')",
        [],
    ).unwrap();
    drop(conn);
    db
}

#[tokio::test]
async fn legacy_override_rows_never_change_raw_operation_results() {
    let db = seeded_operation_db();
    db.conn.lock().unwrap().execute_batch(
        "CREATE TABLE IF NOT EXISTS stock_review_overrides (
            id TEXT PRIMARY KEY,
            override_type TEXT NOT NULL,
            transaction_ids_json TEXT NOT NULL,
            value_json TEXT NOT NULL,
            created_at TEXT NOT NULL,
            updated_at TEXT NOT NULL
         );
         DELETE FROM stock_review_overrides;
         INSERT INTO stock_review_overrides VALUES (
            'legacy', 'non_trade', '[\"buy\"]', '{}', '2026-07-03', '2026-07-03'
         );",
    ).unwrap();

    let report = get_stock_operation_review_with_refresh(&db, query(), false)
        .await
        .unwrap();
    assert_eq!(report.actions.len(), 1);
    assert_eq!(report.actions[0].transaction_ids, vec!["buy".to_string()]);
}
```

`seeded_operation_db()` 在测试模块中插入账户、一笔 `BUY` 原始交易和所需缓存行情；不得调用旧持久化服务。

- [ ] **Step 2: 运行轻量服务测试并确认旧签名或旧覆盖行为导致失败**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review_service -- --nocapture
```

Expected: FAIL，原因是 `project_action_seeds` 仍要求 overrides，或服务仍读取旧表。

- [ ] **Step 3: 切换轻量服务和汇总器**

在 `project_action_seeds` 中调用：

```rust
let actions = build_raw_stock_operations(transactions)
    .into_iter()
    .filter(|action| action.trade_date >= query.start_date && action.trade_date <= query.end_date)
    .filter(|action| query.account_id.as_ref().is_none_or(|id| id == &action.account_id))
    .filter(|action| query.market.as_ref().is_none_or(|market| {
        normalize_stock_market(market) == normalize_stock_market(&action.market)
    }));
```

直接从 `RawStockOperation` 投影 `StockOperationEffect`，删除 `StockReviewOverride`、`build_stock_actions`、`stock_review_persistence::list_overrides` 和 `transfer` 标签过滤。计算器的证券聚合改用 Task 1 的规范化函数。

- [ ] **Step 4: 运行轻量服务与计算器测试**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review -- --nocapture
```

Expected: PASS；旧纠错表存在或不存在都不影响原始操作结果。

- [ ] **Step 5: 提交事实来源切换**

```bash
git add src-tauri/src/services/stock_operation_review_service.rs src-tauri/src/services/stock_operation_review_calculator.rs
git commit -m "refactor(review): ignore legacy trade overrides"
```

---

### Task 3: 抽出轻量端点行情模块

**Files:**
- Create: `src-tauri/src/services/stock_operation_market_data.rs`
- Modify: `src-tauri/src/services/stock_operation_review_service.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/stock_operation_market_data.rs` 内嵌测试模块

**Interfaces:**
- Consumes: `Database`、`stock_daily_prices` 和绩效分析现有基准读取能力。
- Produces: `DailyMarketPoint`、`upsert_stock_closes`、`load_stock_price_series`、`default_benchmark_symbol`。
- Must not consume: 交易日历、覆盖率、调整收盘价模式、Campaign 观察窗口或旧质量状态。

- [ ] **Step 1: 写只依赖真实缓存行的失败测试**

```rust
fn date(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
}

#[test]
fn endpoint_cache_reads_only_real_rows_in_requested_range() {
    let db = Database::new(":memory:").unwrap();
    upsert_stock_closes(
        &db,
        "001248",
        "CN",
        "test",
        &[(date("2026-07-02"), 20.0), (date("2026-07-31"), 24.0)],
    ).unwrap();

    let points = load_stock_price_series(
        &db,
        "001248",
        "CN",
        date("2026-06-30"),
        date("2026-07-31"),
    ).unwrap();
    assert_eq!(points.iter().map(|point| point.date).collect::<Vec<_>>(), [
        date("2026-07-02"),
        date("2026-07-31"),
    ]);
}
```

同时断言默认基准为 US `^GSPC`、CN `000300.SS`、HK `^HSI`，且模块不需要日历表即可工作。

- [ ] **Step 2: 运行测试并确认新模块不存在而失败**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_market_data -- --nocapture
```

Expected: FAIL，提示新模块或接口不存在。

- [ ] **Step 3: 实现最小行情模块并切换轻量服务导入**

`DailyMarketPoint` 只保留轻量端点计算需要的字段：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub(crate) struct DailyMarketPoint {
    pub date: NaiveDate,
    pub close: f64,
}
```

SQL 继续写入现有 `stock_daily_prices`，但读接口只选择 `date, close` 并按日期排序；不查询 `stock_market_sessions` 或 `stock_market_calendar_coverage`。轻量服务完全改用新模块。

- [ ] **Step 4: 运行行情与轻量服务测试**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_market_data -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib stock_operation_review_service -- --nocapture
```

Expected: PASS；新股上市前没有价格时保留真实上市后端点，不构造缺失交易日。

- [ ] **Step 5: 提交轻量行情边界**

```bash
git add src-tauri/src/services/stock_operation_market_data.rs src-tauri/src/services/stock_operation_review_service.rs src-tauri/src/services/mod.rs
git commit -m "refactor(review): isolate endpoint market data"
```

---

### Task 4: 将股票复盘 AI 改成纯只读

**Files:**
- Modify: `src-tauri/src/services/ai_tools.rs`
- Modify: `src-tauri/src/services/ai_chat_service.rs`
- Modify: `src-tauri/src/services/skill_service.rs`
- Modify: `src-tauri/src/skills/stock-review.md`
- Test: 上述 Rust 文件内嵌测试模块

**Interfaces:**
- Consumes: `stock_operation_review_service::get_stock_operation_review_with_refresh` 和 `scope_report_to_symbol`。
- Preserves: AI 工具 `get_stock_review` 的参数与轻量报告返回值。
- Removes: `save_stock_review_annotation`、`ConfirmedAiAnnotationCapability`、`ToolCtx.stock_review_annotation_confirmation`。

- [ ] **Step 1: 把 AI 工具契约测试改成只读失败测试**

```rust
#[test]
fn stock_review_exposes_one_read_only_tool() {
    let definitions = tool_definitions();
    let names = definitions
        .iter()
        .filter_map(|tool| tool["function"]["name"].as_str())
        .collect::<Vec<_>>();
    assert!(names.contains(&"get_stock_review"));
    assert!(!names.contains(&"save_stock_review_annotation"));
}
```

更新现有工具执行测试，断言返回 JSON 的 `deterministic_source` 为 `stock_operation_review_service`。把 `ai_chat_service` 中用于错误工具名的夹具改为无关只读工具名，不再使用被删除的保存工具。

- [ ] **Step 2: 运行 AI 与技能测试并确认仍暴露写工具而失败**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib services::ai_tools::tests::stock_review -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib services::skill_service::tests::stock_review -- --nocapture
```

Expected: FAIL，显示 `save_stock_review_annotation` 仍在定义、派发或技能说明中。

- [ ] **Step 3: 删除 AI 写入能力和旧复杂指标策略**

从 `ai_tools.rs` 删除：

- `save_stock_review_annotation` JSON schema。
- 对应 dispatch 分支与 handler。
- 确认 capability 类型、`ToolCtx` 字段、草稿绑定和回放测试。
- 对 `models::stock_review` 和 `stock_review_service` 的导入。

把仍需规范化股票代码的调用改为 `stock_operation_builder::normalize_stock_symbol`。从 `skill_service.rs` 删除只为旧结果质量、影子调仓、60/120 日窗口、风险结构、Campaign 纠错存在的枚举、策略和测试，保留 `stock-review` 技能解析、触发和路由。将技能正文中的保存注释段落替换为：

```markdown
股票操作复盘是只读分析。交易事实只来自原始交易记录；AI 不保存注释、不确认纠错，也不修改任何确定性指标。确实缺少投资逻辑时，只在当前回答中提出至多三个可跳过的问题。
```

- [ ] **Step 4: 运行 AI、聊天与技能测试**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib services::ai_tools -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib services::ai_chat_service -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib services::skill_service -- --nocapture
```

Expected: PASS；工具列表只有股票复盘只读入口，且股票复盘技能仍能正确路由。

- [ ] **Step 5: 提交 AI 只读化**

```bash
git add src-tauri/src/services/ai_tools.rs src-tauri/src/services/ai_chat_service.rs src-tauri/src/services/skill_service.rs src-tauri/src/skills/stock-review.md
git commit -m "refactor(review): make stock review AI read only"
```

---

### Task 5: 删除旧后端引擎、命令和数据库模式

**Files:**
- Modify: `src-tauri/src/commands/review.rs`
- Modify: `src-tauri/src/commands/reset.rs`
- Modify: `src-tauri/src/db/mod.rs`
- Modify: `src-tauri/src/db/tests.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/models/mod.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Delete: `src-tauri/src/models/stock_review.rs`
- Delete: `src-tauri/src/services/rebalance_attribution.rs`
- Delete: `src-tauri/src/services/shadow_portfolio_engine.rs`
- Delete: `src-tauri/src/services/stock_action_builder.rs`
- Delete: `src-tauri/src/services/stock_campaign_builder.rs`
- Delete: `src-tauri/src/services/stock_review_calendar.rs`
- Delete: `src-tauri/src/services/stock_review_market_data.rs`
- Delete: `src-tauri/src/services/stock_review_metrics.rs`
- Delete: `src-tauri/src/services/stock_review_persistence.rs`
- Delete: `src-tauri/src/services/stock_review_quality.rs`
- Delete: `src-tauri/src/services/stock_review_service.rs`
- Test: `src-tauri/src/db/tests.rs`、`src-tauri/src/commands/review.rs`

**Interfaces:**
- Preserves: `get_stock_operation_review`、`get_holding_review`、`update_decision_quality`、`get_decision_statistics`、`get_reviewed_symbols`。
- Removes: `get_stock_review_report`、`get_stock_campaign_detail`、`save_stock_review_annotation`、`confirm_stock_review_override`。
- Produces: `clear_stock_operation_review_cache(&Transaction<'_>) -> SqlResult<()>`，只清除 `stock_daily_prices`。

- [ ] **Step 1: 先写新数据库模式和惰性旧表兼容的失败测试**

把旧数据库测试替换为：

```rust
#[test]
fn stock_operation_review_creates_only_the_price_cache() {
    let db = create_test_db();
    let conn = db.conn.lock().unwrap();
    let price_cache: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='stock_daily_prices'",
        [],
        |row| row.get(0),
    ).unwrap();
    let legacy_tables: i64 = conn.query_row(
        "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name IN (
            'stock_market_sessions', 'stock_market_calendar_coverage',
            'stock_review_annotations', 'stock_review_overrides'
        )",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(price_cache, 1);
    assert_eq!(legacy_tables, 0);
}

#[test]
fn reset_clears_price_cache_but_leaves_an_existing_legacy_table_inert() {
    let db = create_test_db();
    let mut conn = db.conn.lock().unwrap();
    conn.execute_batch(
        "INSERT INTO stock_daily_prices
           (symbol, market, date, close, source, updated_at)
         VALUES ('AAPL', 'US', '2026-07-31', 200, 'test', '2026-07-31');
         CREATE TABLE stock_review_overrides (id TEXT PRIMARY KEY);
         INSERT INTO stock_review_overrides VALUES ('legacy');",
    ).unwrap();
    let tx = conn.transaction().unwrap();
    clear_stock_operation_review_cache(&tx).unwrap();
    tx.commit().unwrap();
    let cached_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM stock_daily_prices",
        [],
        |row| row.get(0),
    ).unwrap();
    let inert_rows: i64 = conn.query_row(
        "SELECT COUNT(*) FROM stock_review_overrides",
        [],
        |row| row.get(0),
    ).unwrap();
    assert_eq!(cached_rows, 0);
    assert_eq!(inert_rows, 1);
}
```

命令测试只保留 `stock_operation_query` 的格式、账户、市场和币种边界。

- [ ] **Step 2: 运行数据库和命令测试并确认旧表仍创建而失败**

Run:

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib db::tests::stock_operation_review -- --nocapture
cargo test --manifest-path src-tauri/Cargo.toml --lib commands::review -- --nocapture
```

Expected: FAIL，旧表仍由迁移创建，旧命令仍在注册。

- [ ] **Step 3: 删除旧命令、旧表创建和旧模块注册**

`commands/review.rs` 只保留轻量 query/command 和其他非股票复杂复盘命令；删除快照回填、旧 query、注释规范化及其测试。`lib.rs` 从 invoke handler 中删除四个旧命令。

`db/mod.rs` 删除四张旧表的 `CREATE TABLE`、索引和 `ALTER TABLE stock_review_overrides` 迁移。`commands/reset.rs` 将清理函数改为：

```rust
pub(crate) fn clear_stock_operation_review_cache(
    tx: &Transaction<'_>,
) -> SqlResult<()> {
    tx.execute("DELETE FROM stock_daily_prices", [])?;
    Ok(())
}
```

从 `models/mod.rs` 和 `services/mod.rs` 移除旧模块声明，然后删除文件清单中的全部旧 Rust 模块。不得添加旧命令兼容壳或旧表探测逻辑。

- [ ] **Step 4: 格式化并运行后端全套测试和编译检查**

Run:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: PASS；测试总数因删除旧引擎专属测试而下降，但轻量、绩效、期权和其他模块测试全部通过。

- [ ] **Step 5: 提交后端退役**

```bash
git add src-tauri/src
git commit -m "refactor(review): retire legacy stock review engine"
```

---

### Task 6: 删除旧前端界面、状态和类型

**Files:**
- Modify: `src/pages/Review/stockOperationReviewViewModel.ts`
- Modify: `src/pages/Review/stockOperationReviewViewModel.test.ts`
- Modify: `src/pages/AiAssistant/index.tsx`
- Modify: `src/components/ai/ToolCallCard.tsx`
- Modify: `src/pages/AiAssistant/prefill.test.ts`
- Modify: `src/types/index.ts`
- Delete: `src/pages/Review/LegacyStockReviewPanel.tsx`
- Delete: `src/pages/Review/PortfolioComparisonChart.tsx`
- Delete: `src/pages/Review/RebalanceAttributionPanel.tsx`
- Delete: `src/pages/Review/RiskStructurePanel.tsx`
- Delete: `src/pages/Review/StockActionsTable.tsx`
- Delete: `src/pages/Review/StockCampaignDrawer.tsx`
- Delete: `src/pages/Review/StockReviewDataQuality.tsx`
- Delete: `src/pages/Review/StockReviewSummaryCards.tsx`
- Delete: `src/pages/Review/stockReviewViewModel.ts`
- Delete: `src/pages/Review/stockReviewViewModel.test.ts`
- Delete: `src/pages/Review/stockReviewDateBoundary.test.ts`
- Delete: `src/stores/stockReviewStore.ts`
- Delete: `src/stores/stockReviewStore.test.ts`

**Interfaces:**
- Preserves: `StockOperationReviewReport`、`StockOperationEffect`、`StockOperationSecuritySummary`、`StockActionType`、`StockReviewPeriodPreset` 和现有轻量 store/page。
- Removes: `StockReviewReport`、Campaign、旧纠错、旧注释、旧质量和旧组合曲线类型。
- Produces: `getStockOperationReviewDateRange` 内部完整实现，不再委托旧 `getStockReviewDateRange`。

- [ ] **Step 1: 把日期边界测试迁移到轻量视图模型并增加写工具不存在断言**

在 `stockOperationReviewViewModel.test.ts` 加入：

```ts
test("one-year preset preserves leap-day calendar semantics", () => {
  assert.deepEqual(
    getStockOperationReviewDateRange("1Y", new Date("2024-02-29T23:30:00+08:00")),
    { startDate: "2023-03-01", endDate: "2024-02-29" },
  );
});
```

在 `prefill.test.ts` 删除保存注释夹具，并断言股票复盘预填只请求 `get_stock_review`。

- [ ] **Step 2: 运行前端聚焦测试并确认日期函数仍依赖旧文件**

Run:

```bash
node --test src/pages/Review/stockOperationReviewViewModel.test.ts src/stores/stockOperationReviewStore.test.ts src/pages/AiAssistant/prefill.test.ts
```

Expected: FAIL，直到轻量日期函数完全内聚且旧写工具夹具被删除。

- [ ] **Step 3: 内聚轻量日期逻辑并删除旧前端依赖闭包**

把季度、本年、过去一年和自定义区间计算直接移入 `getStockOperationReviewDateRange`，保持现有本地日期格式化和闰年语义。删除旧组件、旧 store 和旧测试。

从 `src/types/index.ts` 删除 `StockReviewFilters` 起至 `StockReviewOverrideInput` 的旧复杂类型，但保留轻量报告使用的 `StockActionType`、`StockReviewPeriodPreset` 和所有 `StockOperation*` 类型。删除 AI 两处 `save_stock_review_annotation` 中文标签。

- [ ] **Step 4: 运行全部前端测试和生产构建**

Run:

```bash
node --test
npm run build
```

Expected: PASS；股票操作复盘页面只使用轻量 store、轻量类型和只读 AI 入口。

- [ ] **Step 5: 提交前端退役**

```bash
git add src
git commit -m "refactor(review): remove legacy stock review UI"
```

---

### Task 7: 删除过时文档并执行最终源码审计

**Files:**
- Modify: `docs/superpowers/specs/2026-08-30-stock-operation-review-lite-design.md`
- Delete: `docs/superpowers/plans/2026-08-28-stock-operation-review-redesign.md`
- Delete: `docs/superpowers/plans/2026-08-29-stock-review-market-calendar.md`
- Delete: `docs/superpowers/specs/2026-08-28-stock-operation-review-redesign.md`
- Delete: `docs/superpowers/specs/2026-08-29-stock-review-market-calendar-design.md`
- Delete: `docs/superpowers/verification/2026-08-29-stock-review-market-calendar.md`
- Verify: entire repository

**Interfaces:**
- Preserves: 本计划、退役设计、轻量版设计和轻量版实施计划。
- Produces: 源码中没有旧引擎、旧命令、旧表访问或 AI 股票复盘写工具的可执行引用。

- [ ] **Step 1: 先执行旧标识审计并记录预期残留**

Run each command separately:

```bash
rg -n "get_stock_review_report|get_stock_campaign_detail|save_stock_review_annotation|confirm_stock_review_override" src src-tauri/src
rg -n "stock_review_service|stock_review_persistence|stock_review_calendar|shadow_portfolio_engine|stock_campaign_builder|rebalance_attribution" src src-tauri/src
rg -n "LegacyStockReviewPanel|stockReviewStore|stockReviewViewModel" src
```

Expected before cleanup completion: any output identifies a remaining executable reference that must be removed; documentation strings are handled in Step 2.

- [ ] **Step 2: 删除被取代的历史文档并更新轻量设计**

从轻量设计删除“旧复杂引擎暂时保留用于回退”的段落，改为链接本退役设计：

```markdown
旧复杂股票复盘引擎已按 `2026-08-30-stock-operation-review-legacy-cleanup-design.md` 退役。轻量页面、刷新流程和 AI 股票复盘入口只调用原始交易轻量链路。
```

删除文件清单中的旧复杂复盘和交易日历文档；不要删除本计划、退役设计或轻量版文档。

- [ ] **Step 3: 运行无残留源码审计**

重新运行 Step 1 三条 `rg`。Expected: 均无输出并以 exit code 1 结束。随后运行：

```bash
rg -n "StockReviewReport|StockReviewOverride|StockReviewAnnotation|StockCampaign" src src-tauri/src
rg -n "stock_review_overrides|stock_review_annotations|stock_market_sessions|stock_market_calendar_coverage" src src-tauri/src
```

Expected: 第一条无旧股票复杂复盘类型引用；期权的通用 Campaign 命名不在本次检索词中。第二条只允许命中 `stock_operation_review_service.rs` 和 `db/tests.rs` 中 `#[cfg(test)]` 保护的惰性旧表兼容测试，不得命中非测试运行时代码，也不得出现 `stock_review_annotations`、`stock_market_sessions` 或 `stock_market_calendar_coverage`。

- [ ] **Step 4: 执行最终格式、测试、构建和差异检查**

Run:

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml --all --check
cargo test --manifest-path src-tauri/Cargo.toml --lib
cargo check --manifest-path src-tauri/Cargo.toml
node --test
npm run build
git diff --check
```

Expected: 全部 PASS。记录 Rust 和前端最终测试数量；确认只有既有、已知的构建警告，没有新错误。

- [ ] **Step 5: 提交文档清理和最终审计结果**

```bash
git add docs
git commit -m "docs(review): retire legacy stock review docs"
```

- [ ] **Step 6: 检查最终提交序列与工作区**

Run:

```bash
git status --short --branch
git log --oneline -10
```

Expected: 工作区干净；提交序列依次包含原始动作构建器、事实来源切换、轻量行情、AI 只读化、后端退役、前端退役和文档退役。
