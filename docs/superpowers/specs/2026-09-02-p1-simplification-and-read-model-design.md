# P1 精简、前端减包与持仓读模型设计

## 背景

旧一轮代码审计中的 P1（质量门禁、版本化数据库迁移、绩效报告聚合）已经完成。新一轮审计确认仍有三组 P1：一批无运行时消费者的旧接口与文件、两个显著偏大的前端依赖块，以及仪表盘和统计页对同一持仓上下文的重复加载与转换。

本设计只处理这三组新 P1，不改变投资指标、交易语义、行情刷新策略或数据库结构。

## 目标

1. 删除已确认无消费者的运行时和维护表面，同时保留仍被内部服务使用的能力。
2. 将 ECharts 和 Markdown 编辑器从季度详情的大型同步依赖中拆出，按实际能力和用户操作加载。
3. 建立 command 层之外的持仓读模型，让仪表盘一次请求只加载一次上下文，统计页一次只计算当前活动视图。
4. 使用行为测试保护接口收敛、统计等价性和请求次数。

## 非目标

- 不删除数据库中的 `decision_quality` 字段或历史数据。
- 不删除股票操作复盘、期权复盘或 AI 工具使用的绩效服务函数。
- 不引入数据库连接池、跨请求持仓缓存或新的状态同步框架。
- 不改变报价启动同步和用户手动刷新的既有策略。
- 不重做统计页面视觉布局或图表配置。
- 不处理本轮审计中的 P2/P3 项目。

## 方案选择

### 采用：三个独立、可验证的提交

按“死代码删除 → 前端减包 → 读模型聚合”推进。每个提交都能独立通过完整质量门禁，出现问题时可以单独回退。

### 不采用：一次返回全部仪表盘和统计数据

一个巨型报告接口虽然能消除重复调用，但会在用户只查看一个统计页签时计算全部隐藏页签，扩大序列化契约，也会重新制造不必要工作。

### 不采用：跨请求缓存持仓读模型

全局缓存需要处理交易、持仓、类别、账户、行情和汇率等多种失效来源。当前问题可以通过单请求内复用解决，无需承担陈旧数据风险。

## 设计一：删除无消费者代码

### 旧复盘路径

删除以下仅互相引用、没有页面消费者的旧路径：

- `src/stores/reviewStore.ts`
- `src-tauri/src/services/review_service.rs`
- `src-tauri/src/models/review.rs`
- `get_holding_review`
- `update_decision_quality`
- `get_decision_statistics`
- `get_reviewed_symbols`

`commands/review.rs` 继续保留正在使用的 `get_stock_operation_review`。数据库表和季度持仓上的 `decision_quality` 字段保持不变，以保留历史数据和向后兼容性。

### 旧绩效 command

删除前端不再调用的六个 Tauri command 包装器及注册：

- `get_performance_summary`
- `get_return_attribution`
- `get_monthly_returns`
- `get_holding_performance_ranking`
- `get_risk_metrics`
- `get_drawdown_analysis`

保留 `get_performance_report` 与 `get_benchmark_return_series`。同名 `performance_service` 函数仍被 AI 上下文、AI 工具、快照服务和 Rust 测试使用，不做删除。

### 零散前端和命令表面

- 从 `skillStore` 删除未使用的 `getSkill`，并移除公开的 `get_skill` Tauri command；`skill_service::get_skill` 继续供克隆和导出内部调用。
- 从 `exchangeRateStore` 删除未使用且会在错误时返回原金额的 `convertAmount`，并移除 `convert_amount` Tauri command；前端保留同步的 `convertWithCachedRates`。
- 删除未使用的 `NotesTimeline.tsx`、`theme-tokens.ts` 和 TypeScript `OptionRecord`。
- 从 `package.json` 删除未配置在 PostCSS 插件中的 `autoprefixer`，通过 Bun 正常更新锁文件，不顺带升级其他依赖。

这些删除不新增“源代码必须不存在”式单元测试；TypeScript 编译、Rust 编译、Clippy、依赖锁定安装和全仓引用检查共同验证边界。

## 设计二：ECharts Core 与按需 Markdown 编辑器

### ECharts Core

新增一个共享图表包装器，使用 `echarts/core` 与 `echarts-for-react/lib/core`，只注册项目实际使用的能力：

- 图表：`LineChart`、`BarChart`、`PieChart`
- 组件：`TitleComponent`、`TooltipComponent`、`LegendComponent`、`GridComponent`、`DataZoomComponent`、`MarkAreaComponent`
- 特性：`LabelLayout`（保留饼图 `avoidLabelOverlap`）
- 渲染器：`CanvasRenderer`

现有图表组件只把 `option`、尺寸和 renderer 参数传给共享包装器。业务数据转换、颜色、tooltip、缩放和图例配置保持原样。

### Markdown 预览和编辑

新增轻量 `MarkdownPreview`，复用仓库已有的 `react-markdown` 与 `remark-gfm` 渲染只读笔记。

编辑器代码拆成独立异步组件：

- 季度总结的自定义缩进工具栏与 `@uiw/react-md-editor` 一起留在异步 chunk。
- 持仓笔记编辑器同样仅在弹窗已打开且处于编辑模式时渲染异步组件。
- `Suspense` 使用小型加载状态，不阻塞季度详情的其余内容。

因此进入季度详情和浏览历史笔记不再下载完整编辑器；只有点击编辑时才加载它。

### 构建验收

当前生产构建基线为：

- 完整 ECharts chunk：1,141.65 kB，gzip 386.65 kB。
- `SnapshotDetail`：931.47 kB，gzip 325.03 kB。

验收时重新运行生产构建。构建结果必须不再包含上述完整 ECharts 大包，Markdown 编辑器必须成为独立异步 chunk，`SnapshotDetail` 必须显著缩小。不能通过调高 `chunkSizeWarningLimit` 隐藏问题。

## 设计三：单次持仓读模型

### 服务边界

新增 `portfolio_read_service`，将当前位于 `commands/dashboard.rs` 的持仓、账户、类别和行情拼装逻辑移入服务层。AI 上下文、AI 工具、市场概览、仪表盘和统计 command 都依赖该服务，不再由 service 反向依赖 command。

服务提供单请求生命周期的 `PortfolioReadModel`。它包含：

- 已过滤的有效持仓行；
- 账户名、类别名和类别颜色；
- 从缓存或指定报价策略取得的价格与涨跌；
- 由调用者按需加载的真实汇率。

服务区分两种报价策略：

- `RefreshMissing`：仪表盘可在缓存缺失时使用既有 provider 流程补齐报价。
- `CacheOnly`：统计、AI 和市场概览只读内存缓存，绝不触发外部行情请求。

不把模型保存在全局状态中；每个用户请求创建一次，完成后释放。

### 仪表盘报告

新增序列化契约：

```text
DashboardReport {
  summary: DashboardSummary,
  holdings: Vec<HoldingDetail>
}
```

`get_dashboard_report(base_currency)` 在同一个请求中：

1. 加载一次汇率；
2. 构造一次 `PortfolioReadModel`；
3. 从该模型计算汇总和已按 USD 归一化的持仓详情；
4. 返回一个报告。

前端 `dashboardStore` 用一次 invoke 原子更新汇总和持仓。Dashboard 页面直接从 `summary.exchange_rates` 展示汇率，不再额外调用 `get_exchange_rates`。手动刷新仍先执行现有行情刷新，再重新取得报告。

旧的 `get_dashboard_summary` 和 `get_holdings_with_quotes` 在前端切换完成后删除，避免保留两套公开入口。

### 统计聚合

统计计算拆为接收 `&PortfolioReadModel` 的纯聚合函数：整体、市场、账户和类别。现有四个统计 command 可以保持名称和返回类型，但每次 command 只加载一次模型。

`StatisticsOverview` 增加 `holdings: Vec<HoldingDetail>`，整体统计表格直接使用后端同一次加载得到的持仓，不再从 `quoteStore`、账户和类别数据重新拼装另一份持仓结果。

统计页面由父页面统一驱动加载：

- 首次只请求整体统计。
- 切换页签后才请求当前市场、账户或类别。
- 选择项变化时只刷新当前可见页签。
- 更换基准货币只重新计算依赖跨币种换算的整体或类别页签。
- 手动刷新行情后只重新请求当前活动页签。

市场和账户页继续使用原生市场货币；整体和类别页继续使用所选基准货币。所有现有聚合、排序和盈亏公式保持不变。

## 错误处理

- 读模型中的数据库、行解码、行情和汇率错误继续向 command 返回，不转换成零值或空报告。
- 仪表盘报告失败时保留前一次成功数据并展示错误；不得只更新 summary 或 holdings 的一半。
- 统计页每个视图独立记录加载和错误状态；一个视图失败不清空其他已成功视图。
- Markdown 异步 chunk 加载失败交给现有页面错误边界；保存错误保持当前显式提示。

## 测试策略

### 删除表面

- 运行 TypeScript 构建和 Rust 全目标 Clippy，确保不存在悬空导入或注册。
- 使用全仓引用扫描确认仅删除公开包装器，内部服务消费者仍存在。
- 使用 `bun install --frozen-lockfile` 验证依赖锁文件可复现。

### 前端减包

- TypeScript 编译保护所有图表包装器的 props 与 ECharts 类型。
- 生产构建验证页面 chunk、编辑器异步 chunk 和 ECharts Core 产物。
- 手动冒烟检查 Dashboard、Statistics、Performance、Quarterly Comparison、Quarterly Trends 和 Snapshot Detail 的图表渲染，以及季度/持仓笔记的预览、编辑、保存和取消。

### 读模型

- Rust fixture 测试比较重构前后仪表盘汇总、整体统计、市场、账户和类别统计的全部字段。
- 测试 `CacheOnly` 不会调用 provider，缺失报价沿用现有零价格表现；`RefreshMissing` 继续使用既有 provider 流程。
- 测试一个 `DashboardReport` 由同一个模型同时生成 summary 和 holdings。
- 为 Zustand store 提供可注入的 invoke，验证仪表盘一次刷新只调用一次 `get_dashboard_report`。
- 验证统计页首次只请求整体视图、切换后只请求目标视图、手动刷新只请求当前活动视图。

## 提交边界

1. `refactor: remove unused runtime surface`
2. `perf: load chart and markdown dependencies on demand`
3. `perf: reuse portfolio read models`

每个提交都运行相关定向测试；第三个提交完成后运行 `bun run check`、`git diff --check` 和生产构建体积对比。

## 成功标准

- 已确认的旧复盘、旧绩效 command 和零散死代码没有运行时注册或前端消费者。
- 仍被 AI、快照、技能克隆/导出使用的内部服务能力保持可用。
- 完整 ECharts 入口不再进入构建，季度详情不再同步携带 Markdown 编辑器。
- 仪表盘一次报告请求只构造一次持仓上下文。
- 统计页首次和刷新时只计算当前需要的视图，每个 command 只构造一次持仓上下文。
- 所有投资公式、货币口径、报价策略和数据库结构保持不变。
- 完整前端测试、生产构建、Rust 格式检查、Rust 测试和严格 Clippy 全部通过。
