# AI 助手工具（Tools）

除了「技能（Skills）」之外，AI 助手还支持「工具（Tools）」机制——这是两个互补的扩展点：

| | 技能（Skills） | 工具（Tools） |
| --- | --- | --- |
| **本质** | 给 AI 的**自然语言指令模板**（注入到 system prompt） | AI 可**回调执行的函数**（拿到真实数据） |
| **形式** | Markdown 文件（`skills/*.md`） | 后端 Rust 函数（注册在 `ai_tools.rs`） |
| **作用** | 约束 AI 的回答结构、步骤、格式 | 让 AI 获取应用数据、缓存数据或按需查询的外部数据 |
| **可定制** | ✅ 用户可新建/编辑/导入导出 | ❌ 由应用内置（需改代码） |
| **何时用** | "按这个格式给我做风险体检" | "今天大盘怎么样" / "AAPL 现价多少" |

一句话区分：**技能告诉 AI「怎么说」，工具让 AI「能查到」**。

## 工作原理

工具通过模型提供商的函数调用能力实现。DeepSeek、Gemini、GLM、Grok、Kimi、MiMo、Ollama、OpenAI、OpenRouter 和 Qwen 使用 OpenAI 兼容协议；Anthropic Claude 使用原生 Messages 协议，应用会把同一份工具定义转换为 Anthropic 的 `tool_use` / `tool_result` 格式。

1. 每次对话，后端把可用工具的清单（名字、描述、参数 schema）随请求发给大模型。
2. 当模型判断需要数据时，它返回工具调用（OpenAI 兼容协议中的 `tool_calls`，或 Anthropic 协议中的 `tool_use`）。
3. 后端执行对应的 Rust 函数，把结果按相应协议回传给模型。
4. 模型拿到真实数据后，继续生成最终回答（流式输出给用户）。

界面会在回答中展示每次工具调用的结果卡片，包括参数、状态、结果或错误和耗时。卡片会随消息写入本地会话，重新打开会话后仍可回看。

应用支持 11 个提供商：Anthropic Claude、DeepSeek、Gemini、GLM、Grok、Kimi、MiMo、Ollama、OpenAI、OpenRouter 和 Qwen。是否能使用工具取决于所选的具体模型；同一提供商内也可能有不支持函数调用的模型。需要工具查询时，请选择支持函数调用的模型。

Qwen 默认使用阿里云百炼北京端点；其他地域或工作空间请在 AI 配置中填写对应 API 端点。Gemini 使用 Google AI Studio 的 API Key，Grok 使用 xAI 的 API Key。接口说明：[Qwen](https://www.alibabacloud.com/help/en/model-studio/compatibility-of-openai-with-dashscope)、[Gemini](https://ai.google.dev/gemini-api/docs/openai)、[Grok](https://docs.x.ai/overview)。

## 内置工具清单

注册表共 23 个工具，按用途分组。启用工具的普通会话会向模型提供其中 22 个；`get_rebalance_context` 只在应用从组合提醒入口创建受信任的再平衡会话时开放。

### 行情类

| 工具名 | 参数 | 作用 |
| --- | --- | --- |
| `get_market_overview` | 无 | 今日主要指数行情 + 用户持仓当日表现 |
| `get_stock_quote` | `symbol`，`market?` | 某只股票的实时行情 |
| `get_price_history` | `symbol`，`market?`，`days?=30` | 近 N 日收盘价序列 |
| `search_stock` | `query`，`direction?` | 名称查代码 / 代码查名称（不确定代码时先调用） |

### 基本面与技术分析

| 工具名 | 参数 | 作用 |
| --- | --- | --- |
| `get_stock_fundamentals` | `symbol`，`market?` | 估值与基本面指标（PE、PB、市值、股息率、EPS、ROE、换手率） |
| `get_technical_indicators` | `symbol`，`market?`，`days?=120` | 均线、MACD、RSI 与布林带技术指标 |
| `get_financial_statements` | `symbol`，`market?`，`periods?=4` | A股近几期营收、净利润、EPS、ROE、资产负债率及同比增速 |

### 组合类

| 工具名 | 参数 | 作用 |
| --- | --- | --- |
| `get_portfolio_overview` | 无 | 组合结构化总览（与自动注入的快照相同） |
| `get_holdings_detail` | 无 | 持仓逐只明细 |
| `get_dashboard_summary` | 无 | 总资产/盈亏/按市场（美股·港股·A股）拆分 |
| `get_transactions` | `txType?`，`symbol?`，`days?`，`limit?=50` | 交易记录，可按类型/日期/标的过滤（PAY 为分红） |

### 绩效类

| 工具名 | 参数 | 作用 |
| --- | --- | --- |
| `get_performance_metrics` | `periodDays?=365` | 收益/回撤/波动率/夏普等综合绩效指标 |
| `get_return_attribution` | `periodDays?=365` | 收益归因到市场/类别/个股 |
| `get_monthly_returns` | `periodDays?=365` | 月度收益序列（每月收益率、盈亏） |
| `get_drawdown_analysis` | `periodDays?=365` | 最大回撤详情（峰值/谷底/恢复日期、持续天数） |
| `get_risk_metrics` | `periodDays?=365` | 波动率/夏普/Calmar/最大回撤 |
| `get_holding_ranking` | `sortBy?=pnl`，`limit?=10`，`periodDays?=365` | 个股绩效排名 |

### 期权

| 工具名 | 参数 | 作用 |
| --- | --- | --- |
| `get_option_positions` | `accountId` | 当前持仓和到期风险 |
| `get_option_review` | `accountId`，`symbol?`，`periodDays?=365`，`allHistory?=false` | 每笔SELL开仓对应一个Campaign；`gross_premium`/`net_premium_pnl`为含进行中Campaign的累计现金口径，`completed_gross_premium`/`completed_net_premium_pnl`及留存率、年化收益率和最差Campaign为已完成口径；`allHistory=true` 返回全部历史并覆盖 `periodDays` |

### 股票操作复盘

| 工具名 | 参数 | 性质与作用 |
| --- | --- | --- |
| `get_stock_review` | `start_date`，`end_date`，`base_currency`，`account_id?`，`market?`，`symbol?` | 只读。调用与股票操作复盘页面相同的 `stock_operation_review_service` 确定性报告；只解释返回数值和事实标签，不重算指标，也不把事后涨跌直接判定为决策对错 |

`get_stock_review` 的日期格式为 `YYYY-MM-DD`，基准币种支持 `USD` / `CNY` / `HKD`，市场支持 `US` / `CN` / `HK`。可选 `symbol` 将范围收窄到一只股票：仅保留该股票的操作并重新汇总。`available`、`degraded`、`pending`、`unavailable` 都是成功报告中的数据状态，不能把不可用指标当成工具执行错误，也不能由 AI 补零或重算。

AI 工具注册表没有股票操作复盘的写入、纠正或注释工具。

### 组合再平衡

| 工具名 | 参数 | 性质与作用 |
| --- | --- | --- |
| `get_rebalance_context` | `config_id` | 只读。按已保存的组合提醒配置和当前缓存行情重新计算可信再平衡上下文，返回配置范围、基准币种、总市值、目标与当前占比、偏离、持仓、活动违规和确定性调仓金额 |

这个工具不是普通会话可自由调用的通用查询。只有用户从组合提醒的违规入口进入 AI 调仓建议时，应用才会预填并授权唯一的 `config_id`；工具不接受模型自行补充的市场、账户、金额或目标占比。配置必须仍启用、数据必须完整且未过期、违规必须仍有效，返回前还会再次校验配置与违规状态。计算假设追加资金为 0，并且不会修改提醒状态、持仓或交易记录，也不会自动下单。

### 其他

| 工具名 | 参数 | 作用 |
| --- | --- | --- |
| `get_dividend_income` | `days?` | 分红/利息收入汇总（按标的聚合，PAY 类型；省略天数时统计全部） |
| `check_price_alerts` | 无 | 价格提醒触发情况（基于缓存行情；检查时会刷新提醒的触发状态和触发时间） |

> `periodDays` 默认 365（近 1 年），最大 3650。期权复盘如需全部历史，显式传 `allHistory=true`。`sortBy` 可选 `pnl`（盈亏金额）或 `return_rate`（收益率）。

### 主要指数

`get_market_overview` 报告以下指数：

| 代码 | 名称 | 市场 |
| --- | --- | --- |
| `^GSPC` | 标普500 | US |
| `^IXIC` | 纳斯达克 | US |
| `^DJI` | 道琼斯 | US |
| `^HSI` | 恒生指数 | HK |
| `000300.SS` | 沪深300 | CN |
| `000001.SS` | 上证综指 | CN |

每个指数包含现价、涨跌额、涨跌幅、昨收。单一指数抓取失败（如 Yahoo 对 CN 指数限流）不会影响整体——该指数显示为空，模型会如实说明。

## 与自动注入快照的关系

普通会话默认可注入一份「当前投资组合快照」（账户总览、持仓表、最近 20 条交易、近一年绩效指标），持仓行情使用**缓存数据**且**不含大盘指数**。用户可在对话顶部关闭「注入数据」；关闭后只停止自动附带这份快照，工具仍可按需查询数据。

从股票操作复盘、期权复盘、组合复盘或组合提醒等专用入口进入时，应用会提供相应的报告或上下文；其中组合范围复盘和再平衡入口还会按应用授权的市场、账户或提醒配置限制该轮可用工具。这类专用数据不等同于普通会话的全组合快照。

币种以各段和字段的标注为准：普通全组合快照的绩效金额为 `USD`，按市场/账户筛选的绩效金额则使用对应市场原币；汇率缓存可用时，组合汇总和明确标注 `USD` 的市值会换算为美元。持仓均价、现价以及交易价格和金额仍是证券原币；近期交易表未逐条展示币种，无法确认时需先核实，不能直接跨币种汇总。汇率不可用时，跨币种持仓汇总会省略，持仓市值按原币展示。

- 快照解决的是"AI 一上来就知道我的持仓长什么样"——无需额外请求。
- 工具解决的是"AI 需要实时/外部数据时能主动去查"——按需调用。

例如问"今天大盘怎么样"：快照里没有指数数据，AI 会调用 `get_market_overview` 工具去取实时指数行情。问"AAPL 现价"：若你未持仓 AAPL，快照里也没有，AI 会调用 `get_stock_quote`。

## 关键文件

| 关注点 | 文件 |
| --- | --- |
| 工具注册 + 执行分发 | `src-tauri/src/services/ai_tools.rs` |
| 大盘总览数据源 | `src-tauri/src/services/market_overview_service.rs` |
| 聊天循环（工具调用迭代） | `src-tauri/src/services/ai_chat_service.rs`（`chat_stream`） |
| 系统提示词（告知模型可用工具） | `src-tauri/src/models/ai_config.rs`（`DEFAULT_SYSTEM_PROMPT`） |
| 前端工具调用卡片与持久化 | `src/components/ai/ToolCallCard.tsx`、`src/stores/chatStore.ts`、`src/stores/chat/persistence.ts` |

## 注意事项

- 工具调用仍是单回合内的模型—工具循环；用于模型继续推理的协议消息不会作为独立聊天消息展示，但每次调用的参数、状态、结果或错误和耗时会随助手消息持久化，并可在会话中回看。
- 工具调用是一个完整的智能体循环（agentic loop）：模型可以多次调用工具、拿到结果后继续推理，直到给出最终回答。轮次上限由 `ai_tools.rs` 的 `MAX_TOOL_ROUNDS` 控制，留有充分余量以支持多步分析。
- 工具描述（告诉模型何时该用哪个工具）写在 `ai_tools.rs` 的 `tool_definitions()` 里；系统提示词里也有一段总览。改这两处可以调整模型选择工具的行为。
- 内置工具不会新增、修改或删除持仓及交易记录，也不会提交交易或下单。`check_price_alerts` 检查缓存行情时会刷新价格提醒的触发状态和触发时间；个别行情工具可能按需请求外部数据，`get_rebalance_context` 则明确只使用缓存行情和缓存汇率。
- 参数以各工具表格及运行时 schema 为准。`market` 只适用于表中明确列出的标的类工具，`accountId` / `account_id` 也只适用于相应的期权或股票复盘工具；从专用入口获得的市场、账户等范围由应用授权，不是所有工具的通用参数。
