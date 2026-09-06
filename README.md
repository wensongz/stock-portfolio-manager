# Stock Portfolio Manager

**简体中文** | [English](README_EN.md)

一个面向个人投资者与小型投资机构的本地优先桌面投资组合管理工具，统一管理美股、A 股和港股的多账户持仓、交易、绩效与投资复盘。

支持平台：**macOS / Windows / Linux**

项目基于 **Tauri 2 + React 19 + TypeScript + Rust + SQLite**。组合数据、配置和 AI 会话默认保存在本机；行情、汇率及已配置的 AI 服务需要联网访问。当前采用单机数据模式，适合独立投资者，以及暂不需要复杂成员权限或云端协作的小型投资团队。

## ✨ 功能概览

### 账户、持仓与交易

- 美股、A 股、港股多市场、多证券账户管理
- 4 个系统投资类别（现金类、分红股、成长股、套利）及自定义类别
- 持仓新增、编辑、删除、筛选和实时盈亏展示
- 支持 `BUY` / `SELL` / `OPEN` / `PAY` 交易，自动维护持仓数量与平均成本
- 支持现金存入、提取及余额校验，可按市场设置卖出/分红是否调整持仓成本
- 持仓和交易列表会记住账户筛选、分页大小等界面偏好

### 行情、仪表盘与统计分析

- 可按市场选择雪球、东方财富或 Yahoo Finance 行情源，并提供缓存与失败回退
- 启动后后台刷新持仓行情，保留最近一次行情及刷新时间
- USD / CNY / HKD 实时汇率、持久化缓存与本位币换算
- 仪表盘展示总市值、成本、累计盈亏、当日盈亏和持仓明细
- 按市场、账户、投资类别查看资产分布、盈亏和个股明细
- 支持红涨绿跌 / 绿涨红跌以及浅色 / 深色 / 跟随系统主题

### 绩效与季度分析

- 时间加权收益率（TWR）、累计/年化收益、波动率、夏普比率和 Calmar 比率
- 自定义时间范围的收益曲线，并与 S&P 500、NASDAQ、沪深 300、上证指数、恒生指数对比
- 最大回撤、收益归因、月度收益、持仓 Top/Bottom 排名
- 自动补齐缺失的历史持仓快照，支持每日组合与持仓快照
- 季度快照创建/刷新、季度对比、多季度趋势及季度交易回顾
- 持仓投资笔记、季度总结和历史笔记时间线

### 分红、期权与操作复盘

- 按年份、市场、账户和标的汇总 `PAY` 类型的分红/利息记录
- 期权 CSV 导入/导出、持仓与已到期合约统计、Sell Put / Sell Call 模拟
- 支持中英文字段名、期权合约状态校验、拆股比例与每张合约股数配置
- 股票操作复盘：按季度快照查看历史决策并标记决策质量
- 期权操作复盘：按 Campaign 匹配开平仓，分析权利金、留存率、担保名义资本口径年化收益率、最差 Campaign 和数据质量
- 可将确定性的期权复盘结果一键交给 AI 助手继续分析

### 导入、提醒、组合再平衡与数据安全

- 通用 CSV 导入/导出，以及 Interactive Brokers、Moomoo、Firstrade、同花顺等格式的持仓/交易导入
- A 股交易截图 OCR 导入，导入前可预览与校验
- 股票持仓/交易导入支持持久化批次、成交编号及原始文件去重、疑似重复确认、失败行重试、持仓/现金对账和受保护整批撤销；详情见 [导入批次与对账](docs/import-batches.md)
- **投资提醒**工作区（`/alerts`）包含“组合提醒”和“价格提醒”两个标签页；原有价格、涨跌幅和持仓盈亏阈值提醒的创建、编辑、启停和删除行为保持不变
- 组合提醒可为整体组合、A 股（CN）、美股（US）、港股（HK）及每个证券账户分别保存独立的目标配置，互不继承或覆盖
- 目标配置复用**设置 → 投资类别**中的类别、颜色和排序；相对偏离阈值与单票集中度阈值均默认 20%，用于识别类别偏离和过度集中的单个证券
- 组合评估只使用缓存行情和所需缓存汇率。缺失行情或汇率时会标记数据不完整、保留最近一次有效快照，且不会错误地产生或恢复违规状态
- 触发组合违规后可打开全页“AI 调仓建议”。建议以当前组合总资产为限，假设追加资金为 0（不追加新资金）；AI 推荐的新证券会标记为待核实的候选标的。该功能只提供分析建议，不会自动交易或下单
- SQLite 手动备份；可在应用启动时按数据库变更和时间间隔自动备份
- 设置页提供恢复出厂设置，可清空数据库与本地界面偏好

### AI 助手（实验性）

- 流式对话、多会话管理、会话持久化、自动标题和 token 用量展示
- 支持 OpenAI、Anthropic Claude、Ollama、OpenRouter、Kimi、GLM、MiMo 和 DeepSeek
- 可选择是否注入当前组合快照，并通过内置工具查询行情、持仓、交易、绩效、分红和期权数据
- 展示推理内容与工具调用卡片，支持 Markdown、GFM 表格和代码高亮
- Markdown 技能支持关键词自动激活、`/` 手动激活、创建、编辑、克隆、导入/导出和恢复内置版本

AI 功能需要在 **设置 → AI 配置** 中选择提供商和模型；除本地 Ollama 外，通常还需要自行提供 API Key。详见 [AI 助手工具](docs/ai-tools.md) 与 [AI 助手技能](docs/skills.md)。

## 技术栈

| 层 | 技术 |
| --- | --- |
| 桌面框架 | Tauri 2 |
| 前端 | React 19 + TypeScript 7 + Vite 8 |
| UI 与样式 | Ant Design 6 + Tailwind CSS 4 |
| 图表 | ECharts 6 + echarts-for-react |
| 状态管理 | Zustand 5 |
| 后端 | Rust 1.97.1（Tauri Core） |
| 数据库 | SQLite + rusqlite |
| 网络与异步 | reqwest + tokio |
| 日期处理 | chrono（Rust）+ dayjs（前端） |

## 项目结构

```text
stock-portfolio-manager/
├── src/                              # React 前端
│   ├── pages/
│   │   ├── Dashboard/                # 仪表盘
│   │   ├── Statistics/               # 多维统计
│   │   ├── Performance/              # 绩效分析
│   │   ├── Quarterly/                # 季度快照、对比与笔记
│   │   ├── Accounts/                 # 证券账户
│   │   ├── Holdings/                 # 持仓管理与券商 CSV 导入
│   │   ├── Transactions/             # 交易、现金流、CSV/OCR 导入
│   │   ├── Dividends/                # 分红分析
│   │   ├── Options/                  # 期权管理与统计
│   │   ├── Review/                   # 股票与期权操作复盘
│   │   ├── Import/                   # 通用导入导出
│   │   ├── Alerts/                   # 投资提醒：组合提醒与价格提醒
│   │   ├── AiAssistant/              # AI 对话助手
│   │   └── Settings/                 # 通用、投资类别、备份、期权与 AI 设置
│   ├── components/                   # 图表、布局和 AI 展示组件
│   ├── hooks/                        # 主题、盈亏配色、分页等 Hooks
│   ├── stores/                       # Zustand stores
│   ├── types/                        # TypeScript 类型
│   └── styles/                       # 全局样式与主题变量
├── src-tauri/                        # Rust / Tauri 后端
│   ├── src/
│   │   ├── commands/                 # 前端可调用的 Tauri commands
│   │   ├── db/                       # SQLite 初始化、迁移与测试
│   │   ├── models/                   # 后端数据模型
│   │   ├── services/                 # 行情、绩效、AI、复盘等业务逻辑
│   │   ├── skills/                   # 内置 AI 技能 Markdown
│   │   ├── lib.rs                    # 应用初始化与 command 注册
│   │   └── main.rs                   # 桌面应用入口
│   ├── capabilities/                 # Tauri 权限配置
│   ├── Cargo.toml
│   └── tauri.conf.json
├── docs/
│   ├── RELEASE-NOTES.md              # 发布说明
│   ├── ai-tools.md                   # AI 工具说明
│   ├── skills.md                     # AI 技能格式与使用说明
│   └── PRD.md                        # 产品需求文档
├── tools/                            # 数据修复/规范化辅助工具
├── package.json
├── bun.lock
├── rust-toolchain.toml
└── vite.config.ts
```

## 开始开发

### 环境要求

- [Bun](https://bun.sh/)（项目和 CI 的前端包管理/构建工具）
- [Node.js](https://nodejs.org/) >= 26（`package.json` 的引擎要求，并用于运行前端测试）
- [Rust](https://rustup.rs/) 1.97.1（由 `rust-toolchain.toml` 固定）
- 对应平台的 [Tauri 2 系统依赖](https://v2.tauri.app/start/prerequisites/)

### 安装与启动

```bash
# 安装前端依赖
bun install

# 启动 Vite 开发服务和 Tauri 桌面应用
bun run tauri dev
```

### 测试与检查

```bash
# 前端单元测试（Node 26 原生 TypeScript 支持）
node --test

# TypeScript 检查和前端生产构建
bun run build

# Rust 后端测试
cd src-tauri && cargo test --lib
```

## 构建与发布

```bash
# 构建当前平台的安装包
bun run tauri build
```

未显式指定 target 时，构建产物位于 `src-tauri/target/release/bundle/`。指定 target 时，产物位于 `src-tauri/target/<target>/release/bundle/`。

| 平台 | 主要产物 |
| --- | --- |
| macOS | `.dmg`、`.app` |
| Windows | `.msi`、`.exe` |
| Linux | `.deb`、`.AppImage` |

macOS 可按架构构建：

```bash
# Apple Silicon
bun run tauri build -- --target aarch64-apple-darwin

# Intel
bun run tauri build -- --target x86_64-apple-darwin
```

### GitHub Actions

推送 `v*` 标签或在 Actions 页面手动触发 `.github/workflows/build.yml`，CI 会使用 Bun 和 Rust 1.97.1 构建 macOS（Apple Silicon / Intel）、Windows 和 Linux 安装包，并创建包含构建产物的 draft release。

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

## 数据存储

主数据库位于 Tauri 的应用数据目录：`{app_data_dir}/portfolio.db`。当前应用标识为 `com.portfolio.manager`，macOS 上的默认路径为：

```text
~/Library/Application Support/com.portfolio.manager/portfolio.db
```

主要数据表：

| 表 | 用途 |
| --- | --- |
| `accounts`、`categories`、`holdings`、`transactions` | 账户、类别、持仓和交易 |
| `daily_portfolio_values`、`daily_holding_snapshots` | 每日组合与持仓快照 |
| `quarterly_snapshots`、`quarterly_holding_snapshots` | 季度快照、笔记与决策质量 |
| `benchmark_daily_prices` | 绩效基准历史行情缓存 |
| `price_alerts` | 价格/涨跌幅/盈亏提醒规则 |
| `portfolio_alert_configs`、`portfolio_alert_targets`、`portfolio_alert_breaches` | 组合提醒的范围配置、类别目标与当前活动违规 |
| `quote_provider_config` | 各市场行情源、雪球 Cookie 与成本调整设置 |
| `ai_config`、`chat_sessions`、`chat_messages` | AI 配置、会话、消息、推理和工具调用记录 |
| `cached_quotes`、`cached_exchange_rates`、`cached_quote_refresh_time` | 行情、汇率及刷新时间缓存 |
| `option_records`、`stock_splits`、`option_share_lots` | 期权记录、拆股与合约股数配置 |

AI 技能不存储在 SQLite 中，而是保存在 `{app_data_dir}/skills/` 下的 Markdown 文件中。备份设置保存在 `{app_data_dir}/backup_config.json`。

## 数据源与配置

| 数据 | 来源 | 说明 |
| --- | --- | --- |
| 美股 / 港股行情 | 雪球、东方财富、Yahoo Finance | 可按市场选择；雪球失败时依次回退到东方财富、Yahoo Finance |
| A 股行情 | 雪球、东方财富 | 可按市场选择；雪球失败时回退到东方财富 |
| USD / CNY / HKD 汇率 | ExchangeRate-API（`open.er-api.com`） | 内存与 SQLite 双层缓存，网络失败时可使用旧缓存 |
| 绩效基准指数 | Yahoo Finance | S&P 500、NASDAQ、沪深 300、上证指数、恒生指数 |
| AI 模型 | 用户配置的 AI 提供商 | API Key 仅保存在本地 SQLite；请求直接发往所选服务 |

雪球是默认行情源，需要有效的 `xq_a_token` 和 `u`。可在 **设置 → 通用设置 → 雪球 Cookie 设置** 中通过以下任一方式配置：

1. 一键打开雪球登录窗口，登录后自动抓取 Cookie（推荐）。
2. 粘贴浏览器请求中的完整 `Cookie` 字符串，由应用解析。
3. 手动填写 `xq_a_token` 和 `u`。

Cookie 过期后重新执行任一配置方式即可；未配置或请求失败时，应用会按上述行情源规则回退并显示提示。

## 相关文档

- [发布说明](docs/RELEASE-NOTES.md)
- [AI 助手工具](docs/ai-tools.md)
- [AI 助手技能](docs/skills.md)
- [产品需求文档](docs/PRD.md)

## License

本项目基于 [GPL-3.0](LICENSE) 许可证发布。投资分析功能仅供参考，不构成投资建议；使用者需自行承担投资决策与数据安全责任。
