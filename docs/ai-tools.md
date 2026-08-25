# AI 助手工具（Tools）

除了「技能（Skills）」之外，AI 助手还支持「工具（Tools）」机制——这是两个互补的扩展点：

| | 技能（Skills） | 工具（Tools） |
| --- | --- | --- |
| **本质** | 给 AI 的**自然语言指令模板**（注入到 system prompt） | AI 可**回调执行的函数**（拿到真实数据） |
| **形式** | Markdown 文件（`skills/*.md`） | 后端 Rust 函数（注册在 `ai_tools.rs`） |
| **作用** | 约束 AI 的回答结构、步骤、格式 | 让 AI 获取行情、持仓、绩效等实时数据 |
| **可定制** | ✅ 用户可新建/编辑/导入导出 | ❌ 由应用内置（需改代码） |
| **何时用** | "按这个格式给我做风险体检" | "今天大盘怎么样" / "AAPL 现价多少" |

一句话区分：**技能告诉 AI「怎么说」，工具让 AI「能查到」**。

## 工作原理

工具基于 OpenAI 标准的**函数调用（Function Calling）**接口实现：

1. 每次对话，后端把可用工具的清单（名字、描述、参数 schema）随请求发给大模型。
2. 当模型判断需要数据时，它返回一个 `tool_calls`（"我想调用 get_market_overview"）。
3. 后端执行对应的 Rust 函数，把结果作为 `tool` 消息回传给模型。
4. 模型拿到真实数据后，继续生成最终回答（流式输出给用户）。

这个过程**对用户透明**——用户只看到 AI 的最终回答，以及回答上方一个 `🔍 已查询：大盘总览` 的蓝色徽标，表示这次回答用到了真实数据查询。

你接入的所有厂商（OpenAI / DeepSeek / 智谱 GLM / 月之暗面 Kimi / OpenRouter / Ollama）都原生支持这个标准接口。

## 内置工具清单

共 21 个工具，按用途分组：

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
| `get_financial_statements` | `symbol`，`periods?=4` | A股近几期营收、净利润、EPS、ROE、资产负债率及同比增速 |

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
| `get_option_review` | `accountId`，`symbol?`，`periodDays?=365`，`allHistory?=false` | 历史Campaign和确定性复盘指标；`allHistory=true` 返回全部历史并覆盖 `periodDays` |

### 其他

| 工具名 | 参数 | 作用 |
| --- | --- | --- |
| `get_dividend_income` | `days?=365` | 分红/利息收入汇总（按标的聚合，PAY 类型） |
| `check_price_alerts` | 无 | 价格提醒触发情况（基于缓存行情） |

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

每次对话默认会自动注入一份「当前投资组合快照」（账户总览、持仓表、近期交易、绩效指标），使用的是**缓存行情**且**不含大盘指数**。

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
| 前端徽标（`ai-chat-tool` 事件） | `src/stores/chatStore.ts`、`src/pages/AiAssistant/index.tsx` |

## 注意事项

- 工具调用是**单回合内部行为**，不会持久化中间过程——会话里只存用户的提问和 AI 的最终回答。（注：每轮工具的入参和返回结果会作为卡片临时展示在界面上，但不会写入数据库。）
- 工具调用是一个完整的智能体循环（agentic loop）：模型可以多次调用工具、拿到结果后继续推理，直到给出最终回答。轮次上限由 `ai_tools.rs` 的 `MAX_TOOL_ROUNDS` 控制，留有充分余量以支持多步分析。
- 工具描述（告诉模型何时该用哪个工具）写在 `ai_tools.rs` 的 `tool_definitions()` 里；系统提示词里也有一段总览。改这两处可以调整模型选择工具的行为。
- 若某厂商不支持 `tools` 字段，请求会失败并提示错误——此时可换用支持函数调用的模型。
