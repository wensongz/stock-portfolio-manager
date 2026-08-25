# 期权操作复盘第一版 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在「操作复盘」中增加一个按账户、周期和个股分析已完成CSP/CC Campaign的页签，并让页面与AI Skill复用同一套确定性指标。

**Architecture:** Rust `option_review_service` 从原始 `option_records` 做FIFO配对、推定Campaign和指标聚合，Tauri命令与AI工具只复用并过滤这一结果。React把现有股票复盘提取为独立页签，新期权页签通过Zustand加载报告；AI入口只预激活Skill并预填问题，不自动发送。

**Tech Stack:** Tauri 2、Rust、rusqlite、chrono、React 19、TypeScript 7、Ant Design 6、Zustand、React Router 7。

**Spec:** `docs/superpowers/specs/2026-08-24-option-operation-review-design.md`

## Global Constraints

- 第一版不新增数据库表或迁移。
- 只有所有期权腿都结束的Campaign进入绩效指标；进行中Campaign始终展示但不进入指标。
- 核心口径只有净权利金、权利金留存率、担保名义资本年化收益率和最差已完成Campaign。
- 不计算或展示完整策略总回报、同期持股基准、最大浮动回撤或Greeks归因。
- Campaign必须标记为系统推定；无法安全匹配的记录必须排除并报告，禁止用0补齐。
- 页面只展示确定性事实与事实标签；AI负责自然语言解释，不得重新计算核心金额。
- `get_option_positions` 保留用于当前仓位；历史复盘使用新的 `get_option_review`。
- AI入口只预填问题，不自动调用模型。
- 不引入新的npm或Cargo依赖。

## File Map

**Create**

- `src-tauri/src/models/option_review.rs`：序列化给页面和AI的复盘模型。
- `src-tauri/src/services/option_review_service.rs`：FIFO配对、拆股匹配、Campaign归组、聚合和标签。
- `src-tauri/src/commands/option_review.rs`：Tauri参数边界与命令入口。
- `src/stores/optionReviewStore.ts`：期权复盘报告加载状态。
- `src/pages/Review/StockReviewTab.tsx`：现有股票复盘内容。
- `src/pages/Review/OptionReviewTab.tsx`：期权复盘界面。
- `src/pages/Review/optionReviewViewModel.ts`：前端默认选股、格式化与排序纯函数。
- `src/pages/Review/optionReviewViewModel.test.ts`：前端纯函数测试。
- `src/pages/AiAssistant/prefill.ts`：一次性路由预填解析纯函数。
- `src/pages/AiAssistant/prefill.test.ts`：预填解析测试。

**Modify**

- `src-tauri/src/models/mod.rs`：注册并导出复盘模型。
- `src-tauri/src/services/mod.rs`：注册复盘服务。
- `src-tauri/src/commands/mod.rs`：注册复盘命令模块。
- `src-tauri/src/lib.rs`：注册 `get_option_review` Tauri命令。
- `src-tauri/src/services/ai_tools.rs`：定义、分发和执行 `get_option_review` AI工具。
- `src-tauri/src/skills/options-review.md`：改为使用确定性复盘数据。
- `src-tauri/src/services/skill_service.rs`：内置Skill版本从4逐步升级，最终版本为6。
- `docs/ai-tools.md`：记录新工具和用途。
- `src/types/index.ts`：加入复盘报告类型。
- `src/pages/Review/index.tsx`：变为页签容器。
- `src/pages/AiAssistant/index.tsx`：消费一次性预填问题并补工具中文标签。
- `src/components/ai/ToolCallCard.tsx`：为新AI工具补充中文显示名。

---

### Task 1: 期权复盘模型与确定性计算服务

**Files:**
- Create: `src-tauri/src/models/option_review.rs`
- Create: `src-tauri/src/services/option_review_service.rs`
- Modify: `src-tauri/src/models/mod.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Test: `src-tauri/src/services/option_review_service.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: 现有 `Database`、`option_records`、`option_share_lots`、`stock_splits`。
- Produces: `pub fn get_option_review(db: &Database, account_id: &str, period_days: Option<i64>) -> Result<OptionReviewReport, String>`。
- Produces: `OptionReviewReport`、`OptionReviewSummary`、`OptionUnderlyingReview`、`OptionCampaign`、`OptionWorstCampaign`、`OptionReviewDataQuality`。

- [x] **Step 1: 创建模型文件和最小测试夹具，让服务测试先因模块缺失失败**

在 `src-tauri/src/services/option_review_service.rs` 写入测试模块，先引用尚未实现的 `get_option_review`：

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Database;

    fn db_with_account() -> (Database, String) {
        let db = Database::new(":memory:").expect("in-memory db");
        let account_id = "acct-option-review".to_string();
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO accounts (id, name, market, created_at, updated_at)
             VALUES (?1, 'Options', 'US', '2026-01-01', '2026-01-01')",
            rusqlite::params![account_id],
        ).unwrap();
        drop(conn);
        (db, account_id)
    }

    fn insert_record(
        db: &Database,
        id: &str,
        account_id: &str,
        symbol: &str,
        underlying: &str,
        expiry: &str,
        strike: f64,
        option_type: &str,
        action: &str,
        code: &str,
        quantity: i64,
        amount: f64,
        commission: f64,
        fee: f64,
        traded_at: Option<&str>,
    ) {
        let conn = db.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO option_records
             (id, account_id, option_symbol, underlying, expiry_date, strike_price,
              option_type, action, code, quantity, price, amount, commission, fee,
              traded_at, settled_at, created_at, contract_status)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, 0, ?11, ?12, ?13, ?14, NULL, '2026-01-01', 'active')",
            rusqlite::params![
                id, account_id, symbol, underlying, expiry, strike, option_type,
                action, code, quantity, amount, commission, fee, traded_at
            ],
        ).unwrap();
    }

    #[test]
    fn expired_put_keeps_premium_net_of_fees() {
        let (db, account_id) = db_with_account();
        insert_record(&db, "o1", &account_id, "AAPL 20FEB26 100 P", "AAPL", "20FEB26", 100.0, "P", "SELL", "O", 1, 200.0, 1.0, 0.2, Some("2026-01-15"));
        insert_record(&db, "c1", &account_id, "AAPL 20FEB26 100 P", "AAPL", "20FEB26", 100.0, "P", "BUY", "C;Ep", 1, 0.0, 0.5, 0.1, Some("2026-02-20"));

        let report = get_option_review(&db, &account_id, None).unwrap();
        assert_eq!(report.summary.completed_campaigns, 1);
        assert!((report.summary.gross_premium - 200.0).abs() < 1e-9);
        assert!((report.summary.net_premium_pnl - 198.2).abs() < 1e-9);
        assert_eq!(report.underlyings[0].campaigns[0].strategy_path, vec!["CSP"]);
    }
}
```

在 `models/mod.rs` 和 `services/mod.rs` 暂时只增加 `pub mod option_review;` / `pub mod option_review_service;`，保证测试能定位文件。

- [x] **Step 2: 运行单测确认失败**

Run: `cd src-tauri && cargo test --lib option_review_service::tests::expired_put_keeps_premium_net_of_fees -- --nocapture`

Expected: FAIL，错误包含 `cannot find function get_option_review` 或缺失复盘模型。

- [x] **Step 3: 实现序列化模型**

在 `src-tauri/src/models/option_review.rs` 定义：

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionReviewReport {
    pub account_id: String,
    pub currency: String,
    pub period_days: Option<i64>,
    pub generated_at: String,
    pub summary: OptionReviewSummary,
    pub underlyings: Vec<OptionUnderlyingReview>,
    pub data_quality: OptionReviewDataQuality,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionReviewSummary {
    pub completed_campaigns: usize,
    pub active_campaigns: usize,
    pub gross_premium: f64,
    pub net_premium_pnl: f64,
    pub retention_rate: Option<f64>,
    pub annualized_yield_on_notional: Option<f64>,
    pub worst_campaign: Option<OptionWorstCampaign>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionWorstCampaign {
    pub campaign_id: String,
    pub underlying: String,
    pub started_at: String,
    pub ended_at: String,
    pub strategy_path: Vec<String>,
    pub net_premium_pnl: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionUnderlyingReview {
    pub underlying: String,
    pub completed_campaigns: usize,
    pub active_campaigns: usize,
    pub gross_premium: f64,
    pub net_premium_pnl: f64,
    pub retention_rate: Option<f64>,
    pub annualized_yield_on_notional: Option<f64>,
    pub worst_campaign_pnl: Option<f64>,
    pub flags: Vec<String>,
    pub campaigns: Vec<OptionCampaign>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionCampaign {
    pub id: String,
    pub underlying: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub status: String,
    pub inferred: bool,
    pub strategy_path: Vec<String>,
    pub gross_premium: f64,
    pub close_cost: f64,
    pub fees: f64,
    pub net_premium_pnl: Option<f64>,
    pub secured_notional: f64,
    pub capital_days: f64,
    pub retention_rate: Option<f64>,
    pub annualized_yield_on_notional: Option<f64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct OptionReviewDataQuality {
    pub excluded_open_campaigns: usize,
    pub unmatched_records: usize,
    pub missing_trade_dates: usize,
    pub notes: Vec<String>,
}
```

在 `models/mod.rs` 加 `pub mod option_review;` 和 `pub use option_review::*;`。

- [x] **Step 4: 实现记录读取、日期解析、FIFO周期配对的最小路径**

服务内创建私有结构 `RawOptionRecord`、`OpenLot`、`OptionCycle`，并实现：

```rust
pub fn get_option_review(
    db: &Database,
    account_id: &str,
    period_days: Option<i64>,
) -> Result<OptionReviewReport, String> {
    get_option_review_at(db, account_id, period_days, chrono::Utc::now().date_naive())
}

fn get_option_review_at(
    db: &Database,
    account_id: &str,
    period_days: Option<i64>,
    today: chrono::NaiveDate,
) -> Result<OptionReviewReport, String> {
    let period_days = period_days.map(|days| days.clamp(1, 3650));
    let (market, records, share_lots, splits) = load_inputs(db, account_id)?;
    let currency = match market.as_str() {
        "CN" => "CNY",
        "HK" => "HKD",
        _ => "USD",
    }.to_string();
    let (cycles, mut quality) = pair_cycles_fifo(records, &share_lots, &splits);
    let campaigns = group_campaigns(account_id, cycles, today);
    let filtered = filter_campaigns(campaigns, period_days, today);
    quality.excluded_open_campaigns = filtered.iter().filter(|c| c.status == "active").count();
    Ok(build_report(account_id, currency, period_days, filtered, quality))
}

fn parse_trade_date(raw: &str) -> Option<chrono::NaiveDate> {
    let date = raw.trim().split([',', ' ']).next()?;
    ["%Y-%m-%d", "%Y/%m/%d", "%d%b%y"]
        .iter()
        .find_map(|fmt| chrono::NaiveDate::parse_from_str(date, fmt).ok())
}

fn safe_ratio(numerator: f64, denominator: f64) -> Option<f64> {
    (denominator.abs() > f64::EPSILON).then_some(numerator / denominator)
}
```

FIFO规则必须按单位金额分摊：

```rust
let matched = open.remaining_quantity.min(close.remaining_quantity);
let open_fraction = matched as f64 / open.original_quantity as f64;
let close_fraction = matched as f64 / close.original_quantity as f64;
let gross_premium = open.amount * open_fraction;
let close_cost = close.amount * close_fraction;
let fees = (open.commission.abs() + open.fee.abs()) * open_fraction
    + (close.commission.abs() + close.fee.abs()) * close_fraction;
```

上面的一比一数量适用于相同期权标识。跨标识拆股匹配必须使用适用的 `ratio_from:ratio_to` 把双方数量换算为同一敞口单位，以有理数或整数敞口安全保存剩余数量，并分别用实际消耗的开仓数量和结束数量计算两端分摊比例；不得直接对未调整张数取 `min`。

从DB读取后把 `quantity.abs()` 归一化为正数匹配数量；按 `traded_at`、同时间戳SELL优先于BUY、最后按 `id` 稳定排序，确保导入顺序不改变FIFO结果。部分匹配后的开仓余量必须用同样的比例分摊为active周期，不得丢弃。

完全未匹配的开仓生成 `status = "active"` 周期。`unmatched_records` 统计“存在任何未匹配数量的结束记录数”，使用record ID去重，不把一条2张的记录计为2条。缺交易日期的记录增加 `missing_trade_dates` 并排除。拆股跨标识匹配只有在候选开仓唯一时才执行；出现多个同样可能的开仓时归入 `unmatched_records`，不猜测。

测试一律给 `get_option_review_at` 的 `today` 参数传 `NaiveDate::from_ymd_opt(2026, 8, 24).unwrap()`，避免“最近365天”随实际日期漂移。`load_inputs` 对不存在的账户返回错误，对存在但没有期权记录的账户返回空报告。

- [x] **Step 5: 实现Campaign归组、聚合和事实标签**

归组连接函数必须严格表达规格：

```rust
fn should_connect(previous: &OptionCycle, next: &OptionCycle) -> bool {
    if next.opened_at <= previous.effective_end() {
        return true;
    }
    let gap = (next.opened_at - previous.effective_end()).num_days();
    (previous.option_type == next.option_type && gap <= 7)
        || (previous.option_type == "P"
            && previous.status == "assigned"
            && next.option_type == "C"
            && gap <= 30)
}
```

`group_campaigns` 不能只比较当前Campaign的最后一个周期；必须使用 `current_cycles.iter().any(|cycle| should_connect(cycle, &next))`，保留“长周期与后续周期重叠、中间又夹了一个短周期”时的传递连接。Campaign ID使用稳定组合 `option-review:{account_id}:{underlying}:{started_at}:{ordinal}`，不引入随机ID。策略路径映射固定为Put → `CSP`、Call → `Covered Call`，并按首次出现顺序去重。

Campaign字段按周期求和；有任一active周期时，`status = "active"` 且 `net_premium_pnl`、`retention_rate`、`annualized_yield_on_notional` 均为 `None`。已完成Campaign使用：

```rust
let net = gross_premium - close_cost - fees;
let retention_rate = safe_ratio(net, gross_premium);
let annualized = safe_ratio(net * 365.0, capital_days);
```

标签函数返回稳定顺序：`净亏损`、`低留存`、`单次损失较大`、`高留存`、`样本不足`、`有进行中仓位`。只有满足定义时加入，避免互相矛盾的 `高留存` 与 `低留存` 同时出现。

- [x] **Step 6: 补齐失败测试覆盖关键口径**

在同一测试模块增加下表的具体fixture和断言；所有日期都通过 `get_option_review_at` 固定在2026-08-24，所有浮点值断言误差 `< 1e-9`。

| 测试 | 记录/fixture | 必须断言 |
|---|---|---|
| `losing_buyback_has_negative_retention` | 1张Put：SELL 200，BUY 260，两端总费用2 | `net=-62`、`retention=-0.31` |
| `partial_close_allocates_amounts_and_fees_fifo` | SELL 2张：amount 400/commission 2/fee 0.4；两次BUY各1张：amount 50和70，每次commission 0.5/fee 0.1 | 两个周期 `gross` 各200，合计 `close=120`、`fees=3.6`、`net=276.4` |
| `rolls_within_seven_days_share_a_campaign` | 同标的两个Put，第一个2026-01-10结束，第二个2026-01-17开始 | 1个Campaign、2个周期金额全部进入聚合 |
| `cycles_after_eight_days_are_separate_campaigns` | 同上，第二个改2026-01-18开始 | 2个Campaign，ID不同 |
| `overlap_checks_every_cycle_in_current_campaign` | 周期A为01-01至01-30，B为01-05至01-06，C为01-25至01-26 | A/B/C归为1个Campaign，防止只比较B和C |
| `assigned_put_links_to_call_within_thirty_days` | Put以 `A;C` 于2026-03-01结束，Call于2026-03-31开始 | 1个Campaign，`strategy_path=["CSP", "Covered Call"]` |
| `active_campaign_is_excluded_from_summary` | 一个已结束Put与7天内的未结束Put | `summary.completed=0`、`summary.active=1`、Campaign三个比率/损益字段为 `None` |
| `completed_period_filter_uses_end_date_but_keeps_active` | 已完成Campaign于2025-01-01结束，另一未结束Campaign于2025-01-01开始，`period_days=365` | 旧completed不返回，旧active仍返回 |
| `missing_dates_and_unmatched_closes_are_reported` | 1条 `traded_at=NULL` 开仓 + 1条无对应开仓的BUY | `missing_trade_dates=1`、`unmatched_records=1`，两者不进入Campaign |
| `forward_split_close_conserves_both_record_allocations` | 配置1:2拆股；`BRK B 16JUN23 330 C` SELL 1张与`BRK B 16JUN23 165 C` BUY 2张 | 1个已完成Campaign、`unmatched_records=0`，gross/close/fees/net完整守恒 |
| `reverse_split_close_conserves_both_record_allocations` | 配置2:1反向拆股；`BRK B 31DEC26 165 C` SELL 2张与`BRK B 31DEC26 330 C` BUY 1张 | 1个已完成Campaign、`unmatched_records=0`，gross/close/fees/net完整守恒 |
| `underlying_and_account_totals_equal_campaign_sums` | AAPL和MSFT各2个已完成Campaign | 账户gross/net与个股再与Campaign求和一致 |
| `fact_flags_follow_threshold_boundaries` | 分别构造3个Campaign且留存率恰好70%、恰好40%；正收益中位数100、最差-300与-300.01 | 70%包含`高留存`，40%不包含`低留存`，-300不标记而-300.01标记`单次损失较大` |

每个测试除Campaign数量外，至少断言一个金额/比率字段或数据质量字段，不允许只留空函数体。

- [x] **Step 7: 运行服务单测并修到全部通过**

Run: `cd src-tauri && cargo test --lib option_review_service::tests -- --nocapture`

Expected: 所有 `option_review_service::tests` PASS。

- [x] **Step 8: 运行Rust格式化和全库单测**

Run: `cd src-tauri && cargo fmt --check`

Expected: exit 0；若失败先运行 `cargo fmt`，再重跑检查。

Run: `cd src-tauri && cargo test --lib`

Expected: 所有库测试PASS。

- [x] **Step 9: 提交核心计算服务**

```bash
git add src-tauri/src/models/option_review.rs src-tauri/src/models/mod.rs src-tauri/src/services/option_review_service.rs src-tauri/src/services/mod.rs
git commit -m "feat: add deterministic option review analysis"
```

---

### Task 2: Tauri命令、AI工具与期权复盘Skill

**Files:**
- Create: `src-tauri/src/commands/option_review.rs`
- Modify: `src-tauri/src/commands/mod.rs`
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/src/services/ai_tools.rs`
- Modify: `src-tauri/src/skills/options-review.md`
- Modify: `src-tauri/src/services/skill_service.rs`
- Modify: `docs/ai-tools.md`
- Test: `src-tauri/src/services/ai_tools.rs`
- Test: `src-tauri/src/services/skill_service.rs`

**Interfaces:**
- Consumes: Task 1 `option_review_service::get_option_review`。
- Produces: Tauri命令 `get_option_review(account_id: String, period_days: Option<i64>)`。
- Produces: AI函数工具 `get_option_review(accountId, symbol?, periodDays?, allHistory?)`；`allHistory=true` 明确表示全部历史并覆盖 `periodDays`。

- [x] **Step 1: 先写AI工具定义测试并确认失败**

在 `ai_tools.rs` 测试中把工具数量从20改为21，并增加：

```rust
#[test]
fn option_review_tool_requires_account_and_supports_filters() {
    let defs = tool_definitions();
    let tool = defs.iter()
        .find(|d| d["function"]["name"] == "get_option_review")
        .expect("get_option_review definition");
    assert_eq!(tool["function"]["parameters"]["required"], json!(["accountId"]));
    assert_eq!(tool["function"]["parameters"]["properties"]["periodDays"]["maximum"], 3650);
    assert!(tool["function"]["parameters"]["properties"]["symbol"].is_object());
    assert_eq!(tool["function"]["parameters"]["properties"]["allHistory"]["type"], "boolean");
}
```

在 `skill_service.rs` 测试增加：

```rust
#[test]
fn options_review_uses_deterministic_review_tool() {
    let (_, body) = BUILTIN_SKILLS.iter()
        .find(|(stem, _)| *stem == "options-review")
        .expect("options-review builtin");
    assert!(body.contains("get_option_review"));
    assert!(body.contains("做得好的"));
    assert!(body.contains("值得改进"));
}
```

- [x] **Step 2: 运行测试确认失败**

Run: `cd src-tauri && cargo test --lib ai_tools::tests::option_review_tool_requires_account_and_supports_filters -- --nocapture`

Run: `cd src-tauri && cargo test --lib skill_service::tests::options_review_uses_deterministic_review_tool -- --nocapture`

Expected: FAIL，工具定义不存在且Skill仍只引用 `get_option_positions`。

- [x] **Step 3: 增加Tauri命令并注册**

`src-tauri/src/commands/option_review.rs`：

```rust
use crate::db::Database;
use crate::models::option_review::OptionReviewReport;
use crate::services::option_review_service;
use tauri::State;

#[tauri::command(rename_all = "camelCase")]
pub fn get_option_review(
    db: State<Database>,
    account_id: String,
    period_days: Option<i64>,
) -> Result<OptionReviewReport, String> {
    let account_id = account_id.trim();
    if account_id.is_empty() {
        return Err("accountId 不能为空".to_string());
    }
    option_review_service::get_option_review(&db, account_id, period_days)
}
```

在 `commands/mod.rs` 增加 `pub mod option_review;`，并在 `lib.rs` Options Management段注册 `commands::option_review::get_option_review`。

- [x] **Step 4: 定义并分发AI工具**

在 `tool_definitions()` 加：

```rust
json!({
    "type": "function",
    "function": {
        "name": "get_option_review",
        "description": "确定性期权历史复盘：按账户和可选个股返回已完成Campaign、净权利金、留存率、担保名义资本年化收益率、最差Campaign及数据质量。用于评价CSP/Covered Call哪些做得好、哪些需要改进；不要用于当前到期风险。",
        "parameters": {
            "type": "object",
            "properties": {
                "accountId": { "type": "string", "description": "账户 ID" },
                "symbol": { "type": "string", "description": "可选标的，例如 AAPL" },
                "periodDays": { "type": "integer", "minimum": 1, "maximum": 3650, "default": 365 },
                "allHistory": { "type": "boolean", "description": "true时返回全部历史并覆盖periodDays", "default": false }
            },
            "required": ["accountId"]
        }
    }
})
```

在 `execute_tool` match增加：

```rust
"get_option_review" => tool_option_review(ctx, &args).await,
```

处理函数复用Task 1服务：

```rust
async fn tool_option_review(ctx: &ToolCtx<'_>, args: &Value) -> ToolResult {
    let account_id = match args.get("accountId").and_then(Value::as_str) {
        Some(value) if !value.trim().is_empty() => value.trim(),
        _ => return ToolResult::err_json("缺少参数 accountId"),
    };
    let period_days = if args.get("allHistory").and_then(Value::as_bool).unwrap_or(false) {
        None
    } else {
        Some(
        args.get("periodDays")
            .and_then(Value::as_i64)
            .unwrap_or(365)
            .clamp(1, 3650),
        )
    };
    let report = match option_review_service::get_option_review(ctx.db, account_id, period_days) {
        Ok(report) => report,
        Err(error) => return ToolResult::err_json(format!("期权复盘失败：{error}")),
    };
    let symbol = args.get("symbol").and_then(Value::as_str);
    match option_review_payload(report, symbol) {
        Ok(payload) => ToolResult::ok_json(payload),
        Err(error) => ToolResult::err_json(error),
    }
}
```

将周期解析抽成纯函数并测试：`allHistory=true` 返回 `None` 且覆盖任何 `periodDays`；否则 `periodDays` 省略时返回 `Some(365)`，传入时限制在1到3650。将symbol过滤抽成纯函数 `option_review_payload(mut report: OptionReviewReport, symbol: Option<&str>) -> Result<Value, String>`。空symbol视为未过滤；非空symbol忽略大小写匹配，无结果返回“账户中没有 {symbol} 的期权复盘数据”。过滤后保留原账户summary，并在序列化JSON根对象中追加 `scope_note: "summary为账户级；underlyings已按个股过滤"`；不得把账户summary误称为个股summary。

- [x] **Step 5: 重写Skill并升级内置版本**

`options-review.md`的核心步骤改为：

```markdown
## 分析步骤
1. 确认账户ID和可选标的；最近周期用 `periodDays`，全部历史用 `allHistory=true`，调用 `get_option_review`。
2. 先说明样本量、已完成/进行中Campaign和数据质量。
3. 只引用工具返回的净权利金、留存率、担保名义资本年化收益率和最差Campaign。
4. 分成「✅ 做得好的」「⚠️ 做得不好的」「🔧 值得改进」三段。
5. 每条结论都引用具体Campaign或事实标签；样本不足3个时明确说结论仅供观察。

## 限制
- 不自行把开仓权利金当利润。
- 不把最差Campaign称为最大回撤。
- 不声称已经计算同期持股基准或未平仓浮盈亏。
- Campaign是系统推定，不能描述成用户明确制定的策略链。
```

把 `BUILTIN_SKILLS_VERSION` 从4升级到最终值6，让未被用户编辑的内置Skill在启动时更新；当前实现版本明确为6。

- [x] **Step 6: 更新AI工具文档并运行目标测试**

在 `docs/ai-tools.md` 的期权工具区同时列出：

- `get_option_positions`：当前持仓和到期风险。
- `get_option_review`：历史Campaign和确定性复盘指标；必传 `accountId`，全部历史显式传 `allHistory=true`。

在 `ai_tools.rs` 用手工构造的AAPL/MSFT报告fixture为 `option_review_payload` 增加两个纯函数测试：

```rust
#[test]
fn option_review_symbol_filter_is_case_insensitive_and_preserves_account_summary() {
    let payload = option_review_payload(option_review_fixture(), Some("aapl")).unwrap();
    assert_eq!(payload["summary"]["completed_campaigns"], 2);
    assert_eq!(payload["underlyings"].as_array().unwrap().len(), 1);
    assert_eq!(payload["underlyings"][0]["underlying"], "AAPL");
    assert!(payload["scope_note"].as_str().unwrap().contains("账户级"));
}

#[test]
fn option_review_symbol_filter_reports_missing_symbol() {
    let error = option_review_payload(option_review_fixture(), Some("NVDA")).unwrap_err();
    assert!(error.contains("NVDA"));
}
```

Run: `cd src-tauri && cargo test --lib ai_tools::tests skill_service::tests -- --nocapture`

Expected: 新工具定义、唯一性、Skill解析和内容测试全部PASS；工具总数为21。

- [x] **Step 7: 运行Rust全库验证并提交**

Run: `cd src-tauri && cargo fmt --check`

Run: `cd src-tauri && cargo test --lib`

Expected: 两条命令均exit 0。

```bash
git add src-tauri/src/commands/option_review.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/src/services/ai_tools.rs src-tauri/src/skills/options-review.md src-tauri/src/services/skill_service.rs docs/ai-tools.md
git commit -m "feat: expose option review to app and AI"
```

---

### Task 3: 前端类型、Store与视图模型

**Files:**
- Modify: `src/types/index.ts`
- Create: `src/stores/optionReviewStore.ts`
- Create: `src/pages/Review/optionReviewViewModel.ts`
- Create: `src/pages/Review/optionReviewViewModel.test.ts`

**Interfaces:**
- Consumes: Task 2 Tauri命令 `get_option_review(accountId, periodDays?)`。
- Produces: `useOptionReviewStore`，字段 `report/loading/error` 和方法 `fetchOptionReview(accountId, periodDays)`、`clearOptionReview()`。
- Produces: `selectDefaultUnderlying(report)`、`sortUnderlyingReviews(items)`、`formatReviewPercent(value)`。

- [x] **Step 1: 先写视图模型失败测试**

`optionReviewViewModel.test.ts`：

```typescript
import test from "node:test";
import assert from "node:assert/strict";
import {
  formatReviewPercent,
  selectDefaultUnderlying,
  sortUnderlyingReviews,
} from "./optionReviewViewModel.ts";

const underlying = (symbol: string, pnl: number, flags: string[] = []) => ({
  underlying: symbol,
  completed_campaigns: 1,
  active_campaigns: 0,
  gross_premium: 100,
  net_premium_pnl: pnl,
  retention_rate: 0.5,
  annualized_yield_on_notional: 0.05,
  worst_campaign_pnl: pnl,
  flags,
  campaigns: [],
});

test("underlyings sort by absolute net premium, then symbol", () => {
  const sorted = sortUnderlyingReviews([
    underlying("MSFT", 500, ["高留存"]),
    underlying("AAPL", -200, ["净亏损"]),
    underlying("NVDA", 900, ["低留存"]),
  ] as never);
  assert.deepEqual(sorted.map((row) => row.underlying), ["NVDA", "MSFT", "AAPL"]);
});

test("default selection uses the largest absolute net premium", () => {
  const report = { underlyings: [underlying("MSFT", 500), underlying("AAPL", -200, ["净亏损"])] } as never;
  assert.equal(selectDefaultUnderlying(report), "MSFT");
});

test("percentage formatter preserves negative values and missing state", () => {
  assert.equal(formatReviewPercent(-0.31), "-31.0%");
  assert.equal(formatReviewPercent(null), "—");
});
```

- [x] **Step 2: 运行测试确认失败**

Run: `node --test src/pages/Review/optionReviewViewModel.test.ts`

Expected: FAIL，模块或导出函数不存在。

- [x] **Step 3: 增加与Rust一致的TypeScript类型**

在 `src/types/index.ts` 增加：

```typescript
export interface OptionReviewReport {
  account_id: string;
  currency: Currency;
  period_days: number | null;
  generated_at: string;
  summary: OptionReviewSummary;
  underlyings: OptionUnderlyingReview[];
  data_quality: OptionReviewDataQuality;
}

export interface OptionReviewSummary {
  completed_campaigns: number;
  active_campaigns: number;
  gross_premium: number;
  net_premium_pnl: number;
  retention_rate: number | null;
  annualized_yield_on_notional: number | null;
  worst_campaign: OptionWorstCampaign | null;
}

export interface OptionWorstCampaign {
  campaign_id: string;
  underlying: string;
  started_at: string;
  ended_at: string;
  strategy_path: string[];
  net_premium_pnl: number;
}

export interface OptionUnderlyingReview {
  underlying: string;
  completed_campaigns: number;
  active_campaigns: number;
  gross_premium: number;
  net_premium_pnl: number;
  retention_rate: number | null;
  annualized_yield_on_notional: number | null;
  worst_campaign_pnl: number | null;
  flags: string[];
  campaigns: OptionCampaign[];
}

export interface OptionCampaign {
  id: string;
  underlying: string;
  started_at: string;
  ended_at: string | null;
  status: "completed" | "active";
  inferred: boolean;
  strategy_path: string[];
  gross_premium: number;
  close_cost: number;
  fees: number;
  net_premium_pnl: number | null;
  secured_notional: number;
  capital_days: number;
  retention_rate: number | null;
  annualized_yield_on_notional: number | null;
}

export interface OptionReviewDataQuality {
  excluded_open_campaigns: number;
  unmatched_records: number;
  missing_trade_dates: number;
  notes: string[];
}
```

- [x] **Step 4: 实现视图模型纯函数**

`optionReviewViewModel.ts`：

```typescript
import type { OptionReviewReport, OptionUnderlyingReview } from "../../types";

export function sortUnderlyingReviews(items: OptionUnderlyingReview[]) {
  return [...items].sort((a, b) =>
    Math.abs(b.net_premium_pnl) - Math.abs(a.net_premium_pnl)
    || a.underlying.localeCompare(b.underlying),
  );
}

export function selectDefaultUnderlying(report: OptionReviewReport | null) {
  return report ? sortUnderlyingReviews(report.underlyings)[0]?.underlying ?? null : null;
}

export function formatReviewPercent(value: number | null) {
  return value == null ? "—" : `${(value * 100).toFixed(1)}%`;
}
```

- [x] **Step 5: 实现Zustand Store**

`optionReviewStore.ts`：

```typescript
import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { OptionReviewReport } from "../types";

interface OptionReviewState {
  report: OptionReviewReport | null;
  loading: boolean;
  error: string | null;
  fetchOptionReview: (accountId: string, periodDays: number | null) => Promise<void>;
  clearOptionReview: () => void;
}

export const useOptionReviewStore = create<OptionReviewState>((set) => ({
  report: null,
  loading: false,
  error: null,
  fetchOptionReview: async (accountId, periodDays) => {
    const requestId = ++latestOptionReviewRequest;
    set({ report: null, loading: true, error: null });
    try {
      const report = await invoke<OptionReviewReport>("get_option_review", {
        accountId,
        periodDays: periodDays ?? null,
      });
      if (requestId === latestOptionReviewRequest) set({ report, loading: false });
    } catch (error) {
      if (requestId === latestOptionReviewRequest) {
        set({ report: null, loading: false, error: String(error) });
      }
    }
  },
  clearOptionReview: () => {
    latestOptionReviewRequest += 1;
    set({ report: null, error: null, loading: false });
  },
}));
```

在 `create` 之前定义 `let latestOptionReviewRequest = 0;`。这个请求序号保证用户快速切换账户/周期时，较早请求的慢响应不会覆盖新报告。

- [x] **Step 6: 运行前端纯函数测试与类型检查**

Run: `node --test src/pages/Review/optionReviewViewModel.test.ts`

Expected: 3个测试PASS。

Run: `npx tsc --noEmit`

Expected: exit 0。

- [x] **Step 7: 提交前端数据层**

```bash
git add src/types/index.ts src/stores/optionReviewStore.ts src/pages/Review/optionReviewViewModel.ts src/pages/Review/optionReviewViewModel.test.ts
git commit -m "feat: add option review frontend data model"
```

---

### Task 4: 操作复盘页签与期权复盘界面

**Files:**
- Create: `src/pages/Review/StockReviewTab.tsx`
- Create: `src/pages/Review/OptionReviewTab.tsx`
- Modify: `src/pages/Review/index.tsx`

**Interfaces:**
- Consumes: Task 3 `useOptionReviewStore`、视图模型函数和复盘类型。
- Consumes: 现有 `useReviewStore`、`useAccountStore`、`usePnlColor`。
- Produces: `StockReviewTab` 和 `OptionReviewTab` React组件。

- [x] **Step 1: 提取股票复盘组件并建立构建基线**

先把现有 `src/pages/Review/index.tsx` 的完整内容复制到 `StockReviewTab.tsx`，然后做且只做三个机械变更：把 `ReviewPage` 重命名为 `StockReviewTab`；删除返回值中包含“历史操作复盘”的整个 `Title` 元素；删除因此不再使用的 `HistoryOutlined` 和 `Title` 引用。其余状态、effect、`DecisionQualityTag`、`HoldingTimeline`和JSX逐行保持。

暂时将 `Review/index.tsx` 替换为可编译的单页签容器：

```tsx
import { HistoryOutlined } from "@ant-design/icons";
import { Typography } from "antd";
import StockReviewTab from "./StockReviewTab";

const { Title } = Typography;

export default function ReviewPage() {
  return (
    <div className="space-y-6">
      <Title level={2}><HistoryOutlined /> 操作复盘</Title>
      <StockReviewTab />
    </div>
  );
}
```

Run: `npm run build`

Expected: 构建通过，证明纯提取没有改变现有行为。

- [x] **Step 2: 创建期权复盘界面骨架并连接Store**

`OptionReviewTab`需要：

```tsx
export default function OptionReviewTab() {
  const { accounts, loading: accountsLoading, fetchAccounts } = useAccountStore();
  const { report, loading, error, fetchOptionReview, clearOptionReview } = useOptionReviewStore();
  const [accountId, setAccountId] = useState(() => localStorage.getItem("review_option_account_id") ?? "");
  const [periodDays, setPeriodDays] = useState<number | null>(365);
  const [selectedSymbol, setSelectedSymbol] = useState<string | null>(null);
  const [accountsReady, setAccountsReady] = useState(false);

  useEffect(() => {
    void fetchAccounts().finally(() => setAccountsReady(true));
  }, [fetchAccounts]);
  useEffect(() => {
    if (!accountId || !accountsReady) return;
    if (!accounts.some((account) => account.id === accountId)) {
      localStorage.removeItem("review_option_account_id");
      setAccountId("");
    }
  }, [accountId, accounts, accountsReady]);
  useEffect(() => {
    if (!accountId) { clearOptionReview(); return; }
    localStorage.setItem("review_option_account_id", accountId);
    void fetchOptionReview(accountId, periodDays);
  }, [accountId, periodDays, fetchOptionReview, clearOptionReview]);
  useEffect(() => { setSelectedSymbol(selectDefaultUnderlying(report)); }, [report]);

  return (
    <div className="space-y-4">
      <Space wrap>
        <Select
          aria-label="期权复盘账户"
          value={accountId || undefined}
          placeholder="选择账户"
          loading={accountsLoading}
          onChange={setAccountId}
          options={accounts.map((account) => ({ value: account.id, label: account.name }))}
          style={{ minWidth: 220 }}
        />
        <Select
          aria-label="期权复盘周期"
          value={periodDays == null ? "all" : String(periodDays)}
          onChange={(value) => setPeriodDays(value === "all" ? null : Number(value))}
          options={[{ value: "365", label: "最近365天" }, { value: "all", label: "全部历史" }]}
          style={{ minWidth: 140 }}
        />
      </Space>
      {error ? <Alert type="error" showIcon title={error} /> : null}
      {!accountId ? <Empty description="请先选择账户" /> : null}
      {accountId && loading ? <Spin /> : null}
      {accountId && !loading && report && report.underlyings.length === 0
        ? <Empty description="该账户暂无可复盘的期权记录" />
        : null}
    </div>
  );
}
```

账户ID失效时使用与期权管理页相同的校验effect：账户加载完成后清空不存在的localStorage值。

- [x] **Step 3: 实现四张指标卡与数据质量提示**

使用Ant Design `Row/Col/Card/Statistic/Alert`，每张卡用 `<Col xs={24} sm={12} xl={6}>` 保证响应式。指标卡必须满足：

- 金额用 `Intl.NumberFormat` 和 `report.currency`。
- 百分比用 `formatReviewPercent`。
- 负数使用 `usePnlColor` 或全局亏损色，不能硬编码红涨绿跌方向。
- 最差Campaign显示金额，卡片下方用小字显示标的和日期。
- 没有已完成Campaign时四张卡的值均为 `—`，不能显示0%伪装成实际结果。
- Alert固定包含「Campaign为系统推定」「进行中Campaign未计入绩效」，并按计数追加未匹配和缺日期记录。
- `accountsReady && accounts.length === 0` 时显示“请先创建账户”；选中账户但报告无任何标的时显示“去期权管理导入CSV”。

数据质量描述通过纯函数拼接，不在JSX里嵌套多层三元表达式。

- [x] **Step 4: 实现个股汇总表**

使用 `sortUnderlyingReviews(report.underlyings)`，并设置 `scroll={{ x: 980 }}`。列定义：

```tsx
const underlyingColumns: ColumnsType<OptionUnderlyingReview> = [
  { title: "标的", dataIndex: "underlying" },
  { title: "Campaign", render: (_: unknown, row: OptionUnderlyingReview) => `${row.completed_campaigns} 完成 / ${row.active_campaigns} 进行中` },
  { title: "净权利金", dataIndex: "net_premium_pnl", align: "right" },
  { title: "留存率", dataIndex: "retention_rate", align: "right" },
  { title: "年化收益率", dataIndex: "annualized_yield_on_notional", align: "right" },
  { title: "最差Campaign", dataIndex: "worst_campaign_pnl", align: "right" },
  { title: "事实标签", dataIndex: "flags" },
];
```

表格行点击设置 `selectedSymbol`；选中行使用 `rowClassName` 加轻量主题背景。事实标签颜色只区分注意项、正向事实和中性项，文本始终可见。个股只有进行中Campaign时，该行的净权利金、留存率、年化收益率和最差Campaign都显示 `—`，不把内部聚合用的0显示为已实现绩效。

- [x] **Step 5: 实现选中个股Campaign表**

从 `report.underlyings.find(item => item.underlying === selectedSymbol)` 取详情，按 `started_at` 降序。列：期间、策略路径、状态、毛权利金、买回成本、费用、净权利金、留存率、年化收益率。

进行中Campaign：

- 状态Tag显示「进行中」。
- `net_premium_pnl`、留存率和年化收益率显示 `—`。
- 不能展示开仓毛权利金为盈利。

标题右侧预留 `AI复盘这只股票` 按钮，Task 5再连接行为；当前按钮可用 `disabled` 并带Tooltip「AI入口将在下一任务接入」。

- [x] **Step 6: 在Review页面加入两个页签**

`Review/index.tsx`最终结构：

```tsx
export default function ReviewPage() {
  return (
    <div className="space-y-6">
      <Title level={2}><HistoryOutlined /> 操作复盘</Title>
      <Tabs
        defaultActiveKey="stock"
        items={[
          { key: "stock", label: "股票操作复盘", children: <StockReviewTab /> },
          { key: "options", label: "期权操作复盘", children: <OptionReviewTab /> },
        ]}
      />
    </div>
  );
}
```

不要在第一版增加「复盘总览」空页签。

- [x] **Step 7: 构建与手动响应式验证**

Run: `npm run build`

Expected: TypeScript和Vite构建通过；既有chunk-size advisory可保留。

Run: `npm run tauri dev`

在桌面宽度和约390px内容宽度检查：

- 两个页签可切换。
- 无账户、无期权数据、只有进行中Campaign都有明确空状态。
- 四张卡在窄屏换行，无文本重叠。
- 宽表格使用Ant Design自身横向滚动，页面不整体横向溢出。
- 切换账户、周期、个股不会出现旧数据残留。

- [x] **Step 8: 提交复盘界面**

```bash
git add src/pages/Review/index.tsx src/pages/Review/StockReviewTab.tsx src/pages/Review/OptionReviewTab.tsx
git commit -m "feat: add option operation review tab"
```

---

### Task 5: AI复盘按钮与一次性预填

**Files:**
- Create: `src/pages/AiAssistant/prefill.ts`
- Create: `src/pages/AiAssistant/prefill.test.ts`
- Modify: `src/pages/AiAssistant/index.tsx`
- Modify: `src/components/ai/ToolCallCard.tsx`
- Modify: `src/pages/Review/OptionReviewTab.tsx`

**Interfaces:**
- Consumes: 现有 `useChatStore().setActiveSkillsForNextTurn`、React Router `navigate`/`useLocation`。
- Produces: 路由state `{ prefillPrompt: string }`。
- Produces: `readAiPrefill(state: unknown): string | null`。

- [x] **Step 1: 写一次性预填解析失败测试**

`prefill.test.ts`：

```typescript
import test from "node:test";
import assert from "node:assert/strict";
import { readAiPrefill } from "./prefill.ts";

test("reads a non-empty prefill prompt", () => {
  assert.equal(readAiPrefill({ prefillPrompt: "  复盘 AAPL  " }), "复盘 AAPL");
});

test("rejects missing, blank, and non-string prompts", () => {
  assert.equal(readAiPrefill(null), null);
  assert.equal(readAiPrefill({ prefillPrompt: "  " }), null);
  assert.equal(readAiPrefill({ prefillPrompt: 42 }), null);
});
```

- [x] **Step 2: 运行测试确认失败**

Run: `node --test src/pages/AiAssistant/prefill.test.ts`

Expected: FAIL，模块不存在。

- [x] **Step 3: 实现预填解析并接入AI助手**

`prefill.ts`：

```typescript
export function readAiPrefill(state: unknown): string | null {
  if (!state || typeof state !== "object" || !("prefillPrompt" in state)) return null;
  const value = (state as { prefillPrompt?: unknown }).prefillPrompt;
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}
```

`AiAssistantPage`引入 `useLocation`，读取一次：

```tsx
const location = useLocation();
const [initialPrompt] = useState(() => readAiPrefill(location.state));

useEffect(() => {
  if (!initialPrompt) return;
  navigate(location.pathname, { replace: true, state: null });
}, [initialPrompt, location.pathname, navigate]);
```

使用state初始器而不是随 `location.state` 重算，这样父组件清除路由state后，首次读到的prompt仍然稳定，不会和子组件写入composer的effect竞态。在现有调用点改为 `<ChatPanel sessionId={currentSessionId} navigate={navigate} initialPrompt={initialPrompt} />`，并在 `ChatPanel` props类型中增加 `initialPrompt: string | null`。`ChatPanel`中：

```tsx
const seededPromptRef = useRef<string | null>(null);
useEffect(() => {
  if (!initialPrompt || seededPromptRef.current === initialPrompt) return;
  seededPromptRef.current = initialPrompt;
  setInput((current) => current.trim().length > 0 ? current : initialPrompt);
}, [initialPrompt]);
```

清除路由state只阻止刷新重复注入，不能清除已写入composer的input。

- [x] **Step 4: 连接期权页AI按钮**

在 `OptionReviewTab`：

```tsx
const navigate = useNavigate();
const setActiveSkillsForNextTurn = useChatStore((state) => state.setActiveSkillsForNextTurn);
const selectedAccount = accounts.find((account) => account.id === accountId) ?? null;
const selectedUnderlying = report?.underlyings.find(
  (item) => item.underlying === selectedSymbol,
) ?? null;

const handleAiReview = () => {
  if (!selectedUnderlying || !selectedAccount) return;
  setActiveSkillsForNextTurn(["options-review"]);
  const period = periodDays == null ? "全部历史" : `最近 ${periodDays} 天`;
  const toolArguments = periodDays == null
    ? { accountId: selectedAccount.id, symbol: selectedUnderlying.underlying, allHistory: true }
    : { accountId: selectedAccount.id, symbol: selectedUnderlying.underlying, periodDays, allHistory: false };
  navigate("/ai-assistant", {
    state: {
      prefillPrompt: `请复盘账户 ${selectedAccount.name}（accountId: ${selectedAccount.id}）在${period}的 ${selectedUnderlying.underlying} 期权交易。请调用 get_option_review，工具参数为 ${JSON.stringify(toolArguments)}。分别说明做得好的、做得不好的和最值得改进的地方。请使用确定性期权复盘数据并说明样本限制。`,
    },
  });
};
```

移除Task 4的disabled状态。按钮只有选中个股时可用。

在 `src/pages/AiAssistant/index.tsx` 和 `src/components/ai/ToolCallCard.tsx` 的 `TOOL_LABELS` 都加：

```typescript
get_option_review: "期权操作复盘",
```

- [x] **Step 5: 运行测试和构建**

Run: `node --test src/pages/AiAssistant/prefill.test.ts src/pages/Review/optionReviewViewModel.test.ts`

Expected: 全部PASS。

Run: `npm run build`

Expected: 构建通过。

- [ ] **Step 6: 手动验证AI入口只预填一次**

> 环境限制：受浏览器安全限制，本轮未取得 composer、Skill待激活、刷新后不重复注入及不自动发送的端到端手工证据，因此此项保持未完成。

从期权复盘页选中AAPL并点击按钮，确认：

- 跳到AI助手欢迎页。
- composer出现AAPL、账户名和周期。
- `options-review`显示为待激活Skill。
- 没有自动发送或自动创建会话。
- 刷新AI页面不再次覆盖用户后来输入的内容。

- [x] **Step 7: 提交AI入口**

```bash
git add src/pages/AiAssistant/index.tsx src/pages/AiAssistant/prefill.ts src/pages/AiAssistant/prefill.test.ts src/components/ai/ToolCallCard.tsx src/pages/Review/OptionReviewTab.tsx
git commit -m "feat: connect option review to AI assistant"
```

---

### Task 6: 全面验证、文档一致性与完成检查

**Files:**
- Modify if needed: `docs/ai-tools.md`
- Modify if needed: `docs/superpowers/specs/2026-08-24-option-operation-review-design.md`
- Modify if needed: 本计划中的完成复选框

**Interfaces:**
- Consumes: Tasks 1–5全部产物。
- Produces: 一个测试通过、口径与文档一致的第一版功能。

- [x] **Step 1: 运行前端测试**

Run: `node --test src/pages/AiAssistant/prefill.test.ts src/pages/Review/optionReviewViewModel.test.ts`

Expected: 全部PASS。

- [x] **Step 2: 运行前端生产构建**

Run: `npm run build`

Expected: TypeScript和Vite构建exit 0；已有chunk-size advisory可保留。

- [x] **Step 3: 运行Rust格式、测试和静态检查**

Run: `cd src-tauri && cargo fmt --check`

Run: `cd src-tauri && cargo test --lib`

Run: `cd src-tauri && cargo check`

Expected: 三条命令均exit 0，无新增warning。

- [x] **Step 4: 检查指标守恒和误导性文案**

在测试数据或本地开发数据中逐项验证：

- 账户净权利金等于所有已完成个股净权利金之和。
- 个股净权利金等于该个股所有已完成Campaign之和。
- 毛权利金、买回成本和费用满足 `net = gross - close - fees`。
- 进行中Campaign不进入四张核心卡。
- 页面没有“最大回撤”“同期持股超额”“胜率”或Greeks字样。
- 所有年化收益率旁都有「担保名义资本口径」。
- Campaign表或Alert明确写「系统推定」。

- [x] **Step 5: 检查Git差异与文档**

Run: `git diff --check`

Run: `git status --short`

Expected: 无空白错误；只有本功能相关文件。确认 `docs/ai-tools.md` 的工具数量和名称与代码一致，设计规格状态为「已确认」。

- [x] **Step 6: 最终提交**

如果前面任务提交后还有格式、文案或文档修正：

```bash
git add docs/ai-tools.md docs/superpowers/specs/2026-08-24-option-operation-review-design.md docs/superpowers/plans/2026-08-24-option-operation-review.md src-tauri/src/models/option_review.rs src-tauri/src/models/mod.rs src-tauri/src/services/option_review_service.rs src-tauri/src/services/mod.rs src-tauri/src/commands/option_review.rs src-tauri/src/commands/mod.rs src-tauri/src/lib.rs src-tauri/src/services/ai_tools.rs src-tauri/src/skills/options-review.md src-tauri/src/services/skill_service.rs src/types/index.ts src/stores/optionReviewStore.ts src/pages/Review/index.tsx src/pages/Review/StockReviewTab.tsx src/pages/Review/OptionReviewTab.tsx src/pages/Review/optionReviewViewModel.ts src/pages/Review/optionReviewViewModel.test.ts src/pages/AiAssistant/index.tsx src/pages/AiAssistant/prefill.ts src/pages/AiAssistant/prefill.test.ts src/components/ai/ToolCallCard.tsx
git commit -m "chore: finalize option operation review"
```

如果工作区已经干净，不创建空提交。
