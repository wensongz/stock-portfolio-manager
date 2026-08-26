# 期权操作复盘第一版设计

日期：2026-08-24  
状态：已确认

## 背景

现有「期权管理」页面主要回答当前有哪些合约、累计收取了多少权利金、到期或指派状态，以及当前价格下可能需要的现金或正股；现有「操作复盘」页面主要展示季度持仓时间线和手工设置的决策质量。两者都不能回答：在某只个股上持续执行 Cash-secured Put（CSP）和 Covered Call（CC）以后，哪些操作值得重复、哪些操作消耗了权利金、哪些地方需要调整。

应用已经有 `options-review` Skill 和 `get_option_positions` AI 工具，但当前工具只返回合约列表。AI缺少Campaign、净权利金、留存率和资金占用等确定性结果，如果直接让AI自行计算，模型和轮次之间可能出现不同口径。

## 目标

第一版把「操作复盘」升级为股票与期权共用的复盘入口：

- 页面上区分「股票操作复盘」和「期权操作复盘」。
- 期权复盘按账户、周期和个股汇总卖方期权Campaign及已完成绩效。
- 核心金额与指标由Rust服务确定性计算，页面和AI使用同一个结果对象。
- 页面展示事实，AI Skill负责解释“做得好的、做得不好的、值得改进的”。
- 对现有数据无法可靠支持的指标明确显示限制，不生成看似精确的结果。

## 非目标

第一版不实现：

- 实时或历史期权行情、IV、Delta、Gamma、Theta、Vega归因。
- 未平仓期权的每日市值和浮动盈亏。
- 组合最大浮动回撤。
- CSP指派后的股票批次与后续CC之间的人工精确关联。
- 完整策略总回报与同期持股基准比较。
- 手工编辑或拆分Campaign。
- 复盘总览首页和跨账户汇总。

这些能力需要额外的期权行情、股票批次关联或用户确认，留给后续版本。

## 产品结构

「操作复盘」页面保留一个页面标题，下面增加两个页签：

```text
操作复盘
├── 股票操作复盘
└── 期权操作复盘
```

### 股票操作复盘

复用现有页面内容和行为：

- 决策统计。
- 个股选择。
- 季度持仓时间线。
- 手工设置正确、错误、待定。

第一版仅把现有内容移入页签，不改变它的指标口径。

### 期权操作复盘

页面由以下区域组成：

1. 账户与周期选择。
2. 四张核心指标卡。
3. 按个股汇总表。
4. 选中个股的Campaign表。
5. 数据口径提示和「AI复盘这只股票」按钮。

第一版不自动调用AI。按钮会跳转AI助手、预激活 `options-review` Skill，并预填包含账户、个股和周期的复盘问题，由用户确认发送。

## 数据口径

### 分析范围

- 数据源为所选账户的 `option_records`。
- 默认周期为最近365天，可切换到全部历史。
- 已完成Campaign的周期筛选以Campaign结束日期为准；进行中Campaign始终展示，因为它仍代表当前期权风险。
- 每条SELL开仓记录形成一个Campaign；同一开仓记录的部分平仓和剩余敞口属于同一Campaign。
- 累计现金净权利金包含已完成和进行中Campaign；只有已完成Campaign进入留存率、年化收益率、最差Campaign和绩效事实标签。
- 含有未平仓敞口的Campaign标记为「进行中」；其现金净权利金不等于已实现利润，也不包含未平仓市值盈亏。
- 无交易日期或无法匹配开平仓数量的记录不进入指标，并进入数据质量提示。

### 开平仓配对

买卖方向由 `action` 决定；金额字段可能因导入来源带正负号，计算成本时使用绝对值：

- `SELL` 且 `code` 以 `O` 开头：卖出开仓。
- `BUY` 且代码为 `C`、`C;Ep`、`A;C`、`C;P`：结束记录。

同一期权标识内按交易时间FIFO配对开仓和结束数量。部分平仓时，开仓金额、结束金额、佣金和费用均按匹配数量比例分摊。

股票拆分后的跨标识结束记录沿用期权管理的现有拆分匹配规则：同一标的、到期日和期权类型，并根据 `stock_splits` 调整执行价；拆股比例不缩放期权合约张数。一个拆股后结束记录可以按FIFO依次匹配多个拆股前开仓记录，每个开仓记录仍形成独立Campaign；开仓与结束记录使用各自实际消耗的张数比例分摊金额和费用，只有剩余未匹配张数进入数据质量提示。

### 单个期权周期

一次FIFO匹配后的开仓份额称为 `OptionCycle`，包含：

- 标的、类型、执行价、合约数量和每张合约股数。
- 开仓日、结束日和持有天数。
- 状态：到期、指派、平仓或进行中。
- 收取权利金、结束成本、佣金费用和净权利金损益。
- 担保名义资本和资本天数。

计算公式：

```text
gross_premium = SELL开仓金额
close_cost = |BUY结束金额|
fees = 开仓与结束记录的 |commission| + |fee|
net_premium_pnl = gross_premium - close_cost - fees

secured_notional = |contracts| × shares_per_contract × strike_price
holding_days = max(结束日 - 开仓日, 1)
capital_days = secured_notional × holding_days
```

对CSP，`secured_notional`近似现金担保金额；对CC，它是以执行价计算的覆盖名义金额，不等于股票实际成本或市值。页面必须把合并指标标记为「担保名义资本口径」。

### Campaign边界

数据库没有展期链或Wheel链的显式ID。为避免时间重叠、连续卖出或推测的Wheel关系把多笔独立开仓错误合并，Campaign使用稳定、可审计的交易记录边界：

1. 每条SELL开仓记录形成一个Campaign。
2. 由FIFO或拆分换算匹配到该开仓记录的结束份额归入同一Campaign。
3. 同一开仓记录的部分平仓与剩余未平仓份额保留在同一Campaign。
4. 不因时间重叠、7天内连续开仓或Put指派后出现Call而合并不同开仓记录。

Campaign由系统根据交易记录自动推导，页面不得把它描述成用户明确制定的策略链。如果同一开仓记录仍有未平仓份额，该Campaign为进行中；其现金净权利金计入累计净权利金，但不进入已完成绩效指标。

## 核心指标

核心金额和绩效比率采用两种明确口径：累计现金净权利金使用筛选范围内全部Campaign；留存率、年化收益率和最差Campaign只使用已完成Campaign。

### 净权利金

```text
sum(all_filtered_campaign.net_premium_pnl)
```

这是截至当前已经扣除买回成本、佣金和费用后的期权现金流口径结果，页面标记为「累计净权利金（含进行中）」。进行中Campaign的该值不代表已实现利润，也没有计入未平仓合约的市值盈亏。

报告中的 `gross_premium` 和 `net_premium_pnl` 均为上述全Campaign累计口径。为让已完成绩效可以独立审计，报告同时返回 `completed_gross_premium` 和 `completed_net_premium_pnl`；留存率使用后两者，不能用累计字段重建。

### 权利金留存率

```text
sum(completed_campaign.net_premium_pnl) / sum(completed_campaign.gross_premium)
```

结果不截断在0%到100%之间；发生净亏损时可以为负数。

### 年化净权利金收益率

```text
sum(completed_campaign.net_premium_pnl) × 365 / sum(completed_campaign.capital_days)
```

该指标使用担保名义资本，并非账户TWR或IRR。没有有效资本天数时返回空值；页面显示 `—` 和原因，不显示0%。

### 最差已完成Campaign

```text
min(completed_campaign.net_premium_pnl)
```

同时显示Campaign日期和策略路径。第一版不把它称为最大回撤，因为没有未平仓期权的每日估值。

## 事实标签

页面不生成不透明的综合评分。个股行仅显示可解释的事实标签：

- `样本不足`：已完成Campaign少于3个。
- `高留存`：已完成Campaign不少于3个且留存率不低于70%。
- `低留存`：留存率低于40%。
- `净亏损`：已完成Campaign净权利金合计小于0。
- `单次损失较大`：最差Campaign亏损绝对值大于正收益Campaign中位数的3倍；没有正收益Campaign时不计算该标签。
- `有进行中仓位`：存在未完成Campaign。

AI可以结合这些事实形成自然语言结论，但不得把标签描述为预测或交易建议。

## 后端设计

### 新模型

新增 `src-tauri/src/models/option_review.rs`：

```rust
struct OptionReviewReport {
    account_id: String,
    currency: String,
    period_days: Option<i64>,
    generated_at: String,
    summary: OptionReviewSummary,
    underlyings: Vec<OptionUnderlyingReview>,
    data_quality: OptionReviewDataQuality,
}

struct OptionReviewSummary {
    completed_campaigns: usize,
    active_campaigns: usize,
    gross_premium: f64,
    net_premium_pnl: f64,
    completed_gross_premium: f64,
    completed_net_premium_pnl: f64,
    retention_rate: Option<f64>,
    annualized_yield_on_notional: Option<f64>,
    worst_campaign: Option<OptionWorstCampaign>,
}

struct OptionWorstCampaign {
    campaign_id: String,
    underlying: String,
    started_at: String,
    ended_at: String,
    strategy_path: Vec<String>,
    net_premium_pnl: f64,
}

struct OptionUnderlyingReview {
    underlying: String,
    completed_campaigns: usize,
    active_campaigns: usize,
    gross_premium: f64,
    net_premium_pnl: f64,
    completed_gross_premium: f64,
    completed_net_premium_pnl: f64,
    retention_rate: Option<f64>,
    annualized_yield_on_notional: Option<f64>,
    worst_campaign_pnl: Option<f64>,
    flags: Vec<String>,
    campaigns: Vec<OptionCampaign>,
}

struct OptionCampaign {
    id: String,
    underlying: String,
    started_at: String,
    ended_at: Option<String>,
    status: String,
    inferred: bool,
    strategy_path: Vec<String>,
    gross_premium: f64,
    close_cost: f64,
    fees: f64,
    net_premium_pnl: Option<f64>,
    secured_notional: f64,
    capital_days: f64,
    retention_rate: Option<f64>,
    annualized_yield_on_notional: Option<f64>,
}

struct OptionReviewDataQuality {
    excluded_open_campaigns: usize,
    unmatched_records: usize,
    missing_trade_dates: usize,
    notes: Vec<String>,
}
```

字段会根据Rust实现需要拆出内部 `OptionCycle`，但不暴露给前端。

### 新服务与命令

新增：

- `src-tauri/src/services/option_review_service.rs`
- `src-tauri/src/commands/option_review.rs`

Tauri命令：

```text
get_option_review(accountId, periodDays?) -> OptionReviewReport
```

服务负责读取原始记录、配对、归组和聚合；命令只做参数边界处理。`periodDays` 为空表示全部历史，传入时限制在1到3650天。

模型、服务和命令分别注册到现有 `mod.rs` 与 `lib.rs`。

第一版不新增数据库表或迁移，Campaign和指标均在请求时由现有交易记录推导。

### AI工具

新增AI工具：

```text
get_option_review(accountId, symbol?, periodDays?, allHistory?)
```

它复用 `option_review_service`，不在AI层重复计算。`accountId` 必须使用账户ID而不是账户名称；最近周期用 `periodDays`（默认365，限制1到3650），全部历史显式传 `allHistory=true`，并覆盖 `periodDays`。指定 `symbol` 时只返回对应个股及账户摘要；找不到时返回明确错误。

更新 `options-review` Skill：

- 优先调用 `get_option_review`，不再用 `get_option_positions` 推测历史绩效。
- 输出「做得好的」「做得不好的」「值得改进的」。
- 明确样本量、Campaign边界和数据质量限制。
- 明确累计现金净权利金包含进行中Campaign，但不把它当作已实现利润或未平仓市值盈亏。

保留 `get_option_positions`，继续服务当前持仓与到期风险问题。

## 前端设计

### 文件边界

- `src/pages/Review/index.tsx`：页面标题和页签容器。
- `src/pages/Review/StockReviewTab.tsx`：从现有页面提取的股票复盘内容。
- `src/pages/Review/OptionReviewTab.tsx`：期权复盘主界面。
- `src/stores/optionReviewStore.ts`：加载报告、错误与加载状态。
- `src/types/index.ts`：增加与Rust返回一致的类型。

### 状态与交互

- 账户列表复用 `accountStore`。
- 显示全部证券账户；没有期权记录的账户由页面空状态解释。
- 账户选择保存在 `localStorage` 的独立键中。
- 周期默认365天，可选全部历史。
- 报告加载后默认选中净权利金绝对值最大的个股。
- 点击个股行切换下方Campaign列表，不重新请求后端。
- 切换账户或周期才重新加载报告。

### 页面内容

四张指标卡：

1. 累计净权利金（含进行中）。
2. 权利金留存率。
3. 年化净权利金收益率，并标注「担保名义资本口径」。
4. 最差已完成Campaign。

个股表字段：

- 标的。
- 已完成/进行中Campaign数。
- 累计净权利金（含进行中）。
- 留存率。
- 年化收益率。
- 最差Campaign。
- 事实标签。

Campaign表字段：

- 开始与结束日期。
- 策略路径。
- 状态。
- 毛权利金。
- 买回成本。
- 费用。
- 净权利金（含进行中现金流）。
- 留存率。
- 年化收益率。

页面顶部或表格上方显示数据质量Alert：每条SELL开仓对应一个系统推导Campaign；进行中Campaign计入累计现金净权利金但不计入已完成绩效；同时说明多少记录因缺日期或无法匹配被排除。

### AI入口

点击「AI复盘这只股票」：

1. 调用 `setActiveSkillsForNextTurn(["options-review"])`。
2. 跳转 `/ai-assistant`，通过路由state预填：

```text
请复盘账户 {accountName}（accountId: {accountId}）在 {period} 的 {symbol} 期权交易。请调用 get_option_review，最近周期的工具参数为 {"accountId":"{accountId}","symbol":"{symbol}","periodDays":365,"allHistory":false}，全部历史则为 {"accountId":"{accountId}","symbol":"{symbol}","allHistory":true}。分别说明做得好的、做得不好的和最值得改进的地方；请使用确定性期权复盘数据并说明样本限制。
```

3. AI助手只预填、不自动发送，用户可检查后发送。
4. AI助手消费一次预填state后清除，避免刷新或返回页面时重复注入。

## 错误与空状态

- 没有账户：提示先创建账户。
- 账户没有期权记录：显示空状态并引导去期权管理导入CSV。
- 只有进行中Campaign：展示进行中列表及累计现金净权利金；留存率、年化收益率和最差Campaign显示 `—`，说明尚无已完成Campaign。
- 没有有效交易日期：不进行时间归组，报告数据质量问题。
- 无法匹配开平仓：排除对应数量，不用0填补。
- 除数为0：比率字段返回空值。
- 不同币种不跨账户合并；第一版一次只分析一个账户。

## 测试

### Rust单元测试

- 卖出开仓后到期归零，净权利金等于开仓金额减费用。
- 买回成本高于开仓权利金时，净损益和留存率为负。
- 多笔开仓、部分平仓按FIFO和数量比例正确分摊。
- 开平仓双方的佣金与费用均计入。
- 每条SELL开仓记录分别形成Campaign，时间重叠或7天内连续开仓也不合并。
- Put指派后的Call开仓形成新的Campaign，不推测合并为Wheel路径。
- 同一SELL开仓记录的部分平仓和剩余敞口保留在同一Campaign。
- 进行中Campaign计入累计现金净权利金，但不进入留存率、年化收益率和最差Campaign。
- BUY结束金额为负数时按绝对值计入买回成本。
- Excel序列日期可参与开平仓配对和周期筛选。
- 周期筛选按结束日期工作。
- 缺失日期和无法匹配记录进入数据质量统计。
- 股份拆分后的结束记录沿用拆分匹配。
- 个股和账户级聚合与底层Campaign求和一致。
- 事实标签边界正确。

### AI工具测试

- 工具定义schema有效且工具名唯一。
- 工具数量断言更新。
- symbol过滤和找不到symbol的错误返回正确。

### 前端验证

- TypeScript与Vite构建通过。
- 股票复盘原有功能在页签迁移后保持不变。
- 切换账户、周期和个股时数据显示正确。
- 手机宽度下指标卡、表格和AI按钮无重叠。
- AI按钮能预激活Skill并只预填一次问题。

## 后续版本

第一版稳定后再增加：

1. Campaign手工合并、拆分和确认，在用户确认后构建跨开仓策略链。
2. 把指派股票交易和后续CC与跨Campaign策略链显式关联。
3. 获取历史股票价格，计算策略总回报与同期持股基准。
4. 获取历史期权估值，计算进行中盈亏和最大回撤。
5. 增加复盘总览、跨账户对比和趋势图。
