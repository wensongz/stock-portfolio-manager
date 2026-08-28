# 股票操作复盘重做第一版 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把股票操作复盘改造成默认零填写、组合优先的确定性报告，用五项核心指标评价中长期投资者的结果质量与整体调仓，并让页面与 AI 共用同一份 Rust 报告。

**Architecture:** Rust 按“交易标准化 → StockAction → StockCampaign → 行情与汇率 → 实际/影子/基准曲线 → 指标与归因 → 数据质量”流水线生成唯一报告；Tauri 页面命令和 AI 工具只调用该服务。React 默认按今年以来自动加载报告，负责筛选、展示、钻取与确认纠正，不重新计算财务指标。原有手工季度决策复盘折叠保留，作为历史入口而非主流程。

**Tech Stack:** Tauri 2、Rust、rusqlite、chrono、serde、React 19、TypeScript 7、Ant Design 6、Zustand、ECharts、Node test runner。

**Spec:** `docs/superpowers/specs/2026-08-28-stock-operation-review-redesign.md`

## Global Constraints

- 第一版只服务中长期股票投资者；不做日内执行质量、因子风险模型、税务归因、最优组合建议和综合评分。
- 默认不向用户索要信息；只有数据问题会显著改变计算时，才通过数据质量区或 AI 最多提出三个高价值问题。
- Rust 是指标、状态和事实标签的唯一计算源；React 和 AI 不得重算 TWR、影子收益、调仓增益、前瞻效果或 Campaign 损益。
- 实际组合 TWR 必须复用 `performance_service::build_twr_return_series` 的现金流时点约定。
- 状态只使用 `available`、`degraded`、`pending`、`unavailable`；缺失数据不能用 0 填充。
- 自动混合基准使用期初固定市场权重；默认美国、A 股、港股基准分别为 `^GSPC`、`000300.SS`、`^HSI`，现金权重收益为 0。
- 影子组合复制期初持仓和现金，只重放外部现金流、汇率、拆股、分红和现金收益，忽略股票交易。
- 有可靠总回报数据时使用总回报模式；否则实际/影子对比同时降为价格模式，真实组合结果卡仍展示记录口径的实际 TWR。
- 前瞻效果以标的本地币种相对所属市场基准计算，主窗口为 60 个交易日，验证窗口为 120 个交易日，20 日只在详情展示。
- 注释与计算纠正分开存储；纠正必须经用户明确确认，且先预演可生成报告，再持久化并返回新报告。
- 原始重复交易污染实际业绩时，在源记录修复前，影子增益和归因必须为 `unavailable`，不能以纠正后的动作悄悄替换账本实际值。
- 不新增 npm 或 Cargo 依赖。

## File Map

**Create — Rust**

- `src-tauri/src/models/stock_review.rs`：报告、指标、动作、Campaign、质量、注释与纠正的序列化契约。
- `src-tauri/src/services/stock_action_builder.rs`：交易排序、合并、持仓重放和 StockAction 分类。
- `src-tauri/src/services/stock_campaign_builder.rs`：账户 Campaign 片段、逻辑 Campaign 和转仓连接。
- `src-tauri/src/services/stock_review_market_data.rs`：股票/基准日线缓存、交易日窗口和覆盖率。
- `src-tauri/src/services/shadow_portfolio_engine.rs`：不调仓影子组合纯计算。
- `src-tauri/src/services/rebalance_attribution.rs`：实际与影子持仓差异归因。
- `src-tauri/src/services/stock_review_metrics.rs`：五项核心指标及 Campaign 指标。
- `src-tauri/src/services/stock_review_quality.rs`：问题聚合、状态阈值和口径元数据。
- `src-tauri/src/services/stock_review_persistence.rs`：注释与纠正读写。
- `src-tauri/src/services/stock_review_service.rs`：异步数据准备和确定性报告编排。
- `src-tauri/src/skills/stock-review.md`：AI 股票操作复盘工作流。

**Create — React**

- `src/stores/stockReviewStore.ts`：报告、Campaign 详情、注释和纠正状态。
- `src/stores/stockReviewStore.test.ts`：竞态、错误与刷新行为测试。
- `src/pages/Review/stockReviewViewModel.ts`：默认周期、筛选持久化、格式化和 AI 预填参数。
- `src/pages/Review/stockReviewViewModel.test.ts`：前端纯函数测试。
- `src/pages/Review/StockReviewFilters.tsx`：账户、周期、市场、基准和币种筛选。
- `src/pages/Review/StockReviewDataQuality.tsx`：覆盖率摘要、局部问题和口径说明。
- `src/pages/Review/StockReviewSummaryCards.tsx`：五项核心指标。
- `src/pages/Review/PortfolioComparisonChart.tsx`：实际、影子、基准曲线。
- `src/pages/Review/RebalanceAttributionPanel.tsx`：贡献、机会损失和风险结构。
- `src/pages/Review/RiskStructurePanel.tsx`：集中度、现金、换手、费用和 HHI 详情。
- `src/pages/Review/StockActionsTable.tsx`：全部调仓动作。
- `src/pages/Review/StockCampaignDrawer.tsx`：单股 Campaign 详情、MAE/MFE、注释和纠正。
- `src/pages/Review/LegacyStockReviewPanel.tsx`：旧手工决策复盘折叠入口。

**Modify**

- `src-tauri/src/db/mod.rs`、`src-tauri/src/db/tests.rs`、`src-tauri/src/commands/reset.rs`：新增缓存及复盘表并覆盖重置。
- `src-tauri/src/models/mod.rs`、`src-tauri/src/services/mod.rs`：注册新模块。
- `src-tauri/src/commands/review.rs`、`src-tauri/src/lib.rs`：注册页面命令。
- `src-tauri/src/services/ai_tools.rs`：注册读取报告和确认后保存注释的 AI 工具。
- `src-tauri/src/services/skill_service.rs`：注册股票复盘 Skill 并升级内置版本。
- `docs/ai-tools.md`：记录新工具边界与参数。
- `src/types/index.ts`：加入与 Rust 一一对应的 TypeScript 类型。
- `src/pages/Review/StockReviewTab.tsx`：改成组合报告容器。
- `src/pages/AiAssistant/index.tsx`、`src/components/ai/ToolCallCard.tsx`：接收复盘预填并显示中文工具名。
- `src/pages/Settings/GeneralSettings.tsx`：清理应用数据时覆盖股票复盘筛选键。

---

### Task 1: 建立报告契约与数据库迁移

**Files:**
- Create: `src-tauri/src/models/stock_review.rs`
- Modify: `src-tauri/src/models/mod.rs`
- Modify: `src-tauri/src/db/mod.rs`
- Modify: `src-tauri/src/db/tests.rs`
- Modify: `src-tauri/src/commands/reset.rs`
- Test: `src-tauri/src/db/tests.rs`

**Interfaces:**
- Consumes: 现有 `Database`、`transactions`、`stock_splits`、`daily_portfolio_values`、`daily_holding_snapshots`、账户及汇率字段。
- Produces: `StockReviewQuery`、`StockReviewMethodology`、`StockReviewReport`、`StockReviewSummary`、`ReviewCurvePoint`、`RebalanceAttributionSummary`、`RiskStructureDetail`、`StockActionReview`、`StockCampaignSummary`、`StockCampaignDetail`、`StockReviewDataQuality`、`StockReviewIssue`、`StockReviewAnnotation`、`StockReviewAnnotationInput`、`StockReviewOverride`、`StockReviewOverrideInput`。
- Produces: `stock_daily_prices`、`stock_review_annotations`、`stock_review_overrides` 三张持久化表。

- [ ] **Step 1: 先写迁移失败测试**

在 `src-tauri/src/db/tests.rs` 增加 `stock_review_tables_are_created_and_resettable`，打开内存数据库后查询 `sqlite_master`，要求三张表存在；再向每张表插入一条合法记录，为后续 reset 测试准备夹具。测试应明确检查 `stock_daily_prices` 的 `(symbol, market, date)` 唯一键，以及注释和纠正表的 `id` 主键。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd src-tauri && cargo test --lib db::tests::stock_review_tables_are_created_and_resettable -- --nocapture`

Expected: FAIL，查询 `sqlite_master` 时找不到 `stock_daily_prices`、`stock_review_annotations` 或 `stock_review_overrides`。

- [ ] **Step 3: 添加迁移与重置范围**

在 `src-tauri/src/db/mod.rs` 现有内联迁移尾部创建：

```sql
CREATE TABLE IF NOT EXISTS stock_daily_prices (
  symbol TEXT NOT NULL,
  market TEXT NOT NULL,
  date TEXT NOT NULL,
  open REAL,
  high REAL,
  low REAL,
  close REAL NOT NULL,
  volume REAL,
  adjusted_close REAL,
  dividend REAL,
  source TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  PRIMARY KEY (symbol, market, date)
);

CREATE TABLE IF NOT EXISTS stock_review_annotations (
  id TEXT PRIMARY KEY,
  scope_type TEXT NOT NULL CHECK(scope_type IN ('period','stock','campaign','action')),
  scope_key TEXT NOT NULL,
  account_id TEXT,
  symbol TEXT,
  annotation_type TEXT NOT NULL,
  value_json TEXT NOT NULL,
  source TEXT NOT NULL CHECK(source IN ('user','ai_confirmed')),
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS stock_review_overrides (
  id TEXT PRIMARY KEY,
  override_type TEXT NOT NULL CHECK(override_type IN ('transfer','duplicate','same_day_order','non_trade')),
  transaction_ids_json TEXT NOT NULL,
  value_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);
```

在 `src-tauri/src/commands/reset.rs` 的业务数据清理集合加入三张表，并让测试执行 reset 后断言三张表行数都为 0。

- [ ] **Step 4: 定义 Rust 报告模型并注册模块**

在 `stock_review.rs` 定义以下稳定边界：

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MetricStatus { Available, Degraded, Pending, Unavailable }

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricAvailability {
    pub status: MetricStatus,
    pub note: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewQuery {
    pub start_date: NaiveDate,
    pub end_date: NaiveDate,
    pub account_id: Option<String>,
    pub market: Option<String>,
    pub benchmark_symbol: Option<String>,
    pub base_currency: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewReport {
    pub methodology: StockReviewMethodology,
    pub summary: StockReviewSummary,
    pub curves: Vec<ReviewCurvePoint>,
    pub attribution: RebalanceAttributionSummary,
    pub risk_structure: RiskStructureDetail,
    pub actions: Vec<StockActionReview>,
    pub campaigns: Vec<StockCampaignSummary>,
    pub data_quality: StockReviewDataQuality,
    pub annotations: Vec<StockReviewAnnotation>,
    pub generated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct StockReviewSummary {
    pub result_quality: ResultQualityMetric,
    pub max_drawdown: MaxDrawdownMetric,
    pub rebalance_value_add: RebalanceValueAddMetric,
    pub forward_effect: ForwardEffectMetric,
    pub risk_structure: RiskStructureMetric,
}
```

`StockReviewMethodology` 固定包含原始筛选、实际/影子/基准收益模式、固定权重及基准符号、行情/汇率覆盖和算法版本。五项 summary 每项自带 `MetricAvailability`；`ForwardEffectMetric` 固定返回 60/120 日窗口，Campaign 详情额外返回 20 日窗口。`StockActionReview` 必须含操作前后股数/组合权重、费用、贡献、观察窗口、状态和事实标签；所有不可用数字使用 `Option<f64>`，不能序列化成 0。

在 `models/mod.rs` 导出 `pub mod stock_review;`。

- [ ] **Step 5: 验证迁移、序列化命名和 reset**

在测试中把 `MetricStatus::Available` 序列化为 JSON，并断言值为 `"available"`；对新表完成 insert、upsert 和 reset 断言。

Run: `cd src-tauri && cargo test --lib db::tests::stock_review -- --nocapture`

Expected: PASS，三张表存在、唯一键生效、状态序列化为小写、reset 清空数据。

- [ ] **Step 6: 提交契约和迁移**

```bash
git add src-tauri/src/models/stock_review.rs src-tauri/src/models/mod.rs src-tauri/src/db/mod.rs src-tauri/src/db/tests.rs src-tauri/src/commands/reset.rs
git commit -m "feat: add stock review data contracts"
```

---

### Task 2: 从交易记录构建 StockAction

**Files:**
- Create: `src-tauri/src/services/stock_action_builder.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/stock_action_builder.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: `Vec<Transaction>`、拆股后的有效持仓数量、`Vec<StockReviewOverride>`。
- Produces: `pub fn build_stock_actions(transactions: &[Transaction], overrides: &[StockReviewOverride]) -> ActionBuildResult`。
- Produces: 合并后的 `StockActionReview`、用于 Campaign 的 `PositionEvent`、顺序不确定/疑似重复/负持仓等 `StockReviewIssue`。

- [ ] **Step 1: 为动作合并与分类写失败测试**

覆盖四条独立测试：

```rust
#[test]
fn merges_same_day_same_direction_fills_with_weighted_price() { /* 40@100 + 60@110 => 100@106 */ }

#[test]
fn classifies_open_add_reduce_close_from_position_path() { /* 0→10→15→8→0 */ }

#[test]
fn excludes_cash_pay_and_synthetic_open_from_review_actions() { /* 只保留真实股票买卖 */ }

#[test]
fn date_only_reversal_is_kept_and_marked_order_uncertain() { /* 同日 BUY/SELL 无时间戳 */ }
```

断言合并键为账户、股票、交易日和方向，且只合并排序后连续的同向成交；均价为金额加权均价，金额与全部已记录费用相加。若中间存在反向成交则开始新动作；若存在可靠时间戳，反向成交按时间顺序逐笔重放。

- [ ] **Step 2: 运行测试确认模块或函数缺失**

Run: `cd src-tauri && cargo test --lib stock_action_builder::tests -- --nocapture`

Expected: FAIL，错误指向 `build_stock_actions`、`ActionBuildResult` 或 `PositionEvent` 尚未定义。

- [ ] **Step 3: 实现稳定排序、归一化与动作分类**

排序键依次使用 `traded_at`、`created_at`、`id`；将没有盘中时间的记录标记为日期精度。股票数量变化按现有交易模型的 BUY/SELL 语义重放，生成 `open/add/reduce/close`。现金符号、`PAY` 和合成 `OPEN` 只进入持仓/现金重建，不进入动作列表。

将动作 ID 生成规则固定为：

```text
action:{转义后的account_id}:{转义后的symbol}:{trade_date}:{side}:{首条transaction_id}
```

项目不新增哈希依赖；各组件转义冒号和路径字符后拼接。测试断言同一输入得到相同 ID，避免随机 UUID 破坏注释关联。

- [ ] **Step 4: 应用纠正规则并报告不能安全修复的问题**

- `same_day_order`：按 `value_json` 中确认的 transaction ID 顺序重放。
- `non_trade`：从动作和持仓事件中排除指定记录。
- `duplicate`：从动作推导中排除重复项，同时产生 `source_ledger_conflict`，供总报告停用影子增益和归因。
- `transfer`：动作构建阶段保留账户事件并加转仓标记，逻辑连接交给 Campaign builder。

增加测试断言纠正只改变派生动作，不改写 `transactions` 表。

- [ ] **Step 5: 验证所有动作构建边界**

Run: `cd src-tauri && cargo test --lib stock_action_builder::tests -- --nocapture`

Expected: PASS，四类动作、合并、费用、排序不确定和四类纠正均符合断言。

- [ ] **Step 6: 提交动作构建器**

```bash
git add src-tauri/src/services/stock_action_builder.rs src-tauri/src/services/mod.rs
git commit -m "feat: derive stock review actions"
```

---

### Task 3: 构建账户片段与逻辑 StockCampaign

**Files:**
- Create: `src-tauri/src/services/stock_campaign_builder.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/stock_campaign_builder.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: `ActionBuildResult.position_events`、`ActionBuildResult.actions`、转仓纠正。
- Produces: `pub fn build_stock_campaigns(events: &[PositionEvent], actions: &[StockActionReview], overrides: &[StockReviewOverride], as_of: NaiveDate) -> CampaignBuildResult`。
- Produces: `AccountCampaignFragment`、`StockCampaignSummary`、动作到 Campaign 的映射和质量问题。

- [ ] **Step 1: 为 Campaign 边界写失败测试**

测试以下路径：

```text
账户 A: 0 -> 10(open) -> 15(add) -> 6(reduce) -> 0(close)
账户 A: 0 -> 4(open) -> 7(add) -> 7(as_of，active)
```

第一条只生成一个 completed 片段且含四项动作；第二条生成一个 active 片段，`ended_at` 为 `None`。随后增加同一股票第二次开仓，断言它是新的 Campaign，而不是与历史片段合并。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd src-tauri && cargo test --lib stock_campaign_builder::tests -- --nocapture`

Expected: FAIL，缺少 `build_stock_campaigns` 或 Campaign 类型。

- [ ] **Step 3: 实现账户 Campaign 片段状态机**

按账户和股票分别重放 `PositionEvent`：持仓从 0 变正时开始；保持为正时追加动作；回到 0 时结束；报告结束仍为正则 active。片段 ID 固定为 `campaign:{account_id}:{symbol}:{opening_event_id}`；转仓连接后的 logical ID 固定引用 override ID，保证注释和详情链接跨刷新稳定。遇到负持仓或无法解释的数量跳变时停止该股票后续 Campaign 推导，并返回 `unavailable` 问题，而不是猜测。

- [ ] **Step 4: 实现经确认的跨账户转仓连接**

用 `transfer` 纠正中的来源/目标 transaction IDs 连接两个账户片段到同一 `logical_campaign_id`。组合级输出不得把配对转出/转入生成投资意义上的 `close/open`；单账户输出保留片段并标记 `transfer_out/transfer_in`。增加测试：A 账户转出 100 股、B 账户转入 100 股后，组合视图只有一个连续 Campaign，单账户视图各有一个带转仓事实的片段。

- [ ] **Step 5: 验证 Campaign 构建器**

Run: `cd src-tauri && cargo test --lib stock_campaign_builder::tests -- --nocapture`

Expected: PASS，completed/active/再入场/跨账户转仓/异常负持仓均有确定结果。

- [ ] **Step 6: 提交 Campaign 构建器**

```bash
git add src-tauri/src/services/stock_campaign_builder.rs src-tauri/src/services/mod.rs
git commit -m "feat: derive stock investment campaigns"
```

---

### Task 4: 建立股票与基准日线缓存

**Files:**
- Create: `src-tauri/src/services/stock_review_market_data.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/stock_review_market_data.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: `quote_service::fetch_stock_candles`、现有 `benchmark_daily_prices`、`exchange_rates`、标的市场和日期区间。
- Produces: `ensure_stock_price_cache`、`load_stock_price_series`、`load_benchmark_series`、`nth_market_session_after`、`MarketDataCoverage`。
- Produces: 后续服务统一使用的 `DailyMarketPoint { date, open, high, low, close, adjusted_close, dividend }`。

- [ ] **Step 1: 为缓存幂等与覆盖率写失败测试**

用内存数据库和固定 candles 测试：首次 upsert 写入三天，第二次写相同主键更新 close 而不增加行数；请求 100 个所需交易日，95 个有效点状态为 `available`，80–94 个为 `degraded`，少于 80 个为 `unavailable`。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd src-tauri && cargo test --lib stock_review_market_data::tests -- --nocapture`

Expected: FAIL，缺少缓存读写和 `MarketDataCoverage`。

- [ ] **Step 3: 实现纯数据库读写与覆盖率**

把网络获取和计算拆开：

```rust
pub fn upsert_stock_candles(db: &Database, symbol: &str, market: &str, source: &str, candles: &[PriceCandle]) -> Result<(), String>;
pub fn load_stock_price_series(db: &Database, symbol: &str, market: &str, start: NaiveDate, end: NaiveDate) -> Result<Vec<DailyMarketPoint>, String>;
pub fn classify_coverage(required: usize, present: usize) -> MarketDataCoverage;
```

总回报能力由 `adjusted_close`/`dividend` 实际覆盖决定，不能仅按 provider 名称推定。覆盖率分母使用目标市场交易日，而不是自然日。

- [ ] **Step 4: 实现异步补数和交易日窗口**

`ensure_stock_price_cache` 先读缓存缺口，再用现有 provider 配置调用 `fetch_stock_candles`，仅补缺失日期。对每项动作准备到 `min(today, action_date + 约 180 自然日)` 的数据，以找到 120 个市场交易日；历史已清仓标的也必须补数，不能依赖持仓快照。

基准读取优先复用 `benchmark_daily_prices` 与现有 benchmark fetch；自动映射固定为 US=`^GSPC`、CN=`000300.SS`、HK=`^HSI`。显式 benchmark 参数覆盖自动映射，并写入 methodology。

- [ ] **Step 5: 测试交易日计数和价格模式降级**

加入周末、节假日缺口和只含 close 的 candles，断言第 60/120 个实际数据点被选中；缺 adjusted/dividend 时 `return_mode = price_only` 且状态为 `degraded`。短期缺口只允许按同一市场交易日序列跳过，不跨长缺口填充评价终值；停牌或退市导致无法形成可靠终值时，对应动作窗口为 `unavailable`。

Run: `cd src-tauri && cargo test --lib stock_review_market_data::tests -- --nocapture`

Expected: PASS，缓存幂等、阈值、交易日和模式选择全部通过。

- [ ] **Step 6: 提交市场数据层**

```bash
git add src-tauri/src/services/stock_review_market_data.rs src-tauri/src/services/mod.rs
git commit -m "feat: cache stock review market data"
```

---

### Task 5: 实现不调仓影子组合引擎

**Files:**
- Create: `src-tauri/src/services/shadow_portfolio_engine.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/shadow_portfolio_engine.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: 报告开始日前一日的股票数量与多币种现金、逐日行情/汇率、外部资金流、拆股、分红。
- Produces: `pub fn build_shadow_series(input: &ShadowPortfolioInput) -> ShadowPortfolioResult`。
- Produces: 每日 `ShadowValuationPoint`、收益模式、缺失数据问题和期末价值。

- [ ] **Step 1: 写最小影子组合失败测试**

构造期初 10 股、股价 100、现金 1000；第二日真实账户卖出股票，但影子输入只重放 500 外部入金，收盘价 110。断言影子仍持有 10 股、现金为 1500、期末价值为 2600，且真实股票卖出不在影子事件输入中。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd src-tauri && cargo test --lib shadow_portfolio_engine::tests::ignores_stock_trades_but_replays_external_flows -- --nocapture`

Expected: FAIL，缺少 `build_shadow_series` 或输入模型。

- [ ] **Step 3: 实现逐日持仓、现金与汇率估值**

影子状态按账户、市场、股票和现金币种保存。每个估值日按顺序处理：拆股 → 外部资金流 → 分红/现金收益 → 收盘估值。所有金额用当天 `exchange_rates` 转到 `query.base_currency`；缺少当天汇率时允许使用最近一个有效工作日值，但必须把前向填充天数和覆盖率交给数据质量层。

外部资金流严格复用绩效服务对 `$CASH-*` BUY/SELL 和 `OPEN` 的识别语义；股票 BUY/SELL 不进入影子现金变化。

- [ ] **Step 4: 实现拆股和两种分红模式**

增加测试：2:1 拆股后数量翻倍、价格相应变化但价值不跳变。总回报数据可靠时用 adjusted/total-return series；否则为实际和影子构造一致的 price-only 曲线，不能只给影子遗漏分红。把 `return_mode` 设为 `total_return` 或 `price_only`，后者附 `degraded` 原因。

- [ ] **Step 5: 复用现有 TWR 约定生成影子净值**

把影子每日价值和外部流适配成 `performance_service::build_twr_return_series` 所需输入，禁止复制 TWR 公式。增加现金流日测试，断言入金本身不产生收益；增加多币种测试，断言同一汇率路径同时用于实际和影子。

Run: `cd src-tauri && cargo test --lib shadow_portfolio_engine::tests -- --nocapture`

Expected: PASS，股票交易被忽略，资金流、拆股、分红、汇率和 TWR 时点都正确。

- [ ] **Step 6: 提交影子引擎**

```bash
git add src-tauri/src/services/shadow_portfolio_engine.rs src-tauri/src/services/mod.rs
git commit -m "feat: build no-rebalance shadow portfolio"
```

---

### Task 6: 实现调仓归因与数据质量规则

**Files:**
- Create: `src-tauri/src/services/rebalance_attribution.rs`
- Create: `src-tauri/src/services/stock_review_quality.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: 两个新服务文件内的 `#[cfg(test)]`

**Interfaces:**
- Consumes: 实际/影子每日持仓和现金、行情、汇率、`StockReviewIssue`、数据覆盖率。
- Produces: `pub fn calculate_rebalance_attribution(input: &AttributionInput) -> RebalanceAttributionSummary`。
- Produces: `pub fn build_stock_review_quality(input: &QualityInput) -> StockReviewDataQuality` 和统一状态合并函数。

- [ ] **Step 1: 写归因恒等式失败测试**

构造两个股票：实际相对影子多持有 A、少持有 B；A 上涨、B 下跌。断言 A 为正贡献，少持有 B 为正机会收益，现金和汇率贡献分别返回。所有贡献金额之和与实际减影子期末价值差的残差单独列出。

- [ ] **Step 2: 运行归因测试确认失败**

Run: `cd src-tauri && cargo test --lib rebalance_attribution::tests -- --nocapture`

Expected: FAIL，缺少归因服务或输出模型。

- [ ] **Step 3: 实现实际减影子的每日差异归因**

股票贡献使用上一估值点的 `actual_quantity - shadow_quantity` 乘本期本地币种价格变动，加上差异持仓对应分红并减去可归属交易费用，再按当日汇率折算；现金贡献使用现金差额的收益；汇率贡献作为独立可加金额项。差异持仓按操作形成的增量批次跟踪，使金额可以按市场、股票、动作类型和具体动作汇总。百分比贡献以期间平均净资产归一化，明确标记为解释性近似，不声称精确分解 TWR。

输出至少包含 `contributors`、`detractors`、`currency_contribution`、`cash_contribution`、`explained_value_difference`、`residual` 和状态。

- [ ] **Step 4: 写并实现质量阈值测试**

在 `stock_review_quality.rs` 测试：

- 行情/汇率覆盖 `>= 95%` 为 `available`；`>= 80% 且 < 95%` 为 `degraded`；`< 80%` 为 `unavailable`。
- `abs(residual) / average_portfolio_nav` 不超过 `0.1%` 为 `available`，`0.1%–0.5%` 为 `degraded`，超过 `0.5%` 为 `unavailable`；平均净资产无有效正值时精确归因为 `unavailable`。
- 尚未走满 60/120 个市场交易日的动作是 `pending`，不算数据缺失。
- 多个输入状态合并时优先级为 `unavailable > degraded > pending > available`；若唯一限制只是观察期未结束则保持 `pending`。

- [ ] **Step 5: 覆盖重复记录与区间回撤提示**

源账本存在 duplicate 纠正时，实际结果仍按账本展示，但归因和调仓增益强制 `unavailable` 并给出“先修复原始交易记录”的可执行建议。报告开始日可能位于既有回撤中途时，增加 `interval_drawdown_only` 提示，不伪造区间外峰值。

Run: `cd src-tauri && cargo test --lib rebalance_attribution::tests -- --nocapture`

Expected: PASS，金额归因与残差计算符合规则。

Run: `cd src-tauri && cargo test --lib stock_review_quality::tests -- --nocapture`

Expected: PASS，覆盖率、pending、残差状态和源账本冲突符合规则。

- [ ] **Step 6: 提交归因与质量层**

```bash
git add src-tauri/src/services/rebalance_attribution.rs src-tauri/src/services/stock_review_quality.rs src-tauri/src/services/mod.rs
git commit -m "feat: attribute and qualify stock rebalancing"
```

---

### Task 7: 计算五项核心指标与 Campaign 详情

**Files:**
- Create: `src-tauri/src/services/stock_review_metrics.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/stock_review_metrics.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: 实际 TWR/回撤曲线、影子曲线、基准曲线、动作、Campaign、日线 OHLC、期初/逐日持仓和费用。
- Produces: `calculate_result_quality`、`calculate_rebalance_value_add`、`calculate_forward_effect`、`calculate_risk_structure`、`calculate_campaign_detail`。
- Produces: 五项 summary、归一化三曲线、Campaign P&L/MAE/MFE 与事实标签。

- [ ] **Step 1: 为期初固定权重混合基准写失败测试**

构造期初美国股票价值 60、A 股价值 30、现金 10；期内美股基准 +10%、A 股基准 -10%、现金 0%。断言混合基准为 `3%`，即 `0.6×10% + 0.3×(-10%)`，且期内真实调仓不改变这组权重。显式单市场筛选时只使用该市场基准。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd src-tauri && cargo test --lib stock_review_metrics::tests::uses_fixed_start_weights_for_mixed_benchmark -- --nocapture`

Expected: FAIL，缺少混合基准计算函数。

- [ ] **Step 3: 实现结果质量、最大回撤和调仓增益**

- 实际结果调用现有 `build_twr_return_series`，按请求币种把每日价值和外部流转换后再适配输入。
- `excess_return = actual_twr - benchmark_return`。
- 最大回撤返回幅度、峰值日、谷值日、持续天数、恢复日、恢复耗时；区间未恢复时恢复字段为 `None`。
- `rebalance_value_add = actual_comparable_return - shadow_return`，并返回期末价值差。price-only 模式下 actual comparable 与 shadow 同时排除分红，并将状态降级；实际 TWR 不被替换。

- [ ] **Step 4: 实现 60/120 日金额加权前瞻效果**

每个动作按操作方向定义效果：买入/加仓使用 `stock_return - market_benchmark_return`；减仓/清仓使用其相反数。收益按标的本地币种计算以排除汇率噪声；权重使用动作名义金额按动作日汇率折算到基准币种。

窗口输出：

```rust
pub struct ForwardEffectWindow {
    pub trading_days: u16,
    pub status: MetricAvailability,
    pub matured_actions: usize,
    pub pending_actions: usize,
    pub amount_weighted_excess_return: Option<f64>,
    pub positive_notional_ratio: Option<f64>,
}
```

60 日是卡片主值，120 日是验证值。观察未结束的动作只增加 pending 数，不进入分子分母。增加测试覆盖盈利买入、及时卖出、未成熟动作和不同市场交易日历。

- [ ] **Step 5: 实现风险结构、换手、费用与事实标签**

最大单股权重、CR5 和 HHI 都以股票资产为分母，现金比例单独返回，避免现金增加制造虚假分散化；返回期初、期末和期间峰值。单边换手率固定为 `sum(abs(stock_trade_notional_base)) / (2 × average_portfolio_nav)`，费用拖累为 `total_stock_trading_fees_base / average_portfolio_nav`。确认的转仓、拆股和非交易持仓变化不计入换手。费用为 0 时如实返回 0 并增加“费用可能未完整导入”提示；无可靠持仓快照时对应字段为 `None` 并降级，不以交易记录猜 0。

事实标签由后端按符号和状态生成：60 日正/负对应短期有效/短期不利，120 日正对应长期有效，卖出类正贡献为有效避损、负效果为事后机会损失，股票权重变化绝对值超过 5 个百分点时标记集中度明显变化；未成熟和缺数据分别为观察中、数据不足。标签只描述事后事实，不输出决策对错。

- [ ] **Step 6: 实现 Campaign P&L、MAE/MFE 和详情窗口**

completed Campaign 的净损益固定为“卖出总收入 + 分红 - 买入总支出 - 交易费用”；active Campaign 再加“剩余股数 × 当前价格”，明确为包含剩余市值的总盈亏而非已实现收益。最大投入资本是 Campaign 现金净投入路径的最大正值，不是事前风险预算。Campaign 超额收益使用同持有期市场基准。

MAE/MFE 使用现金流感知的逐日路径：先应用截至该日已发生的买卖、分红和费用，再分别用当日 low/high 估值长仓最不利/最有利损益；金额除以有效最大投入资本得到百分比。缺 high/low 时保留可算金额或把百分比设为不可用，不隐藏 Campaign，也不得命名为 R 倍数。详情同时返回持有期回撤。

Campaign 详情包含 20/60/120 日动作效果、现金流、成本、分红、费用、操作时间线、账户片段和系统事实标签，不自动给“正确/错误”。

active Campaign 计入组合真实结果和当前风险结构，但不进入依赖完整退出的 completed Campaign 数量、平均净收益或已完成排名；所有聚合字段都分别返回 active/completed 样本数。

- [ ] **Step 7: 验证指标服务**

Run: `cd src-tauri && cargo test --lib stock_review_metrics::tests -- --nocapture`

Expected: PASS，固定权重基准、TWR/回撤、价格模式、前瞻效果、风险结构、费用及 Campaign 指标全部通过。

- [ ] **Step 8: 提交指标服务**

```bash
git add src-tauri/src/services/stock_review_metrics.rs src-tauri/src/services/mod.rs
git commit -m "feat: calculate stock review core metrics"
```

---

### Task 8: 持久化注释与经确认的计算纠正

**Files:**
- Create: `src-tauri/src/services/stock_review_persistence.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/stock_review_persistence.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: `StockReviewAnnotationInput`、`StockReviewOverrideInput`、当前报告筛选。
- Produces: `list_annotations`、`save_annotation`、`list_overrides`、`validate_override`、`save_override`。
- Guarantees: 注释不改变计算；纠正只有通过结构、引用和语义校验后才可持久化。

- [ ] **Step 1: 写注释往返失败测试**

保存一条 `scope_type=campaign`、`source=user`、JSON 内容为投资假设的注释；再次使用相同 ID 保存更新内容。断言只有一行、`created_at` 保持、`updated_at` 更新，且 `list_annotations` 能按 scope/account/symbol 过滤。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd src-tauri && cargo test --lib stock_review_persistence::tests::annotation_upsert_preserves_created_at -- --nocapture`

Expected: FAIL，缺少 persistence 服务。

- [ ] **Step 3: 实现注释读写并隔离计算纠正**

注释 `value_json` 必须解析成 JSON 对象；`source=ai_confirmed` 只表示用户确认后由 AI 工具保存，不授予模型自行写入权限。任何注释字段都不能被 action/campaign/metric builder 读取为计算输入。

- [ ] **Step 4: 写纠正校验失败测试**

分别测试：

- transfer 必须引用两个不同账户、同股票、数量可匹配的记录；
- duplicate 至少引用两条经济含义相同的记录；
- same_day_order 必须引用同账户/股票/日期且方向反转的记录，并给出完整顺序；
- non_trade 必须引用存在的交易；
- 任一不存在的 transaction ID 都返回校验错误且数据库不新增行。

- [ ] **Step 5: 实现校验、幂等保存和审计字段**

`validate_override` 返回结构化错误，不修改数据库。`save_override` 使用稳定 ID upsert，保存原始 transaction IDs、确认值和时间。`list_overrides` 每次读取都校验引用交易：原始交易已删除时忽略该纠正并返回 `stale_override` 质量问题，绝不能静默继续应用。服务不直接重算报告；原子“预演 → 保存 → 返回新报告”由下一任务的 orchestrator 完成。

Run: `cd src-tauri && cargo test --lib stock_review_persistence::tests -- --nocapture`

Expected: PASS，注释与纠正隔离，四类纠正均校验，非法输入没有副作用。

- [ ] **Step 6: 提交持久化服务**

```bash
git add src-tauri/src/services/stock_review_persistence.rs src-tauri/src/services/mod.rs
git commit -m "feat: persist stock review context and corrections"
```

---

### Task 9: 编排确定性报告并暴露 Tauri 命令

**Files:**
- Create: `src-tauri/src/services/stock_review_service.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Modify: `src-tauri/src/commands/review.rs`
- Modify: `src-tauri/src/lib.rs`
- Test: `src-tauri/src/services/stock_review_service.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: Tasks 2–8 的纯服务、`Database`、quote provider 配置和 `StockReviewQuery`。
- Produces: `pub async fn get_stock_review_report(...) -> Result<StockReviewReport, String>`、`get_stock_campaign_detail`、`save_stock_review_annotation`、`confirm_stock_review_override`。
- Produces: camelCase Tauri 参数边界，返回 snake_case JSON 字段。

- [ ] **Step 1: 写报告编排集成失败测试**

创建内存账户、期初持仓、现金流、BUY/SELL、快照、汇率和缓存行情夹具。调用无网络的 `build_stock_review_report_from_cached_data`，断言：

- 报告默认有五项 summary；
- actions 和 campaigns 能互相引用；
- actual/shadow/benchmark 三条曲线使用相同日期轴并从 100 起始；
- methodology 返回筛选、基准、币种、收益模式、覆盖率、算法版本；
- annotations 只附加展示，不改变任一指标。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd src-tauri && cargo test --lib stock_review_service::tests::builds_complete_report_from_cached_data -- --nocapture`

Expected: FAIL，缺少 orchestrator 或缓存报告函数。

- [ ] **Step 3: 实现同步缓存编排核心**

`build_stock_review_report_from_cached_data` 按固定顺序执行：加载覆盖报告开始日之前的交易以重建期初 → 应用纠正 → 构建动作/Campaign → 生成实际/影子/基准曲线 → 指标/归因 → 聚合质量 → 附加注释和季度持仓历史笔记。动作列表只包含所选报告期，但 Campaign 可从更早日期开始并在所选期内继续。季度 `notes` 作为用户背景返回；`decision_quality` 只标明“历史手工评价”，不得参与首页指标。

全部账户和单账户共享同一服务；市场筛选不能把其他市场外部现金流误算为投资收益。无交易但有持仓时仍返回组合结果，动作效果状态为 unavailable 并说明“本期无可评价操作”。

- [ ] **Step 4: 实现异步数据准备和 Campaign 详情**

`get_stock_review_report` 先收集期初/期内涉及标的与基准，补齐缓存，再调用同步核心。网络失败不让整个命令崩溃：缓存足够时继续，覆盖不足时由状态规则降级/停用。

`get_stock_campaign_detail` 复用同一构建结果或同一底层函数，以 campaign ID 返回时间线、20/60/120 日效果、P&L、MAE/MFE、账户片段、注释和问题，禁止另写一套口径。

- [ ] **Step 5: 实现纠正的无副作用预演**

`confirm_stock_review_override` 依次执行：校验输入 → 将新纠正只加入内存 override 集合 → 完整生成候选报告 → 候选报告成功后持久化 → 返回候选报告。若报告构建失败，断言数据库没有新增纠正；成功时调用者无需再发一次读取请求。

- [ ] **Step 6: 注册四个 Tauri 命令**

在 `commands/review.rs` 添加：

```rust
#[tauri::command(rename_all = "camelCase")]
pub async fn get_stock_review_report(
    start_date: String,
    end_date: String,
    account_id: Option<String>,
    market: Option<String>,
    benchmark_symbol: Option<String>,
    base_currency: String,
    state: State<'_, AppState>,
) -> Result<StockReviewReport, String>;
```

同文件加入 `get_stock_campaign_detail`、`save_stock_review_annotation`、`confirm_stock_review_override`，并在 `lib.rs` invoke handler 注册。输入日期必须解析并校验 `start_date <= end_date`，币种只接受应用支持值，错误消息保持可展示。

- [ ] **Step 7: 覆盖设计文档的 12 个后端验收场景**

在 `stock_review_service.rs` 用表驱动夹具逐项覆盖设计文档的固定场景：

1. 无交易：实际与影子一致，调仓增益和换手率为 0。
2. 买入后上涨：建仓后续效果和调仓贡献为正。
3. 卖出后下跌：显示有效避损，贡献为正。
4. 卖出后上涨：显示事后机会损失，不自动判错。
5. 外部存款：资产增加但 TWR 不提高，实际与影子接收相同资金流。
6. 跨账户转仓：确认后不计组合换手、后续效果或新旧 Campaign。
7. 拆股：组合价值不变，不生成操作。
8. 分红：总回报正确；数据不足时影子指标按规则降级。
9. 最近交易：不足观察窗口时为 `pending`。
10. 多币种：使用每日汇率，汇率影响与股票贡献分开。
11. 数据缺失：只降级受影响指标，其他区域继续展示。
12. 归因守恒：操作、费用、分红、汇率和残差与实际/影子期末价值差在阈值内一致。

另外保留同日双向顺序不确定、重复记录冲突、固定权重多市场基准、停牌/退市和费用为 0 的专项测试。每个场景至少断言指标值/状态、质量提示和 actions/campaigns 数量。

Run: `cd src-tauri && cargo test --lib stock_review_service::tests -- --nocapture`

Expected: PASS，12 个验收夹具和无副作用纠正全部通过。

- [ ] **Step 8: 提交报告服务与命令**

```bash
git add src-tauri/src/services/stock_review_service.rs src-tauri/src/services/mod.rs src-tauri/src/commands/review.rs src-tauri/src/lib.rs
git commit -m "feat: expose deterministic stock review report"
```

---

### Task 10: 让 AI 助手复用确定性股票复盘

**Files:**
- Create: `src-tauri/src/skills/stock-review.md`
- Modify: `src-tauri/src/services/ai_tools.rs`
- Modify: `src-tauri/src/services/skill_service.rs`
- Modify: `docs/ai-tools.md`
- Test: `src-tauri/src/services/ai_tools.rs` 与 `src-tauri/src/services/skill_service.rs` 内现有测试模块

**Interfaces:**
- Consumes: `stock_review_service::get_stock_review_report`、`save_annotation`，以及 AI 会话中的账户/周期/市场/币种参数。
- Produces: 只读工具 `get_stock_review`、需明确确认才可调用的写工具 `save_stock_review_annotation`、内置 Skill `stock-review`。
- Guarantees: AI 解释确定性结果，不重算数字；只有必要时最多追问三个高价值问题。

- [ ] **Step 1: 写工具定义失败测试**

在现有 AI 工具清单测试中断言 `get_stock_review` 存在且 required 参数为 `start_date`、`end_date`、`base_currency`；可选参数为 `account_id`、`market`、`benchmark_symbol`、`symbol`、`campaign_id`。断言 `save_stock_review_annotation` 的描述包含“only after explicit user confirmation”，并要求结构化 scope 和 JSON value。

- [ ] **Step 2: 运行测试确认失败**

Run: `cd src-tauri && cargo test --lib ai_tools::tests -- --nocapture`

Expected: FAIL，工具清单中没有 `get_stock_review` 或保存注释工具。

- [ ] **Step 3: 注册和分发两个 AI 工具**

`get_stock_review` 调用与页面相同的 report service；未指定股票/Campaign 时返回组合摘要、归因重点和高价值动作，指定后复用 Campaign 详情并裁剪无关明细，以控制 AI 上下文体积，但任何保留指标都不得改值。不得调用旧的 decision quality 服务重算结果；历史手工评价只能标为用户背景。`save_stock_review_annotation` 只写注释，不暴露计算纠正写入，因为 transfer/duplicate/order/non-trade 需要页面的结构化预演确认流程。

工具错误需保留数据质量语义：参数错误返回可修正信息；指标 unavailable 仍是成功报告，不能变成工具异常。

- [ ] **Step 4: 编写窄触发、事实优先的 Skill**

`stock-review.md` 的触发词限定为“股票操作复盘”“调仓复盘”“股票复盘报告”等股票语义，避免仅凭“复盘”抢占期权 Skill。工作流固定为：

1. 先调用 `get_stock_review`。
2. 按“一句话结论 → 确定性事实 → 做得较好 → 值得复盘 → 无法仅凭数据判断 → 下一周期建议”输出。
3. 先讲结果质量和调仓增益，再讲 60/120 日效果、风险结构和主要归因，并明确区分事实、数据推断、用户背景和待确认事项。
4. 不输出综合分，不把动作自动标成正确/错误，不做因果断言、报价、短期预测或具体买卖建议。
5. 只有单项贡献超过本期调仓增益/损失 20%、权重变化超过 5 个百分点、收益与风险结论冲突、数据歧义会改变指标，或回答可长期复用时才追问；按影响排序，最多三个且允许跳过。
6. 已回答、可从数据确定、影响很小或只为了补全文字的问题不再询问。
7. 只有用户明确说“保存/记录这条背景”后才调用 `save_stock_review_annotation`。

增加 Skill 注册测试，断言内容含工具名、三问上限、禁止重算和禁止自动定性。

- [ ] **Step 5: 升级内置 Skill 版本并写工具文档**

在 `skill_service.rs` 将 `BUILTIN_SKILLS_VERSION` 从 7 升为 8，把 `stock-review.md` 加入内置列表；测试升级会安装新 Skill 且不删除用户 Skill。`docs/ai-tools.md` 记录参数、只读/写入性质、确定性数据来源和确认边界。

Run: `cd src-tauri && cargo test --lib skill_service::tests -- --nocapture`

Expected: PASS，新安装和从版本 7 升级都能获得 `stock-review`。

Run: `cd src-tauri && cargo test --lib ai_tools::tests -- --nocapture`

Expected: PASS，AI 工具参数、权限描述与执行分发一致。

- [ ] **Step 6: 提交 AI 集成**

```bash
git add src-tauri/src/skills/stock-review.md src-tauri/src/services/ai_tools.rs src-tauri/src/services/skill_service.rs docs/ai-tools.md
git commit -m "feat: add deterministic stock review assistant"
```

---

### Task 11: 建立 TypeScript 契约、ViewModel 与 Zustand Store

**Files:**
- Modify: `src/types/index.ts`
- Create: `src/pages/Review/stockReviewViewModel.ts`
- Create: `src/pages/Review/stockReviewViewModel.test.ts`
- Create: `src/stores/stockReviewStore.ts`
- Create: `src/stores/stockReviewStore.test.ts`
- Modify: `src/pages/Settings/GeneralSettings.tsx`

**Interfaces:**
- Consumes: Task 9 的 Tauri 命令和 Rust snake_case JSON；现有 `exchangeRateStore` 的 `baseCurrency`；账户 store。
- Produces: `StockReviewFilters`、默认周期/序列化函数、`useStockReviewStore`、可执行 AI 预填对象。
- Guarantees: 前端不计算财务结果；所有可空值和状态与 Rust 一致。

- [ ] **Step 1: 写 ViewModel 失败测试**

在 `stockReviewViewModel.test.ts` 用固定 `now = new Date('2026-08-28T00:00:00+08:00')` 测试：

- `YTD` 得到 `2026-01-01` 至 `2026-08-28`；
- `QTD` 得到 `2026-07-01` 至 `2026-08-28`；
- `PREV_QUARTER` 得到 `2026-04-01` 至 `2026-06-30`；
- `1Y` 得到 `2025-08-29` 至 `2026-08-28`；
- localStorage 缺失/损坏时回到“全部账户、YTD、全部市场、自动基准”；
- `buildStockReviewAiPrefill` 返回 active skill=`stock-review` 和完整可执行筛选参数，不自动发送。
- `buildStockCampaignAiPrefill` 额外返回 symbol/campaign ID，不自动发送。

两个预填函数分别使用已确认文案：

```text
请基于本期确定性股票复盘报告，分析整体调仓是否创造价值、收益是否依赖少数操作、风险结构是否改善，以及最值得进一步复盘的三项操作。请严格区分确定性事实、事后结果和缺失的决策背景。

请复盘当前股票Campaign，区分确定性事实、事后推断和缺失背景，重点分析加减仓节奏、仓位变化及其对组合的贡献。
```

- [ ] **Step 2: 运行 ViewModel 测试确认失败**

Run: `node --test src/pages/Review/stockReviewViewModel.test.ts`

Expected: FAIL，模块或导出函数不存在。

- [ ] **Step 3: 镜像 Rust 类型并实现纯函数**

在 `src/types/index.ts` 定义字符串联合：

```ts
export type MetricStatus = 'available' | 'degraded' | 'pending' | 'unavailable'
export type StockActionType = 'open' | 'add' | 'reduce' | 'close'
export type StockReviewPeriodPreset = 'QTD' | 'PREV_QUARTER' | 'YTD' | '1Y' | 'CUSTOM'
```

其余接口字段名逐一匹配 `StockReviewReport`，金额/百分比缺失均用 `number | null`，不能用可选字段掩盖后端遗漏。实现 `review_stock_filters_v1` 的解析和写入，忽略未知旧值并回退默认。

- [ ] **Step 4: 写 Store 失败测试**

mock `@tauri-apps/api/core` 的 `invoke`，验证：

- 初次 `loadReport` 以 camelCase 参数调用 `get_stock_review_report`；
- 两次并发请求后，较早返回不能覆盖较新的筛选报告；
- `loadCampaignDetail` 只更新 drawer 数据，不清空总报告；
- `saveAnnotation` 成功后更新当前 scope 的注释；
- `confirmOverride` 直接采用命令返回的新报告；
- 错误保留上一次成功报告并设置可展示 error。

- [ ] **Step 5: 实现 Store 与设置清理**

使用单调递增 request ID 防竞态，状态包含 `reportLoading`、`campaignLoading`、`mutating`、`report`、`selectedCampaign`、`error`。`confirmOverride` 不额外发第二次 load。`GeneralSettings` 的“清除应用数据”加入 `review_stock_filters_v1`，不影响其他偏好键。

- [ ] **Step 6: 验证前端数据层**

Run: `node --test src/pages/Review/stockReviewViewModel.test.ts src/stores/stockReviewStore.test.ts`

Expected: PASS，日期边界、损坏偏好、命令参数、竞态和 mutation 行为全部通过。

- [ ] **Step 7: 提交前端数据层**

```bash
git add src/types/index.ts src/pages/Review/stockReviewViewModel.ts src/pages/Review/stockReviewViewModel.test.ts src/stores/stockReviewStore.ts src/stores/stockReviewStore.test.ts src/pages/Settings/GeneralSettings.tsx
git commit -m "feat: add stock review frontend data layer"
```

---

### Task 12: 构建组合优先的股票复盘页面

**Files:**
- Create: `src/pages/Review/StockReviewFilters.tsx`
- Create: `src/pages/Review/StockReviewDataQuality.tsx`
- Create: `src/pages/Review/StockReviewSummaryCards.tsx`
- Create: `src/pages/Review/PortfolioComparisonChart.tsx`
- Create: `src/pages/Review/RebalanceAttributionPanel.tsx`
- Create: `src/pages/Review/RiskStructurePanel.tsx`
- Create: `src/pages/Review/StockActionsTable.tsx`
- Create: `src/pages/Review/StockCampaignDrawer.tsx`
- Create: `src/pages/Review/LegacyStockReviewPanel.tsx`
- Modify: `src/pages/Review/StockReviewTab.tsx`
- Modify: `src/pages/AiAssistant/index.tsx`
- Modify: `src/components/ai/ToolCallCard.tsx`
- Test: `src/pages/Review/stockReviewViewModel.test.ts`

**Interfaces:**
- Consumes: `useStockReviewStore`、账户列表、`exchangeRateStore.baseCurrency`、路由预填机制和 `StockReviewReport`。
- Produces: 默认自动加载的报告页、Campaign drawer、结构化纠正确认、旧版折叠入口和 AI 深度复盘跳转。
- Guarantees: 状态、口径和问题对用户可见；没有任何强制填写步骤。

- [ ] **Step 1: 给展示映射补失败测试**

扩展 `stockReviewViewModel.test.ts`，断言：

- 四种状态分别映射为正常、降级、观察中、不可用的中文文案和颜色；
- `null` 数值显示 `—`，不能显示 `0.00%`；
- action 类型映射为建仓/加仓/减仓/清仓；
- actual/shadow/benchmark 任一缺失点保持 gap，不前端补 0；
- 数据质量问题按 blocker、warning、info 排序；
- 五张卡顺序固定为结果质量、最大回撤、调仓增益、后续效果、风险结构。
- 空报告生成空状态，部分可用报告只隐藏受影响值，命令完全失败才生成全页错误和重试动作。
- 动作可按日期、金额、调仓贡献和后续效果排序，排序只使用后端返回值。
- 组合级 AI 预填包含当前筛选，Campaign 级预填额外包含 `symbol` 和 `campaign_id`。

- [ ] **Step 2: 运行测试确认失败**

Run: `node --test src/pages/Review/stockReviewViewModel.test.ts`

Expected: FAIL，缺少状态/显示/排序函数。

- [ ] **Step 3: 实现筛选、质量条和五张核心卡**

`StockReviewTab` mount 后直接按持久化筛选加载，不显示选股前置页。顶部筛选支持账户、QTD/上季度/YTD/1Y/自定义、市场、自动/指定基准和基准币种；币种默认跟随全局设置。筛选变化写入偏好并加载，并提供显式“刷新复盘”按钮。全命令失败时显示全页错误和重试；局部指标不可用时仍展示其余区域。

`StockReviewDataQuality` 正常时显示“覆盖率、分析动作数、观察中数量”一行摘要；有问题时按受影响股票、日期和指标展开，不用全局错误遮挡可用区域，并显示行情、汇率、基准、收益模式和算法版本。五张卡每张都读取后端 value/status/note；调仓增益卡并列 actual、shadow 和期末价值差，后续效果卡以 60 日为主、120 日为验证并显示成熟/观察中数量。

- [ ] **Step 4: 实现三曲线、归因和风险结构**

`PortfolioComparisonChart` 用 ECharts 绘制从 100 起始的实际、影子、基准线，并直接使用后端 action 日期/类型添加建仓、加仓、减仓、清仓标记；点击标记打开对应 Campaign。后端点为 null 时 `connectNulls=false`。tooltip 显示收益模式，影子/基准 unavailable 时只画可用曲线并展示原因。

归因面板先按建仓、加仓、减仓、清仓展示动作数量与估算贡献，再分“主要正贡献”“主要机会损失”“费用/分红/汇率/现金”“未解释残差”，不把解释性百分比说成 TWR 精确分解。风险结构展示最大单股权重、CR5、现金比例、换手率、费用拖累及期初/期末变化，HHI 放在展开详情。

- [ ] **Step 5: 实现动作表与 Campaign drawer**

动作表默认按日期倒序，支持股票、动作类型、Campaign 和状态筛选，并支持按日期、金额、调仓贡献和后续效果排序；显示日期、账户、股票、动作类型、加权价格、金额、费用、操作前后股数与组合权重、60/120 日相对效果、基准币种调仓贡献、状态和事实标签。点击行打开对应 Campaign，而不是要求先选择股票。

drawer 展示账户片段、操作时间线、成本/现金流、净收益/超额收益、最大持仓金额和权重、最大投入资本、MAE/MFE、持有期回撤、20/60/120 日效果、组合贡献、进行中/已完成状态、季度历史笔记、注释和问题。进行中总盈亏必须注明包含剩余持仓市值，不称已实现收益。注释保存直接调用 store；transfer/duplicate/same_day_order/non_trade 使用确认弹窗，弹窗必须列出受影响交易和预期影响，用户确认后才调用 `confirmOverride`。

- [ ] **Step 6: 保留旧手工复盘但移出主流程**

把当前 `StockReviewTab.tsx` 的选股、季度持仓和 decision quality 内容无口径改变地移到 `LegacyStockReviewPanel.tsx`，主页面底部用默认收起的“历史手工决策记录”展示。继续使用现有 `reviewStore.ts`，保证旧记录可读写；新版主报告不得读取 decision quality 作为核心指标。

- [ ] **Step 7: 接入 AI 深度复盘**

页面“请 AI 深度复盘”按钮调用 `buildStockReviewAiPrefill(report.methodology.filters)`；Campaign drawer 的按钮使用 `buildStockCampaignAiPrefill(filters, symbol, campaignId)`。两者都导航到 AI 助手并激活 `stock-review`，仅预填不自动发送。`AiAssistant/index.tsx` 消费预填一次后清除；`ToolCallCard.tsx` 为 `get_stock_review` 和 `save_stock_review_annotation` 增加中文显示名。

- [ ] **Step 8: 验证页面编译与关键行为**

Run: `node --test src/pages/Review/stockReviewViewModel.test.ts src/stores/stockReviewStore.test.ts`

Expected: PASS，显示映射、筛选、竞态和预填行为通过。

Run: `npm run build`

Expected: PASS，无 TypeScript 错误；Vite 能产出页面 bundle。

- [ ] **Step 9: 提交组合报告页面**

```bash
git add src/pages/Review src/pages/AiAssistant/index.tsx src/components/ai/ToolCallCard.tsx
git commit -m "feat: redesign stock operation review page"
```

---

### Task 13: 全量验收、回归与交付说明

**Files:**
- Modify: `docs/superpowers/specs/2026-08-28-stock-operation-review-redesign.md`（只在实现发现口径必须澄清时同步已确认事实）
- Modify: `docs/ai-tools.md`（只补实现后最终签名差异）
- Test: 所有新增 Rust/TypeScript 测试及现有回归测试

**Interfaces:**
- Consumes: 完整后端、AI 和页面实现。
- Produces: 可复现的测试证据、无占位符的文档和干净的最终差异。

- [ ] **Step 1: 执行 Rust 格式与定向测试**

Run: `cd src-tauri && cargo fmt --check`

Expected: PASS，无格式差异。

Run: `cd src-tauri && cargo test --lib stock_ -- --nocapture`

Expected: PASS，动作、Campaign、市场数据、指标、质量、持久化和 12 个报告验收场景全绿。

Run: `cd src-tauri && cargo test --lib shadow_portfolio_engine::tests -- --nocapture`

Expected: PASS，影子组合资金流、拆股、分红、汇率和 TWR 场景全绿。

Run: `cd src-tauri && cargo test --lib rebalance_attribution::tests -- --nocapture`

Expected: PASS，实际减影子的归因和残差场景全绿。

- [ ] **Step 2: 执行完整 Rust 回归**

Run: `cd src-tauri && cargo test --lib`

Expected: PASS，现有绩效、交易导入、旧股票复盘、期权复盘和 AI 工具测试没有回归。

- [ ] **Step 3: 执行前端测试与生产构建**

Run: `node --test src/pages/Review/stockReviewViewModel.test.ts src/stores/stockReviewStore.test.ts src/pages/Review/optionReviewViewModel.test.ts src/pages/AiAssistant/prefill.test.ts`

Expected: PASS，新旧复盘和 AI 预填纯函数全绿。

Run: `npm run build`

Expected: PASS，TypeScript 和生产打包成功。

- [ ] **Step 4: 按用户路径做桌面冒烟验收**

Run: `npm run tauri dev`

Expected: 应用启动。人工依次验证：打开股票复盘即见 YTD 报告；切换账户/周期/市场会刷新且重启后保留；五卡、三曲线、归因、动作和 Campaign 能钻取；缺数据展示降级而不是 0；未成熟动作显示观察中；旧手工记录仍可展开；AI 按钮只预填；未经确认不能写注释或纠正；确认纠正后页面采用返回的新报告。

- [ ] **Step 5: 检查占位符、调试输出和差异卫生**

Run: `rg -n "TO[D]O|TB[D]|FIXM[E]|placeholde[r]|console\.log|dbg!" src src-tauri/src docs/ai-tools.md`

Expected: 无本功能新增的占位符或调试输出；仓库原有命中逐条确认与本次无关。

Run: `git diff --check`

Expected: PASS，无尾随空格或冲突标记。

Run: `git status --short`

Expected: 本功能文件都已提交；工作区只可能保留实施开始前已经存在的用户改动，不包含缓存行情或构建产物。

- [ ] **Step 6: 提交最终验收修正**

如果前五步产生必要修正：

```bash
git add src src-tauri/src docs/ai-tools.md docs/superpowers/specs/2026-08-28-stock-operation-review-redesign.md
git commit -m "test: verify stock operation review workflow"
```

如果没有文件变化，不创建空提交；在交付说明中记录实际执行命令和结果。
