# P1 Portfolio Read Model Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把持仓、账户、类别与行情拼装从 command 层移到单请求读模型；仪表盘用一次 Tauri 请求原子取得 summary 与 holdings，统计页只请求当前活动视图，并保持全部公式、货币口径和行情策略不变。

**Architecture:** `portfolio_read_service` 负责构造单请求生命周期的 `PortfolioReadModel`，通过 `CacheOnly`/`RefreshMissing` 显式区分行情策略；`statistics_service` 对该模型执行纯聚合。Tauri command 只加载依赖并委托服务。前端将 Dashboard 与 Statistics store 分开，使用可注入 invoke 的 action；Statistics 父页面成为唯一请求调度者，子页签只展示数据。

**Tech Stack:** Rust 1.97、Tauri 2、rusqlite 0.40、SQLite、React 19、TypeScript 7、Zustand 5、Node test runner、Bun。

**Spec:** `docs/superpowers/specs/2026-09-02-p1-simplification-and-read-model-design.md`

## Global Constraints

- 不增加跨请求缓存、连接池、全局失效逻辑或新的状态框架。
- `RefreshMissing` 只用于 Dashboard；Statistics、AI 和市场概览必须使用 `CacheOnly`，不得触发外部行情。
- 缓存缺失报价继续沿用当前 `current_price = 0`、`change = 0` 行为，本 P1 不改变缺失报价语义。
- 市场/账户统计继续使用市场原生货币；整体/类别统计继续换算到所选基准货币。
- 既有排序、top gainers/losers、盈亏、日盈亏、类别缺失回退和空成本行为保持不变。
- Dashboard 报告失败时保留上次成功的 summary 与 holdings，不能出现半更新。
- Statistics 每次动作只 invoke 一个视图；隐藏页签不得因 mount、货币变化或刷新而请求。
- 每项行为变化遵循 RED/GREEN：先写会失败的服务/store/纯调度测试，再写实现。

---

### Task 1: 为读模型建立数据库与报价行为测试

**Files:**
- Create: `src-tauri/src/services/portfolio_read_service.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/portfolio_read_service.rs` `#[cfg(test)]` module
- Reference: `src-tauri/src/commands/dashboard.rs`
- Reference: `src-tauri/src/services/quote_service/cache.rs`

**Interfaces:**
- Produces: `QuoteReadMode`、`PortfolioReadModel::load(...)`、`holdings()`、`holdings_with_usd(...)`。
- Consumes: `Database`、`QuoteCache`、可选 `QuoteServiceState`。

- [ ] **Step 1: 先创建 module 声明与测试壳，写入真实 fixture helper**：

  ```rust
  fn seeded_db() -> Database {
      let db = Database::new(":memory:").unwrap();
      let conn = db.conn.lock().unwrap();
      conn.execute(
          "INSERT INTO accounts (id, name, market, description, created_at, updated_at)
           VALUES ('acct-us', 'US Broker', 'US', '', '2026-01-01', '2026-01-01')",
          [],
      ).unwrap();
      conn.execute(
          "INSERT INTO categories (id, name, color, icon, is_system, sort_order, created_at)
           VALUES ('growth', '成长', '#1677ff', '', 0, 0, '2026-01-01')",
          [],
      ).unwrap();
      conn.execute(
          "INSERT INTO holdings
           (id, account_id, symbol, name, market, category_id, shares, avg_cost, currency, created_at, updated_at)
           VALUES ('holding-aapl', 'acct-us', 'AAPL', 'Apple', 'US', 'growth', 10, 10, 'USD', '2026-01-01', '2026-01-01')",
          [],
      ).unwrap();
      drop(conn);
      db
  }

  fn cached_aapl() -> StockQuote {
      StockQuote {
          symbol: "AAPL".to_string(),
          name: "Apple".to_string(),
          market: "US".to_string(),
          current_price: 12.0,
          previous_close: 11.0,
          change: 1.0,
          change_percent: 100.0 / 11.0,
          updated_at: "2026-09-02T09:30:00Z".to_string(),
          ..StockQuote::default()
      }
  }
  ```

- [ ] **Step 2: 写 RED 测试：CacheOnly 不需要 provider state 且正确拼装字段**：

  ```rust
  #[tokio::test]
  async fn cache_only_builds_holding_details_without_quote_state() {
      let db = seeded_db();
      let cache = QuoteCache::new();
      cache.set(cached_aapl());

      let model = PortfolioReadModel::load(&db, &cache, None, QuoteReadMode::CacheOnly)
          .await
          .unwrap();

      assert_eq!(model.holdings().len(), 1);
      let holding = &model.holdings()[0];
      assert_eq!(holding.account_name, "US Broker");
      assert_eq!(holding.category_name, "成长");
      assert_eq!(holding.current_price, 12.0);
      assert_eq!(holding.market_value, 120.0);
      assert_eq!(holding.cost_value, 100.0);
      assert_eq!(holding.pnl, 20.0);
      assert_eq!(holding.daily_pnl, 10.0);
  }
  ```

- [ ] **Step 3: 写 RED 测试：RefreshMissing 必须提供 state**：

  ```rust
  #[tokio::test]
  async fn refresh_missing_requires_quote_state() {
      let db = seeded_db();
      let cache = QuoteCache::new();
      let error = PortfolioReadModel::load(&db, &cache, None, QuoteReadMode::RefreshMissing)
          .await
          .unwrap_err();
      assert!(error.contains("quote service state is required"));
  }
  ```

- [ ] **Step 4: 运行测试并验证 RED**：

  ```bash
  cargo test --manifest-path src-tauri/Cargo.toml portfolio_read_service
  ```

  失败原因必须是接口尚未实现，而不是 fixture SQL 错误。

- [ ] **Step 5: 实现公开边界**：

  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum QuoteReadMode {
      CacheOnly,
      RefreshMissing,
  }

  #[derive(Debug)]
  pub struct PortfolioReadModel {
      holdings: Vec<HoldingDetail>,
  }

  impl PortfolioReadModel {
      pub async fn load(
          db: &Database,
          quote_cache: &QuoteCache,
          quote_state: Option<&QuoteServiceState>,
          mode: QuoteReadMode,
      ) -> Result<Self, String>;

      pub fn holdings(&self) -> &[HoldingDetail] {
          &self.holdings
      }

      pub fn holdings_with_usd(&self, rates: &ExchangeRates) -> Vec<HoldingDetail>;
  }
  ```

  将 `commands/dashboard.rs::build_holding_details` 的 SQL、严格 row collect、报价 map 与公式原样迁入 `load`。`CacheOnly` 调用 `quote_cache.get_batch`；`RefreshMissing` 读取 provider 配置并调用现有 `fetch_quotes_batch_cached_with_providers`。

- [ ] **Step 6: 为 USD 归一化补测试并实现**：使用 `usd_cny = 5.0`，断言 1,000 CNY 的 `market_value_usd` 为 200；不得覆盖 native `market_value`。

- [ ] **Step 7: 运行定向测试与 Clippy**：

  ```bash
  cargo test --manifest-path src-tauri/Cargo.toml portfolio_read_service
  cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
  ```

### Task 2: 移除 service → command 反向依赖

**Files:**
- Modify: `src-tauri/src/services/market_overview_service.rs`
- Modify: `src-tauri/src/services/ai_tools.rs`
- Modify: `src-tauri/src/services/ai_chat/context.rs`
- Modify: `src-tauri/src/services/ai_chat_service.rs`
- Modify: `src-tauri/src/commands/statistics.rs`
- Modify: `src-tauri/src/commands/dashboard.rs`

**Interfaces:**
- Removes: `crate::commands::dashboard::build_holding_details_pub`。
- Produces: 所有非 Dashboard 消费者显式使用 `QuoteReadMode::CacheOnly`。

- [ ] **Step 1: 逐个消费者把 import 改为**：

  ```rust
  use crate::services::portfolio_read_service::{PortfolioReadModel, QuoteReadMode};
  ```

- [ ] **Step 2: 将旧调用**：

  ```rust
  let details = build_holding_details_pub(db, quote_cache, true).await?;
  ```

  改为：

  ```rust
  let model = PortfolioReadModel::load(db, quote_cache, None, QuoteReadMode::CacheOnly).await?;
  let details = model.holdings();
  ```

  调用方需要拥有集合时使用 `.to_vec()`；不要把 service 再包装回 command helper。

- [ ] **Step 3: 对 `market_overview_service` 保留现有 best-effort 错误分支**；仅替换数据来源，不改变其错误文案、指数数据或 mover 可用性逻辑。

- [ ] **Step 4: 运行反向依赖扫描**：

  ```bash
  rg -n 'crate::commands::dashboard|build_holding_details_pub' src-tauri/src
  ```

  必须无输出。

- [ ] **Step 5: 运行相关测试**：

  ```bash
  cargo test --manifest-path src-tauri/Cargo.toml ai_tools
  cargo test --manifest-path src-tauri/Cargo.toml market_overview
  cargo test --manifest-path src-tauri/Cargo.toml ai_chat
  ```

### Task 3: 用一个 DashboardReport 替代两次持仓上下文加载

**Files:**
- Modify: `src-tauri/src/models/dashboard.rs`
- Modify: `src-tauri/src/models/mod.rs`
- Modify: `src-tauri/src/services/portfolio_read_service.rs`
- Modify: `src-tauri/src/commands/dashboard.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src/types/index.ts`
- Modify: `src/stores/dashboardStore.ts`
- Create: `src/stores/dashboardStore.test.ts`
- Modify: `src/pages/Dashboard/index.tsx`

**Interfaces:**
- Produces: `get_dashboard_report(base_currency) -> DashboardReport`。
- Removes: Tauri `get_dashboard_summary` 与 `get_holdings_with_quotes`。
- Frontend produces: `fetchReport(baseCurrency?) -> Promise<void>`。

- [ ] **Step 1: 增加 Rust/TypeScript 契约**：

  ```rust
  #[derive(Debug, Serialize, Deserialize, Clone)]
  pub struct DashboardReport {
      pub summary: DashboardSummary,
      pub holdings: Vec<HoldingDetail>,
  }
  ```

  ```ts
  export interface DashboardReport {
    summary: DashboardSummary;
    holdings: HoldingDetail[];
  }
  ```

  在 Rust `models/mod.rs` 一并 re-export `DashboardReport`。

- [ ] **Step 2: 在读模型测试中先写 RED 报告断言**。对 10 股 AAPL、现价 12、日涨 1、USD 基准断言：总市值 120、总成本 100、总盈亏 20、日盈亏 10，且 report holding 的 `market_value_usd == 120`。

- [ ] **Step 3: 把当前 summary 公式原样移动到**：

  ```rust
  impl PortfolioReadModel {
      pub fn dashboard_report(
          &self,
          rates: ExchangeRates,
          base_currency: String,
      ) -> DashboardReport {
          let holdings = self.holdings_with_usd(&rates);
          let mut us_market_value = 0.0;
          let mut cn_market_value = 0.0;
          let mut hk_market_value = 0.0;
          let mut total_cost = 0.0;

          for holding in &self.holdings {
              let market_value = convert_currency(
                  holding.market_value, &holding.currency, &base_currency, &rates,
              );
              let cost_value = convert_currency(
                  holding.cost_value, &holding.currency, &base_currency, &rates,
              );
              match holding.market.as_str() {
                  "US" => us_market_value += market_value,
                  "CN" => cn_market_value += market_value,
                  "HK" => hk_market_value += market_value,
                  _ => {}
              }
              total_cost += cost_value;
          }

          let total_market_value = us_market_value + cn_market_value + hk_market_value;
          let total_pnl = total_market_value - total_cost;
          let total_pnl_percent = if total_cost != 0.0 {
              total_pnl / total_cost * 100.0
          } else {
              0.0
          };
          let daily_pnl = self.holdings.iter().map(|holding| {
              convert_currency(
                  holding.daily_pnl, &holding.currency, &base_currency, &rates,
              )
          }).sum();

          DashboardReport {
              summary: DashboardSummary {
                  total_market_value,
                  total_cost,
                  total_pnl,
                  total_pnl_percent,
                  daily_pnl,
                  us_market_value,
                  cn_market_value,
                  hk_market_value,
                  exchange_rates: rates,
                  base_currency,
              },
              holdings,
          }
      }
  }
  ```

  `holdings_with_usd(&rates)` 与 summary 必须都从 `self.holdings` 生成。

- [ ] **Step 4: 将 Dashboard command 收敛为唯一入口**：

  ```rust
  #[tauri::command(rename_all = "camelCase")]
  pub async fn get_dashboard_report(
      db: State<'_, Database>,
      cache: State<'_, ExchangeRateCache>,
      quote_cache: State<'_, QuoteCache>,
      quote_state: State<'_, QuoteServiceState>,
      base_currency: Option<String>,
  ) -> Result<DashboardReport, String> {
      let base = base_currency.unwrap_or_else(|| "USD".to_string());
      let rates = get_cached_rates(&cache, &db).await?;
      let model = PortfolioReadModel::load(
          &db,
          &quote_cache,
          Some(&quote_state),
          QuoteReadMode::RefreshMissing,
      ).await?;
      Ok(model.dashboard_report(rates, base))
  }
  ```

  删除两个旧 command 函数和注册，注册 `get_dashboard_report`。

- [ ] **Step 5: 先写 RED store 测试**，直接使用 Node runner：

  ```ts
  // @ts-nocheck
  import test from "node:test";
  import assert from "node:assert/strict";
  import { createDashboardStore } from "./dashboardStore.ts";

  const report = {
    summary: {
      total_market_value: 120, total_cost: 100, total_pnl: 20,
      total_pnl_percent: 20, daily_pnl: 10,
      us_market_value: 120, cn_market_value: 0, hk_market_value: 0,
      exchange_rates: { usd_cny: 7, usd_hkd: 7.8, cny_hkd: 1.114, updated_at: "now" },
      base_currency: "USD",
    },
    holdings: [{ id: "holding-aapl", symbol: "AAPL" }],
  };

  test("dashboard refresh uses one report command and updates atomically", async () => {
    const calls = [];
    const store = createDashboardStore(async (command, args) => {
      calls.push([command, args]);
      return report;
    });
    await store.getState().fetchReport("USD");
    assert.deepEqual(calls, [["get_dashboard_report", { baseCurrency: "USD" }]]);
    assert.equal(store.getState().summary, report.summary);
    assert.equal(store.getState().holdingDetails, report.holdings);
  });

  test("failed dashboard refresh preserves the last complete report", async () => {
    let attempt = 0;
    const store = createDashboardStore(async () => {
      attempt += 1;
      if (attempt === 1) return report;
      throw new Error("offline");
    });
    await store.getState().fetchReport("USD");
    await store.getState().fetchReport("CNY");
    assert.equal(store.getState().summary, report.summary);
    assert.equal(store.getState().holdingDetails, report.holdings);
    assert.match(store.getState().error ?? "", /offline/);
  });
  ```

- [ ] **Step 6: 实现可注入 store**：

  ```ts
  export type DashboardInvoke = <T>(
    command: string,
    args?: Record<string, unknown>
  ) => Promise<T>;

  export const createDashboardStore = (invokeFn: DashboardInvoke = invoke) =>
    create<DashboardState>((set) => ({
      summary: null,
      holdingDetails: [],
      loading: false,
      error: null,
      fetchReport: async (baseCurrency) => {
        set({ loading: true, error: null });
        try {
          const report = await invokeFn<DashboardReport>("get_dashboard_report", {
            baseCurrency: baseCurrency ?? null,
          });
          set({
            summary: report.summary,
            holdingDetails: report.holdings,
            loading: false,
          });
        } catch (error) {
          set({ loading: false, error: String(error) });
        }
      },
    }));

  export const useDashboardStore = createDashboardStore();
  ```

- [ ] **Step 7: 更新 Dashboard 页面**：初次加载只调用 `fetchReport(baseCurrency)`；手动刷新先 `fetchHoldingQuotes()` 再 `fetchReport(baseCurrency)`；货币变化只调用一次 `fetchReport(currency)`。汇率卡读取 `summary?.exchange_rates`，删除独立 `fetchRates` 和 rates loading/error 状态。

  初次加载 effect 只在 mount 时执行；货币变化由 handler 驱动，不能同时让依赖 `baseCurrency` 的 effect 再发第二次报告请求。`loading` 同时传给 Summary 与 Holdings，`error` 由现有 Summary 错误区展示。

- [ ] **Step 8: 运行**：

  ```bash
  node --test src/stores/dashboardStore.test.ts
  cargo test --manifest-path src-tauri/Cargo.toml portfolio_read_service
  bun run build
  ```

### Task 4: 把四组统计公式移到纯 service，并让 Overview 携带同源 holdings

**Files:**
- Create: `src-tauri/src/services/statistics_service.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Modify: `src-tauri/src/models/statistics.rs`
- Modify: `src-tauri/src/commands/statistics.rs`
- Modify: `src/types/index.ts`
- Test: `src-tauri/src/services/statistics_service.rs` `#[cfg(test)]` module

**Interfaces:**
- Produces: `overview`、`by_market`、`by_account`、`by_category` 纯聚合函数。
- Adds: `StatisticsOverview.holdings: Vec<HoldingDetail>`。

- [ ] **Step 1: 给 Rust/TypeScript `StatisticsOverview` 增加同名字段**：

  ```rust
  pub holdings: Vec<crate::models::dashboard::HoldingDetail>,
  ```

  ```ts
  holdings: HoldingDetail[];
  ```

- [ ] **Step 2: 在新 service 中先写双市场等价性 fixture**：

  ```rust
  fn holding(
      id: &str,
      account_id: &str,
      account_name: &str,
      symbol: &str,
      market: &str,
      shares: f64,
      avg_cost: f64,
      current_price: f64,
      currency: &str,
  ) -> HoldingDetail {
      let market_value = shares * current_price;
      let cost_value = shares * avg_cost;
      let pnl = market_value - cost_value;
      HoldingDetail {
          id: id.to_string(),
          account_id: account_id.to_string(),
          account_name: account_name.to_string(),
          symbol: symbol.to_string(),
          name: symbol.to_string(),
          market: market.to_string(),
          category_name: "成长".to_string(),
          category_color: "#1677ff".to_string(),
          shares,
          avg_cost,
          current_price,
          market_value,
          cost_value,
          pnl,
          pnl_percent: Some(pnl / cost_value * 100.0),
          daily_pnl: 0.0,
          currency: currency.to_string(),
          market_value_usd: market_value,
      }
  }

  fn fixture() -> (PortfolioReadModel, ExchangeRates) {
      let model = PortfolioReadModel::from_holdings_for_test(vec![
          holding("h-us", "acct-us", "US Broker", "AAPL", "US", 10.0, 10.0, 12.0, "USD"),
          holding("h-cn", "acct-cn", "CN Broker", "600519", "CN", 100.0, 8.0, 10.0, "CNY"),
      ]);
      let rates = ExchangeRates {
          usd_cny: 5.0,
          usd_hkd: 7.8,
          cny_hkd: 1.56,
          updated_at: "2026-09-02T09:30:00Z".to_string(),
      };
      (model, rates)
  }
  ```

- [ ] **Step 3: 写 RED 断言**：
  - `overview(..., "USD")`：总市值 320、总成本 260、总盈亏 60、盈亏率 `60 / 260 * 100`；CN 分布 200、US 分布 120；holdings 长度 2 且 CNY holding 的 `market_value_usd == 200`。
  - `by_market(..., "US")`：原生总市值 120、成本 100、盈亏 20。
  - `by_account(..., "acct-us")`：账户名、市场、分布和 holdings 与旧实现一致。
  - `by_category(..., "growth", "成长", "#1677ff", "USD")`：总市值 320、总成本 260、市场分布与 USD holdings 正确。

  浮点断言统一使用本模块 helper，避免直接比较计算结果：

  ```rust
  fn assert_close(actual: f64, expected: f64) {
      assert!((actual - expected).abs() < 1e-9, "{actual} != {expected}");
  }

  #[test]
  fn overview_preserves_cross_currency_totals_and_exposes_same_holdings() {
      let (model, rates) = fixture();
      let result = overview(&model, &rates, "USD");
      assert_close(result.total_market_value, 320.0);
      assert_close(result.total_cost, 260.0);
      assert_close(result.total_pnl, 60.0);
      assert_close(result.total_pnl_percent, 60.0 / 260.0 * 100.0);
      assert_eq!(result.holdings.len(), 2);
      let cn = result.holdings.iter().find(|item| item.market == "CN").unwrap();
      assert_close(cn.market_value_usd, 200.0);
  }
  ```

- [ ] **Step 4: 在读模型中仅为单元测试增加构造器**：

  ```rust
  #[cfg(test)]
  pub(crate) fn from_holdings_for_test(holdings: Vec<HoldingDetail>) -> Self {
      Self { holdings }
  }
  ```

- [ ] **Step 5: 将 `commands/statistics.rs` 的四段聚合代码原样移到**：

  ```rust
  pub fn overview(
      model: &PortfolioReadModel,
      rates: &ExchangeRates,
      base_currency: &str,
  ) -> StatisticsOverview;

  pub fn by_market(model: &PortfolioReadModel, market: &str) -> MarketStatistics;
  pub fn by_account(model: &PortfolioReadModel, account_id: &str) -> AccountStatistics;
  pub fn by_category(
      model: &PortfolioReadModel,
      rates: &ExchangeRates,
      base_currency: &str,
      category_id: &str,
      category_name: &str,
      category_color: &str,
  ) -> CategoryStatistics;
  ```

  Overview 与 Category 返回 `model.holdings_with_usd(rates)` 中各自需要的 rows；Market/Account 保持 native values。

- [ ] **Step 6: command 只负责编排**：每个 command 调用一次 `PortfolioReadModel::load(..., CacheOnly)`，需要汇率的 Overview/Category 各加载一次真实 rates；Category 继续用现有严格 `load_category`，缺失类别沿用当前回退。

- [ ] **Step 7: 运行**：

  ```bash
  cargo test --manifest-path src-tauri/Cargo.toml statistics_service
  cargo test --manifest-path src-tauri/Cargo.toml commands::statistics
  cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features -- -D warnings
  ```

### Task 5: 建立单视图 Statistics store 与纯路由决策

**Files:**
- Create: `src/stores/statisticsStore.ts`
- Create: `src/stores/statisticsStore.test.ts`
- Modify: `src/stores/dashboardStore.ts`
- Create: `src/pages/Statistics/statisticsView.ts`
- Create: `src/pages/Statistics/statisticsView.test.ts`

**Interfaces:**
- Produces: `StatisticsView` union、`fetchView(view)`、`resolveStatisticsView(...)`。
- Removes: `dashboardStore.ts` 中混杂的 Statistics store。

- [ ] **Step 1: 定义明确的请求 union 与稳定 cache key**：

  ```ts
  export type StatisticsView =
    | { kind: "overview"; baseCurrency: Currency }
    | { kind: "market"; market: string }
    | { kind: "account"; accountId: string }
    | { kind: "category"; categoryId: string; baseCurrency: Currency };

  export function statisticsViewKey(view: StatisticsView): string {
    switch (view.kind) {
      case "overview": return `overview:${view.baseCurrency}`;
      case "market": return `market:${view.market}`;
      case "account": return `account:${view.accountId}`;
      case "category": return `category:${view.categoryId}:${view.baseCurrency}`;
    }
  }
  ```

- [ ] **Step 2: 先写 RED store 测试**，对四个 view 分别调用 `fetchView`，逐次断言只产生一个命令及准确参数：

  ```ts
  assert.deepEqual(calls, [
    ["get_statistics_overview", { baseCurrency: "USD" }],
    ["get_statistics_by_market", { market: "US" }],
    ["get_statistics_by_account", { accountId: "acct-us" }],
    ["get_statistics_by_category", { categoryId: "growth", baseCurrency: "CNY" }],
  ]);
  ```

  再写失败用例，断言某 view 失败不会清空另外三个 view 的成功缓存，并在 `errorByView[viewKey]` 记录错误。

- [ ] **Step 3: 实现可注入 `createStatisticsStore(invokeFn = invoke)`**。`fetchView` 用 exhaustive `switch` 选择一个 command，设置对应的 `loadingByView`/`errorByView`，成功后只更新目标 map 或 overview。`marketStats`/`accountStats` 继续分别按 market/account id 缓存；`categoryStats` 改按 `categoryId:baseCurrency` 缓存，避免切换基准货币显示旧类别结果。

- [ ] **Step 4: 从 `dashboardStore.ts` 删除 `StatisticsState` 与 `useStatisticsStore`；所有统计页 import 改到 `../../stores/statisticsStore`。

  `CategoryTab` 读取结果时使用下面的复合 key，与 Step 3 的缓存规则保持一致：

  ```ts
  const stats = categoryStats[`${selectedCategoryId}:${baseCurrency}`];
  ```

- [ ] **Step 5: 先写 RED 纯调度测试并实现**：

  ```ts
  export interface StatisticsSelection {
    activeTab: "overview" | "market" | "account" | "category";
    baseCurrency: Currency;
    selectedMarket: string;
    selectedAccountId: string;
    selectedCategoryId: string;
  }

  export function resolveStatisticsView(
    selection: StatisticsSelection
  ): StatisticsView | null;
  ```

  测试整体/市场总能返回一个 view；账户或类别 id 为空时返回 `null`；货币只进入 overview/category view，不进入 market/account view。

- [ ] **Step 6: 运行**：

  ```bash
  node --test src/stores/statisticsStore.test.ts src/pages/Statistics/statisticsView.test.ts
  bun run build
  ```

### Task 6: 让 Statistics 父页面成为唯一请求调度者

**Files:**
- Modify: `src/pages/Statistics/index.tsx`
- Modify: `src/pages/Statistics/MarketTab.tsx`
- Modify: `src/pages/Statistics/AccountTab.tsx`
- Modify: `src/pages/Statistics/CategoryTab.tsx`
- Modify: `src/pages/Statistics/OverviewTab.tsx`

**Interfaces:**
- Produces: 首次/切换/选择/货币/刷新均只请求当前活动视图。
- Removes: 子页签 mount fetch、Statistics 对 `holdingStore` 的依赖、Overview 对 quote/account/category/exchange-rate stores 的持仓重建。

- [ ] **Step 1: 父页面初次 mount 只并行加载账户、类别和 overview**。删除 `fetchHoldings()` 与 `fetchHoldingQuotes([])`；available markets 从 `overview?.holdings` 推导：

  ```ts
  const availableMarkets = useMemo(() => {
    const markets = new Set((overview?.holdings ?? []).map((holding) => holding.market));
    return VALID_MARKETS.filter((market) => markets.has(market as Market));
  }, [overview]);
  ```

- [ ] **Step 2: 新增统一 helper**：

  ```ts
  const loadCurrentView = useCallback(
    (overrides: Partial<StatisticsSelection> = {}) => {
      const view = resolveStatisticsView({
        activeTab,
        baseCurrency,
        selectedMarket,
        selectedAccountId,
        selectedCategoryId,
        ...overrides,
      });
      return view ? fetchView(view) : Promise.resolve();
    }, [activeTab, baseCurrency, selectedMarket, selectedAccountId, selectedCategoryId, fetchView]
  );
  ```

- [ ] **Step 3: 事件规则严格实现**：
  - `handleTabChange(tab)`：立即更新 active tab，并用 `{ activeTab: tab }` 解析、fetch 目标 view；
  - 市场/账户/类别选择变化：更新选择；只有对应 tab 当前活动时 fetch 新 view；
  - 货币变化：更新 base；仅当活动 tab 是 overview/category 时 fetch；market/account 不 fetch；
  - 首个账户/类别被自动选中：仅当对应 tab 活动时 fetch；
  - 手动刷新行情：保持账户页只刷新该账户 symbols 的优化，然后只 `await loadCurrentView()`，不得 Promise.all 四个统计接口。

- [ ] **Step 4: 删除三个子页签中的数据请求 effects**：
  - `MarketTab` 不再调用 `fetchMarketStats`；
  - `AccountTab` 不再调用 `fetchAccounts` 或 `fetchAccountStats`；
  - `CategoryTab` 不再调用 `fetchCategories` 或 `fetchCategoryStats`。

  子页签只读取 store 中已缓存结果和父页面传入的 selection handler。

- [ ] **Step 5: Overview 直接消费 `overview.holdings`**。删除 `useQuoteStore`、`useExchangeRateStore`、`useCategoryStore`、`useAccountStore`；聚合时使用后端已提供的 `account_name`、`category_name`、`category_color` 与 `market_value_usd`。跨币种仓位百分比基于 USD 合计；该比例与任意统一基准货币下的比例等价。

- [ ] **Step 6: 修正 loading/error 显示**：父页面和各 tab 用 `statisticsViewKey(view)` 读取当前 view 的 `loadingByView`/`errorByView`；失败只显示当前 view 错误，不能清空其他 tab 缓存。

- [ ] **Step 7: 运行 Node 测试和 build**：

  ```bash
  node --test src/stores/statisticsStore.test.ts src/pages/Statistics/statisticsView.test.ts src/pages/Statistics/categoryHoldings.test.ts
  bun run build
  ```

- [ ] **Step 8: 手动冒烟并观察 Tauri 调用**：首次进入只看到 overview；依次切换 market/account/category 每次只出现目标 command；切换货币时 market/account 不请求；刷新时只重新请求当前 tab。

### Task 7: 全量回归、边界扫描与提交

**Files:**
- Verify: all files changed in Tasks 1–6

**Interfaces:**
- Consumes: 单请求读模型、Dashboard report、纯统计聚合和单视图前端调度。
- Produces: 一个可独立回退的读模型性能提交。

- [ ] **Step 1: 运行架构边界扫描**：

  ```bash
  rg -n 'crate::commands::dashboard|build_holding_details_pub|get_holdings_with_quotes' src src-tauri/src
  rg -n 'commands::dashboard::get_dashboard_summary|pub async fn get_dashboard_summary' src-tauri/src
  rg -n 'fetchOverview|fetchMarketStats|fetchAccountStats|fetchCategoryStats' src/pages/Statistics src/stores
  ```

  三条都必须无输出；AI tool 的业务名称字符串 `get_dashboard_summary` 不在第二条的旧 Rust command 模式内，继续保留。

- [ ] **Step 2: 运行所有新增定向测试**：

  ```bash
  node --test src/stores/dashboardStore.test.ts src/stores/statisticsStore.test.ts src/pages/Statistics/statisticsView.test.ts
  cargo test --manifest-path src-tauri/Cargo.toml portfolio_read_service
  cargo test --manifest-path src-tauri/Cargo.toml statistics_service
  cargo test --manifest-path src-tauri/Cargo.toml commands::statistics
  ```

- [ ] **Step 3: 运行完整质量门禁**：

  ```bash
  bun run check
  git diff --check
  ```

- [ ] **Step 4: 审查完整 diff**，逐项确认：没有迁移；没有报价刷新策略变化；市场/账户仍为 native；整体/类别仍按 base；缺失报价仍为零；Dashboard store 只做原子更新；Statistics 隐藏 tab 没有 effect fetch。

- [ ] **Step 5: 提交**：

  ```bash
  git add src src-tauri
  git commit -m "perf: reuse portfolio read models"
  ```
