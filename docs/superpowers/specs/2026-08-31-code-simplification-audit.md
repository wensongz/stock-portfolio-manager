# 代码精简与优化审计

## 审计目标

在不改变现有产品口径、持仓/交易语义、行情刷新策略和本地优先架构的前提下，找出最值得先处理的数据安全问题、重复计算、重复实现和维护热点，并给出按风险与依赖排序的治理路线。

## 当前基线

- 前端：React 19、TypeScript 7、Vite 8、Ant Design 6、Zustand 5。
- 后端：Tauri 2、Rust 1.97.1、rusqlite 0.40、SQLite。
- 代码规模：前端非测试代码约 28,103 行；Rust 代码约 37,739 行。
- 自动验证：`node --test` 通过 75 个测试；`cargo test --lib` 通过 400 个测试、忽略 8 个网络集成测试；`bun run build` 通过；`cargo fmt --all -- --check` 通过。
- 严格静态检查：`cargo clippy --all-targets --all-features -- -D warnings` 失败，当前共有 10 个诊断。
- 前端构建：当前只有一个主 JavaScript 产物，压缩前 4,182.24 kB，gzip 后 1,362.16 kB；Vite 明确提示应进行代码分割。
- 工作树在审计开始和结束时均无已跟踪改动；构建产物位于已忽略的 `dist/`。

## 关键发现

### P0：通用 CSV 导入只会提交前 20 条有效记录

`parse_import_csv` 为了预览只把前 20 条有效记录写入 `preview_data`，前端确认导入时又直接把 `preview.preview_data` 作为完整导入数据发送给后端。因此包含 21 条以上有效记录的通用持仓/交易 CSV 会静默漏掉第 21 条及之后的记录。

同时，`valid_rows` 使用“总行数减错误条目数”计算。一行出现两个字段错误时会被扣减两次，统计口径也会失真。

### P0：通用导入绕过交易与持仓的唯一业务写入路径

`confirm_import` 直接写 `holdings` 和 `transactions`：

- 持仓导入不会生成 `OPEN` 基线交易，而正常 `create_holding` 会生成；这会影响绩效、复盘和历史重放。
- 交易导入先更新持仓，再插入交易，整行没有事务或 savepoint；插入失败时可能留下已改变的持仓。
- 导入路径的 SELL 成本公式与 `create_transaction` 不一致，且没有复用现金持仓联动逻辑。
- 数字解析失败会降级成 `0.0`，没有复用单笔命令的数值和市场校验。

这不是纯粹的代码风格问题，而是同一业务存在两套会产生不同数据结果的写入实现。

### P0：数据库备份没有使用 SQLite 一致性快照

手动和自动备份都使用 `std::fs::copy` 复制正在使用的 `portfolio.db`。该方式没有通过 SQLite online backup API 获取一致性快照，也没有在完成后运行完整性检查。应用恰好处于写事务时，备份文件存在不可用或状态不一致的风险。

备份核心逻辑在手动和自动路径中重复，且没有测试。

### P0：恢复出厂设置没有完整恢复默认状态

- `ai_config` 的 UPSERT 没有写入或更新 `tools_enabled`，所以用户关闭工具调用后执行恢复出厂设置，旧值仍可能保留。
- `cached_quote_refresh_time` 没有清空，恢复后界面可能显示旧的行情刷新时间。
- 系统分类定义在数据库初始化和恢复逻辑中各维护一份，存在漂移风险。
- 备份配置文件先于数据库事务写入；后续数据库步骤失败时，注释中承诺的“不会留下混合状态”并不成立。

### P1：数据库迁移吞掉真实错误

`db::run_migrations` 在每次启动时重复执行多条 `ALTER TABLE`，并用 `let _ = ...` 忽略所有错误。它无法区分“列已存在”与磁盘、权限、SQL 或数据库损坏错误，也没有 schema 版本或迁移历史。

`get_ai_config` 和 `get_quote_provider_config` 同样把所有查询错误降级成默认值。这会把迁移失败伪装成“用户尚未配置”。

### P1：绩效页重复加载同一计算上下文

前端一次 `fetchAll` 并行调用 6 个 Tauri command。后端的 summary、drawdown、attribution、monthly returns、ranking、risk metrics 都各自调用 `PerformanceCalculation::load`。由于数据库只有一个 `Mutex<Connection>`，这些请求会争用同一连接，并重复读取每日估值、基线和现金流。

这是静态调用链得出的确定事实；实际耗时收益需要在有代表性的数据集上做改造前后测量。

### P1：质量门禁和实际约束不一致

- README 要求 `node --test`、前端构建和 Rust 测试，但 `package.json` 没有 `test` 或统一 `check` 脚本。
- 当前 GitHub Actions 只在发版时构建，没有 pull request 检查。
- CI 使用 `bun install --frozen-lockfile`，但 `.gitignore` 忽略 `bun.lock`，仓库当前没有跟踪任何前端 lockfile。
- 严格 Clippy 当前失败 10 处，说明之前建立的“严格检查通过”基线已经回退。

### P2：导入前端存在高密度重复

9 个持仓/交易导入弹窗合计 6,137 行，其中 8 个 CSV 弹窗高度重复；图片 OCR 弹窗只适合复用结果模型和名称补全，不应被强行塞进 CSV 解析抽象。重复部分包括：

- CSV 拆行、数字转换、代码格式化等辅助函数；
- 文件选择、步骤状态、行选择、行编辑、结果统计和关闭重置；
- 持仓名称补全；
- 按时间排序并逐行调用 Tauri command 的导入循环。

应保留券商解析器的差异，只抽取工作流、通用行模型和展示骨架。把所有券商塞进一个巨型解析函数会降低清晰度。

### P2：首屏加载没有路由级代码分割

`App.tsx` 静态导入所有页面，因此 Dashboard 启动也会加载 AI 助手、期权、绩效、季度分析和全部导入弹窗依赖。当前构建结果是单个 4.18 MB JavaScript chunk。

路由级 `React.lazy` 是低风险、可独立验证的优化；无需先重写页面内部结构。

### P2：行情持仓查询存在 N+1

`get_holding_quotes` 对每个已清仓持仓单独执行一次 `compute_realized_pnl` 查询。可以用一次按 `holding_id` 聚合的 SQL 替代，保留相同结果口径。

### P3：热点文件职责过多

最大的生产代码热点包括：

- `quote_service.rs`：2,543 行生产代码，同时包含缓存、持久化、Yahoo、东方财富、雪球、历史行情、K 线和财报。
- `quarterly_service.rs`：2,420 行生产代码，包含快照、刷新、比较、笔记、趋势和交易回顾。
- `ai_chat_service.rs`：2,167 行生产代码，包含上下文、OpenAI 流式协议、Anthropic 协议、工具循环和标题生成。
- `ocr.rs`：1,525 行生产代码，包含证券搜索、图片切分、预处理、Tesseract 调用和文本解析。
- `AiAssistant/index.tsx`：1,803 行，已有明确的 SessionSidebar、ChatPanel、Composer、MessageRow 等组件边界，但仍放在同一文件。

这些文件值得拆分，但应排在数据一致性、质量门禁和已确认性能重复之后。单纯按行数拆文件不会自动改善架构。

### P3：可删除的小范围表面积

- `take_snapshot` 和 `get_portfolio_history` 已注册为 Tauri command，但前端没有调用。
- `get_return_series` command 已被 summary 中的 `return_series` 取代。
- `@tauri-apps/plugin-shell`、Rust shell plugin 初始化和 `shell:allow-open` 权限没有代码消费者。

这些项目适合在质量门禁恢复后删除。

## 推荐策略

采用“数据安全优先、计算聚合其次、界面去重最后”的渐进方案：

1. 先恢复可重复构建与严格检查。
2. 修复通用导入截断和双写语义，建立唯一持仓/交易写入路径。
3. 修复备份与恢复出厂设置的数据完整性。
4. 建立显式、可失败的 schema 迁移机制。
5. 聚合绩效请求并消除已确认的 N+1。
6. 做路由级代码分割。
7. 在测试保护下合并导入工作流。
8. 最后按职责拆分行情、AI、季度和 OCR 热点。

## 明确不建议

- 不为 40–60 行的 CRUD Zustand store 建立通用基类；节省的代码不足以抵消间接层。
- 不在本轮引入数据库连接池；单机 SQLite 的瓶颈应先通过减少重复查询验证。
- 不把依赖大版本升级与重构混在同一批提交中。
- 不一次性生成或替换全部 Rust/TypeScript 类型；先在新聚合接口和导入接口上建立清晰契约。
- 不改变现有 symbol 聚合不变量、行情刷新策略或投资指标口径。

## 成功标准

- 任意长度的通用 CSV 都只导入有效行，不再受 20 行预览限制。
- 所有持仓/交易写入路径复用同一业务函数，导入失败不会留下半写状态。
- 备份可在应用运行中生成，并能通过 `PRAGMA integrity_check` 和关键表读取。
- 迁移错误可见、可测试，重复启动保持幂等。
- 绩效页一次刷新只加载一次 `PerformanceCalculation`。
- pull request 自动运行前端测试/构建、Rust fmt/test/strict Clippy。
- 路由构建产物出现独立页面 chunk，首屏不再依赖全部页面代码。
- 重构前后的 75 个前端测试和 400 个 Rust 测试继续通过，并补齐导入、备份、迁移与恢复测试。
