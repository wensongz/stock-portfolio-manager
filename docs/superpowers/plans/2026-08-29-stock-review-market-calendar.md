# 股票操作复盘交易日历生产同步 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 在股票操作复盘首次加载时，自动为查询涉及的 CN、HK、US 市场构建经过官方休市表和宽基指数 K 线验证的交易日历，并把行情覆盖缺口定位到具体市场、证券和日期。

**Architecture:** 新增独立的 `stock_review_calendar` 服务，编译期嵌入 2025–2026 官方休市资源，通过雪球和东方财富两个独立的指数历史行情适配器验证开市日期，再以单个 SQLite 事务发布逐自然日会话与覆盖元数据。`stock_review_service` 只在异步数据准备边界触发同步，所有金融计算继续读取现有权威日历缓存；行情覆盖诊断作为结构化质量问题返回，由前端单独展示。

**Tech Stack:** Rust 2021、Tokio、Rusqlite/SQLite、Serde/JSON、Chrono + chrono-tz、Tauri 2、React 19、TypeScript 7、Ant Design 6、Node 26 test runner

**Spec:** `docs/superpowers/specs/2026-08-29-stock-review-market-calendar-design.md`

## Global Constraints

- 用户打开或刷新股票复盘时零配置同步，不新增 API Key、设置页或手工日历表单。
- 个股 K 线、持仓快照和单个基准缓存都不是交易日历权威；只能使用配置的宽基指数证据。
- 只有完整验证整个发布区间后才能写入 `encodes_closed_dates = 1`。
- 验证失败不得删除或覆盖上一版完整缓存；没有旧缓存时继续返回 `market_calendar_unavailable`。
- 首版官方休市资源只覆盖 2025-01-01 至 2026-12-31；区间外不推测交易日。
- 半日市仍属于开市日；资源只记录工作日全天休市和临时全天闭市。
- 本次不补复权价、完整公司行动或影子组合分红，`comparable_price_only` 警告保留。
- 行情缺口按市场、日期、标准化证券代码稳定排序，最多返回前 20 项，并返回省略数量。
- 日历同步只允许幂等缓存写入，不得修改交易、持仓、注释或纠错数据。
- 每项实现遵循 RED → GREEN，并在任务边界提交一次可独立审查的变更。

## 文件结构与职责

- Create `src-tauri/resources/stock_review_market_holidays.v1.json`：2025–2026 CN/HK/US 官方工作日休市日期、公告版本与来源 URL。
- Create `src-tauri/src/services/stock_review_calendar.rs`：资源加载、市场本地截止日、指数日期证据校验、稳定 revision、并发同步和 SQLite 原子发布。
- Modify `src-tauri/Cargo.toml`、`src-tauri/Cargo.lock`：加入 IANA 时区支持，准确处理美东夏令时。
- Modify `src-tauri/src/services/mod.rs`：注册 `stock_review_calendar` 服务。
- Modify `src-tauri/src/services/quote_service.rs`：提供不经过 fallback 的雪球指数历史入口，并补齐恒生国企指数的东方财富映射。
- Modify `src-tauri/src/services/stock_review_service.rs`：确定实际涉及市场、同步日历、重读缓存、生成精确行情缺口与同步质量问题。
- Modify `src-tauri/src/models/stock_review.rs`：为质量问题增加 `affected_market`，为质量摘要增加缺口总数与省略数。
- Modify `src-tauri/src/services/stock_review_quality.rs`：把缺口计数传入序列化质量摘要，不改变现有覆盖率阈值。
- Modify `src-tauri/src/services/stock_review_persistence.rs`：保持现有候选 revision 对日历会话和覆盖行的摘要校验；仅增加针对新同步路径的回归断言。
- Modify `src/types/index.ts`：同步 Rust 报告契约。
- Modify `src/pages/Review/stockReviewViewModel.ts`：质量问题分组和缺口入口文案的纯函数。
- Modify `src/pages/Review/stockReviewViewModel.test.ts`：前端缺口分组、排序和省略文案测试。
- Modify `src/pages/Review/StockReviewDataQuality.tsx`：独立显示“查看行情缺口”，展示市场、证券和日期。
- Modify `src/pages/Review/StockReviewTab.tsx`：首次加载文案明确正在同步交易日历并生成复盘。

---

### Task 1: 官方休市资源与市场本地截止日

**Files:**
- Create: `src-tauri/resources/stock_review_market_holidays.v1.json`
- Create: `src-tauri/src/services/stock_review_calendar.rs`
- Modify: `src-tauri/src/services/mod.rs`
- Modify: `src-tauri/Cargo.toml`
- Modify: `src-tauri/Cargo.lock`
- Test: `src-tauri/src/services/stock_review_calendar.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: `chrono::DateTime<Utc>`、编译期 `include_str!("../../resources/stock_review_market_holidays.v1.json")`。
- Produces: `MarketHolidayRules`、`load_market_holiday_rules(market, start, end)`、`latest_fully_closed_date(market, now)`，供 Task 2–4 使用。

- [ ] **Step 1: 写资源加载和截止日的失败测试**

在新模块测试区先定义固定日期帮助函数，并写出以下断言：

```rust
fn day(value: &str) -> NaiveDate {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").unwrap()
}

#[test]
fn embedded_rules_cover_only_declared_2025_2026_range() {
    let rules = load_market_holiday_rules("CN", day("2025-01-01"), day("2026-12-31")).unwrap();
    assert_eq!(rules.complete_start, day("2025-01-01"));
    assert_eq!(rules.complete_through, day("2026-12-31"));
    assert!(rules.weekday_closures.contains(&day("2025-02-03")));
    assert!(rules.weekday_closures.contains(&day("2026-10-07")));
    assert!(load_market_holiday_rules("CN", day("2024-12-31"), day("2025-01-02")).is_err());
}

#[test]
fn embedded_rules_reject_duplicates_weekends_and_wrong_years() {
    let malformed = r#"{"revision":"bad","entries":[{"market":"US","year":2026,"source_urls":["https://www.nyse.com/trade/hours-calendars"],"notice_versions":["bad"],"closed_weekdays":["2026-01-01","2026-01-01","2026-01-03"]}]}"#;
    let error = parse_market_holiday_bundle(malformed).unwrap_err();
    assert!(error.contains("duplicate"));
}

#[test]
fn latest_closed_date_uses_exchange_timezone_and_conservative_close_buffer() {
    let before_cn_close = Utc.with_ymd_and_hms(2026, 8, 28, 7, 20, 0).unwrap();
    let after_cn_close = Utc.with_ymd_and_hms(2026, 8, 28, 7, 40, 0).unwrap();
    assert_eq!(latest_fully_closed_date("CN", before_cn_close).unwrap(), day("2026-08-27"));
    assert_eq!(latest_fully_closed_date("CN", after_cn_close).unwrap(), day("2026-08-28"));

    let before_us_close_in_dst = Utc.with_ymd_and_hms(2026, 7, 6, 20, 20, 0).unwrap();
    let after_us_close_in_dst = Utc.with_ymd_and_hms(2026, 7, 6, 20, 40, 0).unwrap();
    assert_eq!(latest_fully_closed_date("US", before_us_close_in_dst).unwrap(), day("2026-07-05"));
    assert_eq!(latest_fully_closed_date("US", after_us_close_in_dst).unwrap(), day("2026-07-06"));
}
```

- [ ] **Step 2: 运行测试并确认因模块/函数不存在而失败**

Run: `cd src-tauri && cargo test --lib stock_review_calendar::tests::embedded_rules -- --nocapture`

Expected: FAIL，错误明确指向 `stock_review_calendar` 尚未注册或目标函数尚未定义。

- [ ] **Step 3: 加入时区依赖并定义资源契约**

在 `src-tauri/Cargo.toml` 的 `[dependencies]` 加入：

```toml
chrono-tz = "0.10"
```

在 `stock_review_calendar.rs` 定义：

```rust
use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use chrono_tz::{America::New_York, Asia::Hong_Kong, Asia::Shanghai, Tz};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};

const HOLIDAY_RESOURCE: &str = include_str!("../../resources/stock_review_market_holidays.v1.json");

#[derive(Debug, Clone, Deserialize)]
struct MarketHolidayBundle {
    revision: String,
    entries: Vec<MarketHolidayEntry>,
}

#[derive(Debug, Clone, Deserialize)]
struct MarketHolidayEntry {
    market: String,
    year: i32,
    source_urls: Vec<String>,
    notice_versions: Vec<String>,
    closed_weekdays: Vec<NaiveDate>,
    #[serde(default)]
    exceptional_closures: Vec<NaiveDate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarketHolidayRules {
    pub market: String,
    pub resource_revision: String,
    pub complete_start: NaiveDate,
    pub complete_through: NaiveDate,
    pub weekday_closures: BTreeSet<NaiveDate>,
    pub source_urls: Vec<String>,
    pub notice_versions: Vec<String>,
}

fn market_clock(market: &str) -> Result<(Tz, NaiveTime), String> {
    match market {
        "CN" => Ok((Shanghai, NaiveTime::from_hms_opt(15, 30, 0).unwrap())),
        "HK" => Ok((Hong_Kong, NaiveTime::from_hms_opt(16, 30, 0).unwrap())),
        "US" => Ok((New_York, NaiveTime::from_hms_opt(16, 30, 0).unwrap())),
        _ => Err(format!("Unsupported stock-review market '{market}'.")),
    }
}

pub fn latest_fully_closed_date(market: &str, now: DateTime<Utc>) -> Result<NaiveDate, String> {
    let (timezone, cutoff) = market_clock(market)?;
    let local = now.with_timezone(&timezone);
    Ok(if local.time() >= cutoff {
        local.date_naive()
    } else {
        local.date_naive().pred_opt().ok_or("Market-local date underflow")?
    })
}
```

`parse_market_holiday_bundle` 必须拒绝：未知市场、空来源、同一市场年份重复、日期年份不匹配、周末出现在 `closed_weekdays`/`exceptional_closures`、两个集合内或集合间日期重复、同一市场年份不连续。加载时把两类闭市日期合并进 `weekday_closures`；`load_market_holiday_rules` 只在请求区间完整落入资源范围时返回成功。

- [ ] **Step 4: 写入经过官方公告核对的首版资源**

资源顶层 revision 固定为 `exchange-holidays-v1-2025-2026`，每个首版条目的 `exceptional_closures` 为 `[]`。六个市场年份条目的 `closed_weekdays` 使用以下精确集合：

```text
CN 2025: 01-01, 01-28..01-31, 02-03..02-04, 04-04, 05-01..05-02, 05-05, 06-02, 10-01..10-03, 10-06..10-08
CN 2026: 01-01..01-02, 02-16..02-20, 02-23, 04-06, 05-01, 05-04..05-05, 06-19, 09-25, 10-01..10-02, 10-05..10-07
HK 2025: 01-01, 01-29..01-31, 04-04, 04-18, 04-21, 05-01, 05-05, 07-01, 10-01, 10-07, 10-29, 12-25..12-26
HK 2026: 01-01, 02-17..02-19, 04-03, 04-06..04-07, 05-01, 05-25, 06-19, 07-01, 10-01, 10-19, 12-25
US 2025: 01-01, 01-20, 02-17, 04-18, 05-26, 06-19, 07-04, 09-01, 11-27, 12-25
US 2026: 01-01, 01-19, 02-16, 04-03, 05-25, 06-19, 07-03, 09-07, 11-26, 12-25
```

资源中保留这些官方来源，notice version 使用公告标题/编号或日历年份：

- CN 2025：[SSE 上证公告〔2024〕38号](https://big5.sse.com.cn/site/cht/www.sse.com.cn/disclosure/announcement/general/c/c_20241223_10767108.shtml)、[SZSE 深证会〔2024〕413号](https://www.szse.cn/disclosure/notice/general/t20241223_611283.html)
- CN 2026：[SSE 上证公告〔2025〕45号](https://www.sse.com.cn/disclosure/announcement/general/c/c_20251222_10802507.shtml)、[SZSE 深证会〔2025〕481号](https://investor.szse.cn/disclosure/notice/general/t20251222_618087.html)
- HK 2025：[HKEX 2025 Calendar](https://www.hkex.com.hk/-/media/HKEX-Market/News/HKEX-Calendar/2025-HKEX-Calendar_r.pdf)
- HK 2026：[HKEX 2026 Index Feed Calendar](https://www.hkex.com.hk/-/media/HKEX-Market/Services/Market-Data-Services/Infrastructure/Index-Feed-Calendar-2026-%28English-%2C-a-%2C-Chinese%29.pdf)
- US 2025–2026：[NYSE Holidays & Trading Hours](https://www.nyse.com/trade/hours-calendars)、[Nasdaq Trading Calendar](https://www.nasdaqtrader.com/trader.aspx?id=calendar)

早收市日期不写入 `closed_weekdays`，因为对日级复盘它们仍是有效交易日。

- [ ] **Step 5: 运行纯资源与时区测试**

Run: `cd src-tauri && cargo test --lib stock_review_calendar::tests -- --nocapture`

Expected: PASS，包含 US 夏令时、CN/HK 时区、资源范围和格式拒绝用例。

- [ ] **Step 6: 提交官方资源与纯加载器**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/resources/stock_review_market_holidays.v1.json src-tauri/src/services/stock_review_calendar.rs src-tauri/src/services/mod.rs
git commit -m "feat(review): add authoritative holiday rules"
```

---

### Task 2: 宽基指数行情适配器、证据校验与稳定 revision

**Files:**
- Modify: `src-tauri/src/services/stock_review_calendar.rs`
- Modify: `src-tauri/src/services/quote_service.rs`
- Test: `src-tauri/src/services/stock_review_calendar.rs` 内 `#[cfg(test)]`
- Test: `src-tauri/src/services/quote_service.rs` 内现有测试模块

**Interfaces:**
- Consumes: Task 1 的 `MarketHolidayRules`，以及雪球/东方财富 provider-specific 历史行情函数。
- Produces: `IndexHistorySource`、`CalendarValidationRequest`、`ValidatedMarketCalendar`、`validate_market_calendar`、`LiveIndexHistorySource`，供 Task 3 同步器使用。

- [ ] **Step 1: 写来源一致性和完整性的失败测试**

使用内存 fake source，覆盖以下确定性场景：

```rust
#[tokio::test]
async fn matching_two_index_two_provider_evidence_validates_every_calendar_day() {
    let request = fixture_request("US", "2026-01-01", "2026-01-09");
    let sources = fake_sources([
        ("xueqiu", "sp500", ["2026-01-02", "2026-01-05", "2026-01-06", "2026-01-07", "2026-01-08", "2026-01-09"]),
        ("xueqiu", "nasdaq_composite", ["2026-01-02", "2026-01-05", "2026-01-06", "2026-01-07", "2026-01-08", "2026-01-09"]),
        ("eastmoney", "sp500", ["2026-01-02", "2026-01-05", "2026-01-06", "2026-01-07", "2026-01-08", "2026-01-09"]),
        ("eastmoney", "nasdaq_composite", ["2026-01-02", "2026-01-05", "2026-01-06", "2026-01-07", "2026-01-08", "2026-01-09"]),
    ]);
    let validated = validate_market_calendar(&request, &sources).await.unwrap();
    assert_eq!(validated.rows.len(), 9);
    assert_eq!(validated.rows[0], (day("2026-01-01"), false));
    assert_eq!(validated.rows[1], (day("2026-01-02"), true));
    assert_eq!(validated.providers, vec!["eastmoney", "xueqiu"]);
}

#[tokio::test]
async fn one_missing_expected_weekday_or_provider_conflict_rejects_publish() {
    let request = fixture_request("US", "2026-01-01", "2026-01-09");
    let error = validate_market_calendar(&request, &fake_source_missing("2026-01-06"))
        .await
        .unwrap_err();
    assert!(matches!(error.kind, CalendarValidationErrorKind::MissingExpectedSession));

    let error = validate_market_calendar(&request, &fake_sources_with_conflict("2026-01-08"))
        .await
        .unwrap_err();
    assert!(matches!(error.kind, CalendarValidationErrorKind::ProviderConflict));
}

#[tokio::test]
async fn one_complete_provider_is_accepted_only_with_full_official_expectation() {
    let request = fixture_request("CN", "2026-02-16", "2026-02-27");
    let validated = validate_market_calendar(&request, &fake_xueqiu_unavailable_eastmoney_complete())
        .await
        .unwrap();
    assert_eq!(validated.providers, vec!["eastmoney"]);
    assert!(validated.warnings.iter().any(|item| item.code == "market_calendar_single_provider"));
}

#[test]
fn revision_is_independent_of_provider_and_date_input_order() {
    assert_eq!(stable_calendar_revision(&fixture_validated_a()), stable_calendar_revision(&fixture_validated_b_reordered()));
}
```

另加普通 K 线落在周末、官方闭市日出现 K 线、两个同提供方指数集合不同、响应标记截断、所有请求失败、请求范围无锚定交易日等拒绝用例。

- [ ] **Step 2: 运行目标测试并确认失败**

Run: `cd src-tauri && cargo test --lib stock_review_calendar::tests::matching_two_index_two_provider_evidence -- --nocapture`

Expected: FAIL，缺少 `IndexHistorySource` 和校验函数。

- [ ] **Step 3: 定义可注入的异步适配器契约和市场指数映射**

在 `stock_review_calendar.rs` 定义对象安全接口：

```rust
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum CalendarProvider { Xueqiu, EastMoney }

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferenceIndex {
    pub logical_name: &'static str,
    pub xueqiu_symbol: &'static str,
    pub eastmoney_symbol: &'static str,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexHistoryEvidence {
    pub provider: CalendarProvider,
    pub logical_index: String,
    pub request_start: NaiveDate,
    pub request_end: NaiveDate,
    pub session_dates: Vec<NaiveDate>,
    pub complete_response: bool,
}

#[derive(Debug, Clone)]
pub struct CalendarValidationRequest {
    pub market: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub rules: MarketHolidayRules,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarSyncWarning {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedMarketCalendar {
    pub market: String,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub resource_revision: String,
    pub rows: Vec<(NaiveDate, bool)>,
    pub providers: Vec<String>,
    pub references: Vec<String>,
    pub warnings: Vec<CalendarSyncWarning>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarValidationErrorKind {
    ResourceUnavailable,
    ProviderUnavailable,
    TruncatedResponse,
    IndexConflict,
    ProviderConflict,
    UnexpectedClosedDateBar,
    MissingExpectedSession,
    MissingAnchorSession,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CalendarValidationError {
    pub kind: CalendarValidationErrorKind,
    pub message: String,
}

pub trait IndexHistorySource: Send + Sync {
    fn provider(&self) -> CalendarProvider;
    fn fetch<'a>(
        &'a self,
        market: &'a str,
        reference: &'a ReferenceIndex,
        start: NaiveDate,
        end: NaiveDate,
    ) -> Pin<Box<dyn Future<Output = Result<IndexHistoryEvidence, String>> + Send + 'a>>;
}

pub async fn validate_market_calendar(
    request: &CalendarValidationRequest,
    sources: &[Arc<dyn IndexHistorySource>],
) -> Result<ValidatedMarketCalendar, CalendarValidationError>;

pub struct LiveIndexHistorySource {
    provider: CalendarProvider,
}

pub fn live_index_history_sources() -> Vec<Arc<dyn IndexHistorySource>> {
    vec![
        Arc::new(LiveIndexHistorySource { provider: CalendarProvider::Xueqiu }),
        Arc::new(LiveIndexHistorySource { provider: CalendarProvider::EastMoney }),
    ]
}
```

`LiveIndexHistorySource::fetch` 的 provider 分派必须直接调用来源专属函数：

```rust
let prices = match self.provider {
    CalendarProvider::Xueqiu => match quote_service::fetch_index_history_xueqiu(
        reference.xueqiu_symbol, market, start, end,
    ).await? {
        XueqiuHistoryOutcome::Prices(prices) => prices,
        XueqiuHistoryOutcome::StartsAfterRange { first_available_date } => {
            return Err(format!("Xueqiu index history starts after the requested range at {first_available_date}."));
        }
        XueqiuHistoryOutcome::Empty => {
            return Err("Xueqiu returned no index sessions for the anchored request.".to_string());
        }
    },
    CalendarProvider::EastMoney => quote_service::fetch_stock_history_eastmoney(
        reference.eastmoney_symbol, market, start, end,
    ).await?,
};
Ok(IndexHistoryEvidence {
    provider: self.provider,
    logical_index: reference.logical_name.to_string(),
    request_start: start,
    request_end: end,
    session_dates: prices.into_iter().map(|(date, _)| date).collect(),
    complete_response: true,
})
```

只有 HTTP/解析成功且 provider 明确响应请求边界时才设置 `complete_response: true`；错误、超时和雪球 `StartsAfterRange` 不得转换为成功空集合。

`reference_indices` 必须返回以下 provider-specific 代码：

```rust
match market {
    "CN" => [("sse_composite", "SH000001", "^SSEC"), ("shenzhen_component", "SZ399001", "399001.SZ")],
    "HK" => [("hang_seng", "HKHSI", "^HSI"), ("hang_seng_china_enterprises", "HKHSCEI", "^HSCEI")],
    "US" => [("sp500", ".INX", "^GSPC"), ("nasdaq_composite", ".IXIC", "^IXIC")],
    _ => return Err(format!("Unsupported stock-review market '{market}'.")),
}
```

- [ ] **Step 4: 暴露严格保留来源身份的指数历史入口**

在 `quote_service.rs` 把雪球请求的公共部分抽为：

```rust
async fn fetch_history_xueqiu_api_symbol(
    api_symbol: &str,
    display_symbol: &str,
    market: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<XueqiuHistoryOutcome, String>;

pub(crate) async fn fetch_index_history_xueqiu(
    api_symbol: &str,
    market: &str,
    start_date: NaiveDate,
    end_date: NaiveDate,
) -> Result<XueqiuHistoryOutcome, String>;
```

现有 `fetch_stock_history_xueqiu_outcome` 先做个股代码转换，再调用同一个 helper，保证原有个股 fallback 行为不变。新指数函数直接使用 `.INX`、`.IXIC`、`HKHSI` 等已确认的雪球代码，并且不调用东方财富或 Yahoo fallback，这样 provider provenance 不会丢失。

在 `resolve_index_secid` 增加：

```rust
"HSCE" | "HSCEI" => Some(("100.HSCEI", "恒生中国企业指数")),
```

补测试断言 `resolve_index_secid("^HSCEI")`，并保留既有 `^GSPC`、`^IXIC`、`^HSI` 映射回归。

- [ ] **Step 5: 实现完整验证与稳定摘要**

`validate_market_calendar` 的顺序必须固定：

1. 从 Task 1 资源计算请求范围内 `weekday && !official_closed` 的期望开市集合。
2. 若范围内没有期望开市日，向左扩展证据请求起点，直到包含最近一个资源覆盖内开市日；只发布原请求范围的自然日行。
3. 对每个 provider 并发请求两个参考指数；同一 provider 只有两个请求均成功、`complete_response = true` 且日期集合相同才算成功。
4. 过滤请求范围外日期，排序、去重；周末或官方闭市日期出现 K 线立即失败。
5. 一个成功 provider 也必须与官方期望集合完全相同；第二 provider 成功时必须与第一 provider 相同。
6. 至少一个 provider 成功才生成逐自然日 `(date, is_session)`。

稳定 revision 使用现有仓库 FNV-1a 风格，不使用随机 hash：

```rust
pub fn stable_calendar_revision(calendar: &ValidatedMarketCalendar) -> String {
    let mut digest = StableCalendarDigest::new("stock-review-calendar-v1");
    digest.write(&calendar.resource_revision);
    digest.write(&calendar.market);
    let mut providers = calendar.providers.clone();
    providers.sort();
    providers.dedup();
    for provider in providers {
        digest.write(&provider);
    }
    let mut references = calendar.references.clone();
    references.sort();
    references.dedup();
    for reference in references {
        digest.write(&reference);
    }
    for (date, is_session) in &calendar.rows {
        digest.write(&date.format("%Y-%m-%d").to_string());
        digest.write(if *is_session { "1" } else { "0" });
    }
    format!("{}:{:016x}", calendar.resource_revision, digest.finish())
}
```

provider 名称和逻辑参考指数都进入 revision，但先排序去重；相同证据不会因为来源返回顺序不同而产生无意义更新，单来源升级为双来源时会得到可审计的新 revision。

- [ ] **Step 6: 运行适配器和校验测试**

Run: `cd src-tauri && cargo test --lib stock_review_calendar::tests -- --nocapture`

Expected: PASS。

Run: `cd src-tauri && cargo test --lib quote_service::tests::resolve_index_secid_handles_common_forms -- --nocapture`

Expected: PASS。

Run: `cd src-tauri && cargo test --lib quote_service::tests::test_parse_xueqiu_kline -- --nocapture`

Expected: PASS，既有雪球日线日期解析不回退。

- [ ] **Step 7: 提交指数证据层**

```bash
git add src-tauri/src/services/stock_review_calendar.rs src-tauri/src/services/quote_service.rs
git commit -m "feat(review): validate calendar from index sessions"
```

---

### Task 3: 原子日历缓存、并发串行化和失败保旧

**Files:**
- Modify: `src-tauri/src/services/stock_review_calendar.rs`
- Test: `src-tauri/src/services/stock_review_calendar.rs` 内 `#[cfg(test)]`
- Test: `src-tauri/src/db/tests.rs`

**Interfaces:**
- Consumes: Task 2 的 `ValidatedMarketCalendar` 和 `LiveIndexHistorySource`。
- Produces: `CalendarSyncRequest`、`CalendarSyncOutcome`、`sync_market_calendar_with_sources`、`sync_market_calendars_with_sources`、`sync_market_calendars`，供 Task 4 调用。

- [ ] **Step 1: 写持久化、回滚、幂等和并发失败测试**

新增这些精确测试：

```rust
#[tokio::test]
async fn first_sync_writes_every_natural_day_and_complete_coverage() {
    let db = Database::new(":memory:").unwrap();
    let sources = complete_fake_sources();
    let outcome = sync_market_calendar_with_sources(
        &db,
        fixture_sync_request("US", "2026-01-01", "2026-01-09"),
        &sources,
    ).await;
    assert_eq!(outcome.status, CalendarSyncStatus::Published);
    assert_eq!(row_count(&db, "stock_market_sessions", "US"), 9);
    assert_eq!(coverage_flag(&db, "US"), 1);
}

#[tokio::test]
async fn coverage_write_failure_rolls_back_session_rows() {
    let db = Database::new(":memory:").unwrap();
    install_coverage_abort_trigger(&db);
    let result = publish_validated_calendar(&db, &fixture_validated_a(), None);
    assert!(result.is_err());
    assert_eq!(row_count(&db, "stock_market_sessions", "US"), 0);
    assert_eq!(row_count(&db, "stock_market_calendar_coverage", "US"), 0);
}

#[tokio::test]
async fn refresh_failure_preserves_last_complete_revision() {
    let db = Database::new(":memory:").unwrap();
    publish_validated_calendar(&db, &fixture_validated_a(), None).unwrap();
    let before = coverage_revision(&db, "US");
    let sources = conflicting_sources();
    let outcome = sync_market_calendar_with_sources(&db, extended_request(), &sources).await;
    assert_eq!(outcome.status, CalendarSyncStatus::StaleCacheUsed);
    assert_eq!(coverage_revision(&db, "US"), before);
}

#[tokio::test]
async fn same_market_concurrent_syncs_publish_one_coherent_revision() {
    let db = Arc::new(Database::new(":memory:").unwrap());
    let source = Arc::new(CountingFakeSource::complete());
    let sources: Vec<Arc<dyn IndexHistorySource>> = vec![source.clone()];
    let (left, right) = tokio::join!(
        sync_market_calendar_with_sources(&db, fixture_request_a(), &sources),
        sync_market_calendar_with_sources(&db, fixture_request_a(), &sources),
    );
    assert!(left.status.is_success());
    assert!(right.status.is_success());
    assert_calendar_rows_match_coverage(&db, "US");
    assert_eq!(distinct_calendar_revisions(&db, "US"), 1);
}
```

再覆盖：旧覆盖向前扩展、向后扩展、资源 revision 改变时全区间重验、相同 revision 不更新 `updated_at`、请求超资源范围只保留旧缓存并返回结构化原因。

- [ ] **Step 2: 运行持久化测试并确认失败**

Run: `cd src-tauri && cargo test --lib stock_review_calendar::tests::first_sync_writes_every_natural_day -- --nocapture`

Expected: FAIL，缺少同步和发布函数。

- [ ] **Step 3: 定义同步结果和市场级异步锁**

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CalendarSyncStatus { Reused, Published, StaleCacheUsed, Unavailable }

impl CalendarSyncStatus {
    pub fn is_success(&self) -> bool {
        matches!(self, Self::Reused | Self::Published | Self::StaleCacheUsed)
    }
}

#[derive(Debug, Clone)]
pub struct CalendarSyncRequest {
    pub market: String,
    pub required_start: NaiveDate,
    pub required_through: NaiveDate,
}

#[derive(Debug, Clone)]
pub struct CalendarSyncOutcome {
    pub market: String,
    pub requested_start: NaiveDate,
    pub requested_through: NaiveDate,
    pub available_start: Option<NaiveDate>,
    pub available_through: Option<NaiveDate>,
    pub status: CalendarSyncStatus,
    pub issue_code: Option<String>,
    pub message: Option<String>,
    pub warnings: Vec<CalendarSyncWarning>,
}

pub async fn sync_market_calendar_with_sources(
    db: &Database,
    request: CalendarSyncRequest,
    sources: &[Arc<dyn IndexHistorySource>],
) -> CalendarSyncOutcome;

pub async fn sync_market_calendars(
    db: &Database,
    markets: &BTreeSet<String>,
    required_start: NaiveDate,
    now: DateTime<Utc>,
) -> Vec<CalendarSyncOutcome>;

pub async fn sync_market_calendars_with_sources(
    db: &Database,
    markets: &BTreeSet<String>,
    required_start: NaiveDate,
    now: DateTime<Utc>,
    sources: &[Arc<dyn IndexHistorySource>],
) -> Vec<CalendarSyncOutcome>;

fn publish_validated_calendar(
    db: &Database,
    calendar: &ValidatedMarketCalendar,
    expected_coverage_revision: Option<&str>,
) -> Result<CalendarSyncStatus, String>;
```

生产 `sync_market_calendars` 仅构造 `live_index_history_sources()` 并委托给 `sync_market_calendars_with_sources`；所有测试调用后者，保证生产与 fake source 走同一增量/发布路径。

用 `OnceLock<BTreeMap<&'static str, tokio::sync::Mutex<()>>>` 为 CN/HK/US 各建一把锁。锁只包住同市场的“重读覆盖 → 拉取验证 → 事务发布”流程，不持有 `rusqlite::Connection` 锁跨越 `.await`。

- [ ] **Step 4: 实现增量决策和失败保旧**

同步流程按以下可验证状态机实现：

```text
read valid coverage
  ├─ covers request + resource revision matches -> Reused
  ├─ resource revision changed -> validate full union(existing, request)
  └─ revision matches but missing front/back -> validate missing contiguous segment(s)

validation failed
  ├─ old coverage exists -> StaleCacheUsed + market_calendar_refresh_failed
  └─ no old coverage -> Unavailable + market_calendar_sync_failed

validation succeeded
  └─ publish rows + coverage in one transaction -> Published
```

前后增量段发布前必须与现有覆盖相邻；资源版本变化必须重验完整并集。若段内没有开市日，证据请求向左包含一个最近开市锚点，但只写目标段。

- [ ] **Step 5: 在单一 SQLite 事务中发布会话和覆盖**

`publish_validated_calendar` 必须：

1. 获取 `db.conn`，立即开启 transaction。
2. 重读当前 coverage revision；若与 `expected_coverage_revision` 不同，回滚并让调用者重读后重试一次。
3. `DELETE` 只删除本次已完整重验的目标范围。
4. 批量 `INSERT` 每个自然日的 `market/date/is_session/source/updated_at`。
5. 计算最终连续覆盖范围，验证数据库中每天恰好一行且 flag 只能是 0/1。
6. 同一 transaction 内 upsert `stock_market_calendar_coverage`，最后写 `encodes_closed_dates = 1`。
7. commit 后调用现有 `load_market_sessions` 重新读取，并以 `MarketCalendar::covers` 证明发布范围可用。

若相同 revision 已存在，直接返回 `Reused`，不改行、不刷新 `updated_at`。

- [ ] **Step 6: 运行同步与数据库回归测试**

Run: `cd src-tauri && cargo test --lib stock_review_calendar::tests -- --nocapture`

Expected: PASS。

Run: `cd src-tauri && cargo test --lib db::tests::stock_review_tables_are_created_and_resettable -- --nocapture`

Expected: PASS，确认没有引入新表且 factory reset 仍覆盖现有日历表。

- [ ] **Step 7: 提交原子同步器**

```bash
git add src-tauri/src/services/stock_review_calendar.rs src-tauri/src/db/tests.rs
git commit -m "feat(review): atomically sync market calendars"
```

---

### Task 4: 接入复盘准备边界并只同步实际涉及市场

**Files:**
- Modify: `src-tauri/src/services/stock_review_service.rs`
- Modify: `src-tauri/src/services/stock_review_persistence.rs`
- Test: `src-tauri/src/services/stock_review_service.rs` 内 `#[cfg(test)]`
- Test: `src-tauri/src/services/stock_review_persistence.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: Task 3 的 `sync_market_calendars` 和 `CalendarSyncOutcome`。
- Produces: 首次加载自动同步后的 `CachedStockReviewInput`；测试注入入口 `get_stock_review_report_with_calendar_sources`；候选纠错继续使用相同的日历 revision 边界。

- [ ] **Step 1: 把原有“缺日历必阻断”用例改写为失败的首次自动同步用例**

不要删除 `missing_authoritative_calendar_suppresses_exact_session_metrics` 的安全含义；把纯无来源路径保留为 fake source 失败测试，并增加：

```rust
#[tokio::test]
async fn empty_calendar_first_report_syncs_only_scoped_us_and_unlocks_session_metrics() {
    let db = complete_live_review_db_without_calendars("US");
    let sources = complete_fake_sources();
    let report = get_stock_review_report_with_calendar_sources(
        &db,
        live_query("2026-01-02", "2026-08-28", Some("US")),
        &sources,
        fixed_now("2026-08-28T22:00:00Z"),
    ).await.unwrap();
    assert!(!report.data_quality.issues.iter().any(|issue| issue.code == "market_calendar_unavailable"));
    assert_ne!(report.summary.forward_effect.day_60.status.status, MetricStatus::Unavailable);
    assert_eq!(calendar_markets(&db), vec!["US"]);
}

#[tokio::test]
async fn unavailable_source_without_old_cache_keeps_exact_metrics_blocked() {
    let db = complete_live_review_db_without_calendars("US");
    let sources = all_sources_failed();
    let report = get_stock_review_report_with_calendar_sources(
        &db,
        live_query("2026-01-02", "2026-08-28", Some("US")),
        &sources,
        fixed_now("2026-08-28T22:00:00Z"),
    ).await.unwrap();
    assert!(report.data_quality.issues.iter().any(|issue| issue.code == "market_calendar_sync_failed"));
    assert!(report.data_quality.issues.iter().any(|issue| issue.code == "market_calendar_unavailable"));
}

#[tokio::test]
async fn stale_complete_cache_remains_usable_and_reports_refresh_warning() {
    let db = complete_live_review_db_with_old_valid_calendar("US");
    let sources = conflicting_sources();
    let report = get_stock_review_report_with_calendar_sources(
        &db,
        live_query("2026-01-02", "2026-08-28", Some("US")),
        &sources,
        fixed_now("2026-08-28T22:00:00Z"),
    ).await.unwrap();
    assert!(report.data_quality.issues.iter().any(|issue| issue.code == "market_calendar_refresh_failed"));
    assert!(!report.data_quality.issues.iter().any(|issue| issue.code == "market_calendar_unavailable"));
}
```

再加 unfiltered 单市场持仓不会同步另外两个市场、个股停牌日不改变会话表、同步后 calendar revision 被候选 source digest 捕获的测试。

测试入口只在 `#[cfg(test)]` 下定义，并与生产入口共享同一个 preparation core：

```rust
#[cfg(test)]
async fn get_stock_review_report_with_calendar_sources(
    db: &Database,
    query: StockReviewQuery,
    sources: &[Arc<dyn IndexHistorySource>],
    now: DateTime<Utc>,
) -> Result<StockReviewReport, String> {
    let input = prepare_cached_stock_review_input_with_calendar_sources(
        db, query, None, None, sources, now, |_| Ok(()),
    ).await?;
    build_stock_review_report_from_cached_data(&input)
}
```

- [ ] **Step 2: 运行集成测试并确认首次报告仍被阻断**

Run: `cd src-tauri && cargo test --lib stock_review_service::tests::empty_calendar_first_report_syncs_only_scoped_us -- --nocapture`

Expected: FAIL，当前准备流程在同步前直接读取空日历。

- [ ] **Step 3: 在安全异步边界确定市场并触发同步**

在 `prepare_cached_stock_review_input_with_calendar_sources` 中，完成 `security_keys` 和 `price_start` 后执行：

```rust
let review_markets = required_review_markets(&query, &security_keys);
let calendar_outcomes = stock_review_calendar::sync_market_calendars_with_sources(
    db,
    &review_markets,
    price_start,
    now,
    sources,
).await;
```

生产 wrapper 构造一次 `live_index_history_sources()` 并把 `Utc::now()` 传给内部 core；测试 wrapper 传 fake sources 和固定时间。内部签名固定为：

```rust
async fn prepare_cached_stock_review_input_with_calendar_sources<F>(
    db: &Database,
    query: StockReviewQuery,
    candidate: Option<StockReviewOverride>,
    candidate_revision: Option<&mut stock_review_persistence::ValidatedOverrideCandidate>,
    sources: &[Arc<dyn IndexHistorySource>],
    now: DateTime<Utc>,
    after_cache_fill: F,
) -> Result<CachedStockReviewInput, String>
where
    F: FnOnce(&Database) -> Result<(), String>;
```

`required_review_markets` 只包含：

- 当前 scoped transaction/current holding 的市场；
- 显式 `query.market`；
- 不因默认混合基准把没有持仓/交易的 CN、HK、US 全部加入。

把 `benchmark_specs` 改为 `benchmark_specs(query, &review_markets)`；未指定自定义基准时只为实际市场构建固定期初权重混合基准。显式单市场筛选永远只同步该市场。

- [ ] **Step 4: 把同步结果映射为结构化质量问题并重读权威缓存**

同步返回后再调用 `load_market_sessions`。问题映射固定为：

```rust
CalendarSyncStatus::Published | CalendarSyncStatus::Reused => None,
CalendarSyncStatus::StaleCacheUsed => Some(StockReviewIssue {
    code: "market_calendar_refresh_failed".into(),
    severity: StockReviewIssueSeverity::Warning,
    message: outcome.message.clone().unwrap(),
    affected_symbol: None,
    affected_date: outcome.available_through,
}),
CalendarSyncStatus::Unavailable => Some(StockReviewIssue {
    code: "market_calendar_sync_failed".into(),
    severity: StockReviewIssueSeverity::Warning,
    message: outcome.message.clone().unwrap(),
    affected_symbol: None,
    affected_date: None,
}),
```

此外把 `outcome.warnings` 全部映射为 `Warning` issue；例如单来源成功发布时保留 `market_calendar_single_provider`，message 必须带 outcome market。这样“第二来源不可用”不会阻断报告，也不会静默消失。Task 5 扩展公共 issue 契约后，再把这些 calendar issue 的 `affected_market` 设为对应市场。

现有 `market_calendar_unavailable` 改为根据 `calendar.covers(price_start, required_through)` 产生，而不是只检查 `calendar.availability.status`；message 应包含该市场可用覆盖边界。部分旧缓存可以用于其覆盖内计算，但不能冒充完整请求范围；网络错误不可提升为 command failure。

- [ ] **Step 5: 使用每市场已完整收盘终点而不是盘中当天推断**

把每个 outcome 的 `requested_through` 传给 `portfolio_valuation_session_authority` 和 forward action 完整性判断：

```rust
let required_through = query.end_date.min(calendar_terminal_by_market[market]);
calendar.covers(authority_start, required_through)
```

多市场 expected session 为各市场 `authority_start..=required_through` 会话的并集。当前市场尚未收盘时，不要求日历含当天；历史范围缺口仍会阻断。`nth_market_session_after`、Campaign 和实际 NAV 都继续使用同一缓存会话集合。

- [ ] **Step 6: 验证候选 revision 仍覆盖日历写入**

保留 `cache_source_revision` 对 `stock_market_sessions` 和 `stock_market_calendar_coverage` 的查询。新增回归测试：在 candidate pin 之后改变 calendar revision，`verify_candidate_source_revision` 必须失败；相同 revision 的幂等同步不得导致候选失效。

Run: `cd src-tauri && cargo test --lib stock_review_persistence::tests::prepared_override_rejects_a_changed_full_review_source_revision -- --nocapture`

Expected: PASS。

- [ ] **Step 7: 运行复盘集成测试**

Run: `cd src-tauri && cargo test --lib stock_review_service::tests -- --nocapture`

Expected: PASS，原有“非权威单行会话不能解锁指标”和“覆盖中间缺日无效”测试保持通过。

- [ ] **Step 8: 提交复盘接入**

```bash
git add src-tauri/src/services/stock_review_service.rs src-tauri/src/services/stock_review_persistence.rs
git commit -m "fix(review): sync calendars before report preparation"
```

---

### Task 5: 精确行情覆盖缺口与公共报告契约

**Files:**
- Modify: `src-tauri/src/models/stock_review.rs`
- Modify: `src-tauri/src/services/stock_action_builder.rs`
- Modify: `src-tauri/src/services/stock_campaign_builder.rs`
- Modify: `src-tauri/src/services/stock_review_service.rs`
- Modify: `src-tauri/src/services/stock_review_quality.rs`
- Modify: `src-tauri/src/services/stock_review_persistence.rs`
- Modify: `src-tauri/src/db/tests.rs`
- Test: `src-tauri/src/services/stock_review_service.rs` 内 `#[cfg(test)]`
- Test: `src-tauri/src/services/stock_review_quality.rs` 内 `#[cfg(test)]`

**Interfaces:**
- Consumes: Task 4 重读后的 `market_calendars_by_market`、`prices_by_security` 和 valuation interval。
- Produces: `MarketCoverageAssessment`；公开的 `StockReviewIssue.affected_market`、`StockReviewDataQuality.market_price_gap_total`、`market_price_gap_omitted`，供 Task 6 UI 使用。

- [ ] **Step 1: 写 94.4%、非交易日和 20 项上限的失败测试**

```rust
#[test]
fn market_coverage_reports_the_exact_missing_point_for_seventeen_of_eighteen() {
    let prices = two_us_symbols_with_one_of_eighteen_session_points_missing("MSFT", day("2026-01-08"));
    let calendars = us_calendar_with_nine_sessions();
    let assessment = assess_market_coverage(&prices, &calendars, day("2026-01-02"), day("2026-01-14"));
    assert!((assessment.coverage_ratio.unwrap() - 17.0 / 18.0).abs() < 1e-12);
    assert_eq!(assessment.total_gaps, 1);
    assert_eq!(assessment.issues[0].code, "market_price_gap");
    assert_eq!(assessment.issues[0].affected_market.as_deref(), Some("US"));
    assert_eq!(assessment.issues[0].affected_symbol.as_deref(), Some("MSFT"));
    assert_eq!(assessment.issues[0].affected_date, Some(day("2026-01-08")));
}

#[test]
fn closed_market_dates_are_not_denominator_or_gap() {
    let assessment = assess_market_coverage(&prices_without_us_holiday(), &calendar_marking_holiday_closed(), day("2026-01-01"), day("2026-01-02"));
    assert_eq!(assessment.coverage_ratio, Some(1.0));
    assert!(assessment.issues.is_empty());
}

#[test]
fn market_gaps_are_stable_sorted_capped_and_count_omitted() {
    let assessment = assessment_with_25_gaps_in_unsorted_input();
    assert_eq!(assessment.total_gaps, 25);
    assert_eq!(assessment.issues.len(), 20);
    assert_eq!(assessment.omitted_gaps, 5);
    assert!(assessment.issues.windows(2).all(|pair| issue_sort_key(&pair[0]) <= issue_sort_key(&pair[1])));
}
```

- [ ] **Step 2: 运行 94.4% 用例并确认失败**

Run: `cd src-tauri && cargo test --lib stock_review_service::tests::market_coverage_reports_the_exact_missing_point -- --nocapture`

Expected: FAIL，现有 `aggregate_market_coverage` 只返回 `Option<f64>`。

- [ ] **Step 3: 扩展 Rust 公共契约并机械更新现有构造器**

```rust
pub struct StockReviewDataQuality {
    pub availability: MetricAvailability,
    pub actual_result_availability: MetricAvailability,
    pub shadow_value_add_availability: MetricAvailability,
    pub attribution_availability: MetricAvailability,
    pub forward_effect_availability: MetricAvailability,
    pub issues: Vec<StockReviewIssue>,
    pub market_data_coverage: Option<f64>,
    pub exchange_rate_coverage: Option<f64>,
    pub interval_drawdown_only: bool,
    pub market_price_gap_total: u32,
    pub market_price_gap_omitted: u32,
}

pub struct StockReviewIssue {
    pub code: String,
    pub severity: StockReviewIssueSeverity,
    pub message: String,
    pub affected_market: Option<String>,
    pub affected_symbol: Option<String>,
    pub affected_date: Option<NaiveDate>,
}
```

给所有非市场问题构造器显式加 `affected_market: None`；`market_calendar_authority`、`market_calendar_unavailable`、`market_calendar_refresh_failed`、`market_calendar_sync_failed`、`market_calendar_single_provider` 使用 `Some(market.clone())`。`QualityInput` 和 `CachedStockReviewInput` 都增加 `market_price_gap_total: u32`、`market_price_gap_omitted: u32`，`build_stock_review_quality` 原样复制到报告；不要改变 `classify_coverage_status` 的 95%/80% 阈值。

在 `db::tests::stock_review_contract_serializes_status_and_reset_clears_rows` 增加序列化断言，固定 snake_case 字段名称和值。

- [ ] **Step 4: 用结构化 assessment 替换单值聚合**

```rust
#[derive(Debug, Clone)]
struct MarketCoverageAssessment {
    coverage_ratio: Option<f64>,
    issues: Vec<StockReviewIssue>,
    total_gaps: u32,
    omitted_gaps: u32,
}

fn assess_market_coverage(
    prices: &BTreeMap<(String, String), Vec<DailyMarketPoint>>,
    calendars: &BTreeMap<String, MarketCalendar>,
    start: NaiveDate,
    end: NaiveDate,
) -> MarketCoverageAssessment;
```

对每个 `(symbol, market)` 先要求 `calendar.covers(start, end)`，再只枚举该市场权威 calendar 中的开市日期；calendar 不完整时不猜缺口。把不存在精确 `DailyMarketPoint.date` 的点收集为 issue，按 `(market, date, normalized_stock_symbol(symbol))` 排序后截取 20。分母为所有可证明应有行情的 `(security, market_session)`，分子为其中存在精确点的数量。

问题固定格式：

```rust
StockReviewIssue {
    code: "market_price_gap".to_string(),
    severity: StockReviewIssueSeverity::Warning,
    message: format!("{market} 行情缓存缺少 {symbol} 在 {date} 的收盘价。"),
    affected_market: Some(market.clone()),
    affected_symbol: Some(symbol.clone()),
    affected_date: Some(date),
}
```

把前 20 项追加到 `preparation_issues`，计数写入 cached input。覆盖率数值继续进入现有质量分类。

- [ ] **Step 5: 运行覆盖率和质量契约测试**

Run: `cd src-tauri && cargo test --lib stock_review_service::tests::market_coverage -- --nocapture`

Expected: PASS。

Run: `cd src-tauri && cargo test --lib stock_review_quality::tests -- --nocapture`

Expected: PASS，94.4% 仍被分类为 `Degraded`，缺口 issue 不会误改独立指标状态。

- [ ] **Step 6: 提交精确缺口契约**

```bash
git add src-tauri/src/models/stock_review.rs src-tauri/src/services/stock_action_builder.rs src-tauri/src/services/stock_campaign_builder.rs src-tauri/src/services/stock_review_service.rs src-tauri/src/services/stock_review_quality.rs src-tauri/src/services/stock_review_persistence.rs src-tauri/src/db/tests.rs
git commit -m "feat(review): report exact market price gaps"
```

---

### Task 6: 前端同步状态与“查看行情缺口”信息架构

**Files:**
- Modify: `src/types/index.ts`
- Modify: `src/pages/Review/stockReviewViewModel.ts`
- Modify: `src/pages/Review/stockReviewViewModel.test.ts`
- Modify: `src/pages/Review/StockReviewDataQuality.tsx`
- Modify: `src/pages/Review/StockReviewTab.tsx`
- Modify: `src/stores/stockReviewStore.test.ts`

**Interfaces:**
- Consumes: Task 5 的 `affected_market`、`market_price_gap_total`、`market_price_gap_omitted`。
- Produces: `partitionStockReviewIssues`、`marketPriceGapLabel` 和可直接定位缺口的质量面板。

- [ ] **Step 1: 写前端纯函数失败测试**

```typescript
test("market price gaps are separated and preserve stable backend order", () => {
  const issues = [
    { code: "market_calendar_authority", severity: "info", message: "calendar", affected_market: "US", affected_symbol: null, affected_date: null },
    { code: "market_price_gap", severity: "warning", message: "gap", affected_market: "US", affected_symbol: "MSFT", affected_date: "2026-01-08" },
  ];
  assert.deepEqual(partitionStockReviewIssues(issues), {
    gapIssues: [issues[1]],
    otherIssues: [issues[0]],
  });
});

test("market gap label exposes shown total and omitted counts", () => {
  assert.equal(marketPriceGapLabel({ total: 25, omitted: 5 }), "查看 20 / 25 项行情缺口（另 5 项未展示）");
  assert.equal(marketPriceGapLabel({ total: 1, omitted: 0 }), "查看 1 项行情缺口");
});
```

同时更新现有 issue fixtures，加入 `affected_market: null`。

- [ ] **Step 2: 运行视图模型测试并确认失败**

Run: `node --test src/pages/Review/stockReviewViewModel.test.ts`

Expected: FAIL，目标 helper 未导出。

- [ ] **Step 3: 同步 TypeScript 契约并实现纯函数**

在 `src/types/index.ts` 增加：

```typescript
export interface StockReviewDataQuality {
  availability: MetricAvailability;
  actual_result_availability: MetricAvailability;
  shadow_value_add_availability: MetricAvailability;
  attribution_availability: MetricAvailability;
  forward_effect_availability: MetricAvailability;
  issues: StockReviewIssue[];
  market_data_coverage: number | null;
  exchange_rate_coverage: number | null;
  interval_drawdown_only: boolean;
  market_price_gap_total: number;
  market_price_gap_omitted: number;
}

export interface StockReviewIssue {
  code: string;
  severity: StockReviewIssueSeverity;
  message: string;
  affected_market: string | null;
  affected_symbol: string | null;
  affected_date: string | null;
}
```

纯函数只按 `code === "market_price_gap"` 分组，不在前端重新推导日期或覆盖率。同步更新 `stockReviewStore.test.ts` 的报告 fixture：quality 加入两个 0，所有 issue fixture 加入 `affected_market: null`。

实现签名和返回值固定为：

```typescript
export function partitionStockReviewIssues(issues: StockReviewIssue[]) {
  return {
    gapIssues: issues.filter((issue) => issue.code === "market_price_gap"),
    otherIssues: issues.filter((issue) => issue.code !== "market_price_gap"),
  };
}

export function marketPriceGapLabel(input: { total: number; omitted: number }): string {
  const shown = input.total - input.omitted;
  return input.omitted > 0
    ? `查看 ${shown} / ${input.total} 项行情缺口（另 ${input.omitted} 项未展示）`
    : `查看 ${input.total} 项行情缺口`;
}
```

- [ ] **Step 4: 调整质量面板的信息架构**

`StockReviewDataQuality.tsx` 的 Collapse 使用两个 item：

1. `quality-detail`：非 gap 问题和既有计算口径。
2. `market-price-gaps`：仅在 `market_price_gap_total > 0` 时出现，label 使用 `marketPriceGapLabel`。

每个 gap 行按以下顺序显示：严重级别、市场 tag、证券 tag、日期、message。总览仍保留“行情覆盖 94.4%”，并在同一行追加可见的“1 项缺口”提示。不要在 UI 补价格或改变后端状态。

Collapse items 的数据边界固定为：

```tsx
const { gapIssues, otherIssues } = partitionStockReviewIssues(issues);
const items = [
  { key: "quality-detail", label: otherIssues.length ? `查看 ${otherIssues.length} 项数据限制与计算口径` : "查看计算口径", children: qualityDetail },
  ...(quality.market_price_gap_total > 0 ? [{
    key: "market-price-gaps",
    label: marketPriceGapLabel({ total: quality.market_price_gap_total, omitted: quality.market_price_gap_omitted }),
    children: marketGapList,
  }] : []),
];
```

首次无报告加载时，把 `StockReviewTab.tsx` 的 Spin 文案改为：

```tsx
<Spin description="正在同步交易日历并生成股票操作复盘…" />
```

有旧报告时沿用现有行为：继续显示旧报告和 loading 状态；刷新失败后保留旧报告并显示 warning。

- [ ] **Step 5: 运行前端测试和生产构建**

Run: `node --test src/pages/Review/stockReviewViewModel.test.ts src/stores/stockReviewStore.test.ts`

Expected: PASS。

Run: `npm run build`

Expected: PASS，无 TypeScript 契约遗漏。

- [ ] **Step 6: 提交前端呈现**

```bash
git add src/types/index.ts src/pages/Review/stockReviewViewModel.ts src/pages/Review/stockReviewViewModel.test.ts src/pages/Review/StockReviewDataQuality.tsx src/pages/Review/StockReviewTab.tsx src/stores/stockReviewStore.test.ts
git commit -m "feat(review): expose calendar sync and price gaps"
```

---

### Task 7: 全量回归、真实数据库验证与完成记录

**Files:**
- Create: `docs/superpowers/verification/2026-08-29-stock-review-market-calendar.md`
- Verify only: all implementation files from Tasks 1–6
- Runtime database: `/Users/wensongzhang/Library/Application Support/com.portfolio.manager/portfolio.db`（只允许日历缓存幂等写入）

**Interfaces:**
- Consumes: Tasks 1–6 的完整实现。
- Produces: 可复现的自动化测试、构建、真实数据库前后对比和 UI 验证记录。

- [ ] **Step 1: 运行格式、目标测试和全量 Rust 测试**

Run: `cd src-tauri && cargo fmt --check`

Expected: PASS。

Run: `cd src-tauri && cargo test --lib stock_review_calendar::tests -- --nocapture`

Expected: PASS。

Run: `cd src-tauri && cargo test --lib stock_review_service::tests -- --nocapture`

Expected: PASS。

Run: `cd src-tauri && cargo test --lib`

Expected: PASS，0 failed。

Run: `cd src-tauri && cargo check`

Expected: PASS。

- [ ] **Step 2: 运行全部 Node 测试和生产构建**

Run:

```bash
node --test src/hooks/tablePageSize.test.ts src/pages/AiAssistant/prefill.test.ts src/pages/AiAssistant/sidebarPreference.test.ts src/pages/Options/expiredOptionsViewModel.test.ts src/pages/Quarterly/aggregateSnapshotHoldings.test.mjs src/pages/Review/optionReviewViewModel.test.ts src/pages/Review/reviewTabPreference.test.ts src/pages/Review/stockReviewDateBoundary.test.ts src/pages/Review/stockReviewViewModel.test.ts src/pages/Statistics/categoryHoldings.test.ts src/stores/chatStore.test.ts src/stores/optionReviewStore.test.ts src/stores/optionStore.test.ts src/stores/quoteErrors.test.ts src/stores/stockReviewStore.test.ts
```

Expected: PASS，0 failed。

Run: `npm run build`

Expected: PASS。

- [ ] **Step 3: 记录真实数据库运行前基线**

先退出正在运行的应用，再执行只读查询并把输出记入 verification 文档：

```bash
sqlite3 -readonly "/Users/wensongzhang/Library/Application Support/com.portfolio.manager/portfolio.db" "SELECT 'transactions', COUNT(*) FROM transactions UNION ALL SELECT 'holdings', COUNT(*) FROM holdings UNION ALL SELECT 'annotations', COUNT(*) FROM stock_review_annotations UNION ALL SELECT 'overrides', COUNT(*) FROM stock_review_overrides UNION ALL SELECT 'sessions', COUNT(*) FROM stock_market_sessions UNION ALL SELECT 'coverage', COUNT(*) FROM stock_market_calendar_coverage;"
```

再记录用户数据内容摘要，避免只比较行数漏掉更新：

```bash
sqlite3 -readonly "/Users/wensongzhang/Library/Application Support/com.portfolio.manager/portfolio.db" "SELECT 'transactions', total(length(CAST(id AS BLOB)) + length(CAST(traded_at AS BLOB)) + length(CAST(total_amount AS BLOB))) FROM transactions UNION ALL SELECT 'holdings', total(length(CAST(id AS BLOB)) + length(CAST(updated_at AS BLOB)) + length(CAST(shares AS BLOB))) FROM holdings UNION ALL SELECT 'annotations', total(length(CAST(id AS BLOB)) + length(CAST(updated_at AS BLOB)) + length(CAST(value_json AS BLOB))) FROM stock_review_annotations UNION ALL SELECT 'overrides', total(length(CAST(id AS BLOB)) + length(CAST(updated_at AS BLOB)) + length(CAST(value_json AS BLOB))) FROM stock_review_overrides;"
```

- [ ] **Step 4: 有界启动 Tauri 并触发真实股票复盘刷新**

Run: `npm run tauri dev`

在应用中打开“股票操作复盘”，保持现有筛选并点击一次刷新。确认首次加载显示“正在同步交易日历并生成股票操作复盘…”，等待报告出现后停止开发进程。

UI 断言：

- 查询实际涉及的市场不再出现空表导致的 `market_calendar_unavailable`。
- 质量区域可展开查看市场日历 authority 信息。
- 若行情覆盖仍为 94.4%，可以展开看到具体市场、证券和日期。
- `shadow_dividend_source_incomplete` 与 `shadow_degradedreturnmode` 继续显示，证明本修复没有掩盖 price-only 降级。

- [ ] **Step 5: 对比真实数据库并验证第二次刷新幂等**

重新运行 Step 3 两条 SQL。要求 transactions、holdings、annotations、overrides 的行数和摘要完全相同；sessions/coverage 允许增加。

记录日历覆盖：

```bash
sqlite3 -readonly "/Users/wensongzhang/Library/Application Support/com.portfolio.manager/portfolio.db" "SELECT c.market, c.source, c.complete_start, c.complete_through, c.revision, c.encodes_closed_dates, c.updated_at, COUNT(s.date), MAX(s.updated_at) FROM stock_market_calendar_coverage c LEFT JOIN stock_market_sessions s ON s.market = c.market GROUP BY c.market, c.source, c.complete_start, c.complete_through, c.revision, c.encodes_closed_dates, c.updated_at ORDER BY c.market;"
```

再次启动应用、刷新相同报告并退出；重跑上条查询，要求 revision、范围、会话行数、coverage `updated_at` 和 session `MAX(updated_at)` 全部不变。

- [ ] **Step 6: 扫描占位和调试遗留并写验证记录**

Run: `mkdir -p docs/superpowers/verification`

Expected: `docs/superpowers/verification` 存在，目录内不复制任何运行时数据库。

Run: `rg -n "TO[D]O|TB[D]|FIXM[E]|placeholde[r]|console\.log|dbg!" src src-tauri/src src-tauri/resources`

Expected: 没有本次变更新增的匹配项。

在 verification 文档记录：各命令、通过数量、真实数据库前后计数/摘要、日历覆盖范围、第二次刷新幂等结果、保留的 price-only 警告。不要把数据库文件、cookie、完整交易数据或用户证券明细写入仓库。

- [ ] **Step 7: 提交验证记录并确认工作区干净**

```bash
git add docs/superpowers/verification/2026-08-29-stock-review-market-calendar.md
git commit -m "docs(review): verify market calendar sync"
```

Run: `git status --short`

Expected: 无输出。
