# P3 定向精简设计

## 背景

新一轮审计把 P3 定义为三类低风险、需先验证收益的维护工作：在已经修改过的后端热点中拆出稳定职责，按实际构建结果缩小 ECharts 首屏依赖，以及移除绩效筛选刷新中的零延迟定时器。

当前基线为：

- `commands/options.rs` 共 1,834 行，其中生产代码约 1,167 行，仍同时承担 CSV、合约投影、模拟和导出。
- `services/performance_service.rs` 共 2,854 行，其中生产代码约 1,785 行，仍同时承担数据加载、收益计算、归因、持仓排名和基准行情。
- `commands/transactions.rs` 的生产代码已经因 P0 重放改造缩至约 256 行，不再具备值得拆分的独立职责。
- 生产构建中的共享 `EChart` chunk 为 635.73 kB，gzip 后 218.35 kB；Dashboard 只使用饼图，却会加载柱图、折线图、缩放和标记区域注册。
- 绩效页面的市场和账户筛选仍通过 `setTimeout(fetchAll, 0)` 等待 Zustand 状态变化，但 Zustand 的 `set` 本身是同步的。

## 目标

1. 让期权命令和绩效服务的文件边界反映现有职责，同时完全保留外部接口与业务口径。
2. 让 Dashboard 的饼图路径不加载柱图、折线图专用实现，并以构建产物证明收益。
3. 让绩效筛选在 store 内完成“更新状态后刷新”，删除页面层时序补丁。

## 非目标

- 不修改 Tauri 命令名、参数、返回类型或数据库结构。
- 不改变期权 FIFO、拆股匹配、状态投影、模拟公式或 CSV 兼容规则。
- 不改变绩效 TWR、回撤、归因、风险、排名和基准收益口径。
- 不拆分已经足够聚焦的 `transactions.rs`。
- 不通过提高 Vite chunk 告警阈值隐藏体积问题。

## 后端模块边界

### 期权命令

保留 `commands/options.rs` 作为 Tauri 公共入口和兼容门面，并在 `commands/options/` 下建立：

- `csv.rs`：CSV 字段解析、预览、强类型导入与导出。
- `contracts.rs`：匹配输入加载、状态重算和合约只读投影。
- `simulation.rs`：卖出 Put 与卖出 Call 模拟。
- `tests.rs`：原有命令级特征测试，继续通过父模块访问私有实现。

`commands::options::*` 的现有公开函数与 `StockPriceInput`、`ImportOptionsResult` 继续从门面导出；AI 工具使用的 `get_option_contracts_inner` 路径保持不变。

### 绩效服务

保留 `services/performance_service.rs` 作为 `PerformanceFilter`、聚合报告和公共服务门面，并在 `services/performance_service/` 下建立：

- `calculation.rs`：估值/现金流加载、TWR、回撤、波动率、夏普率、汇总和月度收益。
- `attribution.rs`：市场、分类和持仓归因。
- `ranking.rs`：持仓绩效排名。
- `benchmark.rs`：基准行情缓存、读取、网络刷新和收益序列转换。
- `tests.rs`：现有服务特征测试。

内部模块只使用 `pub(super)` 暴露编排所需能力；仓库其他调用方仍通过 `performance_service` 门面访问原函数。

## 前端图表边界

保留一个只封装 `echarts/core`、Canvas 渲染器和 React 适配器的核心组件，并建立两个注册入口：

- 饼图入口注册 `PieChart` 以及饼图实际使用的标题、提示和图例组件。
- 笛卡尔入口注册 `BarChart`、`LineChart`、网格、缩放、标记区域及实际使用的公共组件。

`PieChart.tsx` 使用饼图入口；`BarChart.tsx`、绩效图表和季度趋势/对比图使用笛卡尔入口。注册仍作用于同一个 ECharts core 实例，因此晚加载其他路由后不会产生重复实例或行为差异。

验收时重新构建并比较 chunk：Dashboard 依赖闭包不得包含柱图、折线图专用注册，饼图首载体积必须有明确下降。若构建器仍把全部实现合并成同一首载 chunk，则撤销该拆分，不保留只增加文件数量的抽象。

## 绩效筛选刷新

`performanceStore` 的 `setMarket` 与 `setAccountId` 改为返回刷新 Promise：先同步写入互斥筛选条件，再立即调用同一 store 的 `fetchAll`。页面只触发这两个动作，不再自行调度定时器。

回归测试直接等待筛选动作，并断言 `get_performance_report` 收到刚写入的市场或账户参数，同时保持既有“最新请求获胜”语义。

## 测试与提交

分三批提交：

1. 期权与绩效后端职责拆分。先运行模块边界红灯测试，再迁移实现；验证对应 Rust 测试、格式和严格 Clippy。
2. ECharts 注册拆分。先添加注册边界测试，再修改消费者；验证前端测试、TypeScript 构建和前后 chunk 对比。
3. 绩效筛选刷新。先添加立即刷新红灯测试，再删除定时器；验证 store 测试和生产构建。

全部完成后运行 `bun run check` 与 `git diff --check`。最终结果不得改变任何公开后端接口、金融计算口径或页面路由。
