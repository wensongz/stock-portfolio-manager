# P3 后端热点拆分规格

## 目标

在不改变 Tauri 命令名、服务层公共函数签名、数据库结构、事件名称和前端请求形状的前提下，按既有职责拆分 `ai_chat_service.rs`、`quarterly_service.rs` 与 `commands/ocr.rs`。本轮只做结构性重构和必要的特征测试，不修改业务规则。

## 不可变契约

- AI：保留 `chat_stream`、`stop_chat`、`generate_title`、`build_portfolio_context`、`ChatParams`、`ChatUsage` 及全部 `ai-chat-*` 事件协议。
- 季度报告：保留所有 `quarterly_service` 公共函数、返回模型、SQL 读写顺序及快照计算口径。
- OCR：保留 `lookup_cn_stock_code`、`lookup_stock_name_by_symbol`、`parse_trade_image` 三个命令及 `ParsedTradeRow` 序列化形状。
- 不新增网络请求，不改变错误文案，不调整数据库迁移。

## 模块边界

### AI 聊天

- `ai_chat_service.rs`：公共类型、停止状态、技能选择和 OpenAI 工具循环编排。
- `ai_chat/context.rs`：只读投资组合上下文与近期交易查询。
- `ai_chat/title.rs`：非流式会话标题生成。
- `ai_chat/anthropic.rs`：Anthropic 请求、SSE 解析和工具循环。

### 季度报告

- `quarterly_service.rs`：创建、刷新、删除和当前季度兜底编排。
- `quarterly/dates.rs`：季度字符串与日期边界。
- `quarterly/comparison.rs`：快照加载和市场、分类、持仓对比。
- `quarterly/notes.rs`：持仓与季度笔记。
- `quarterly/trends.rs`：季度趋势聚合。
- `quarterly/transactions.rs`：季度交易查询。

### OCR

- `commands/ocr.rs`：公开命令和流程编排。
- `commands/ocr/lookup.rs`：证券代码/名称远程查询。
- `commands/ocr/image_pipeline.rs`：图片切分、预处理和 Tesseract 调用。
- `commands/ocr/parser.rs`：同花顺 OCR 文本解析与字段推断。

## 验证

每个热点独立提交。提交前运行对应 Rust 单元测试与严格 Clippy；全部完成后运行 `bun run check`。新增或保留的纯逻辑特征测试必须覆盖 AI 协议整形、季度日期边界和 OCR 解析/图片切分。
