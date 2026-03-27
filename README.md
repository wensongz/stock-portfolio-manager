# Stock Portfolio Manager

A desktop application for managing personal stock portfolios across US, CN (A-shares), and HK markets. 
It can run on MacOS, Windows and Linux.

Built with **Tauri 2.0** + **React 18** + **TypeScript** + **Rust** + **SQLite**.

## ✨ Features

### Phase 1 — 基础框架 & 数据管理
- 📊 Multi-account portfolio management across US / CN / HK markets
- 🗂️ Investment category management (4 system presets + custom)
- 📈 Holdings management (add/edit/delete positions)
- 💱 Transaction recording (BUY/SELL with automatic avg cost calculation)
- 🖥️ Dashboard overview

### Phase 2 — 实时数据集成
- 📡 Real-time stock quotes (Yahoo Finance for US/HK, Tencent Finance for CN A-shares)
- 💱 Real-time exchange rates (USD/CNY/HKD) with caching
- 📸 Daily portfolio snapshot auto-collection (market value, cost, P&L)
- ⏱️ Configurable refresh intervals

### Phase 3 — 仓位统计与可视化
- 📊 Dashboard with summary cards (total value, cost, P&L, daily change)
- 🥧 Pie charts — asset allocation by market / category / account
- 📊 Bar charts — P&L comparison across holdings
- 📈 Line charts — portfolio value trend over time
- 🔍 Multi-dimensional statistics: overall / by market / by account / by category

### Phase 4 — Performance Analysis（绩效分析）
- 📐 TWR (Time-Weighted Return) & annualized return calculation
- 📈 Return curve with customizable time range (1W / 1M / 3M / 6M / YTD / 1Y / ALL)
- 🏦 Benchmark comparison (S&P 500, NASDAQ, CSI 300, Hang Seng Index)
- 📉 Maximum drawdown analysis with drawdown area chart
- 🧩 Return attribution (by market / category / holding — waterfall & treemap charts)
- 📅 Monthly returns heatmap table
- 🏆 Top/Bottom 10 holding performance ranking
- ⚠️ Risk metrics: volatility, Sharpe ratio, Calmar ratio

### Phase 5 — 季度分析与持仓思考
- 📸 Quarterly snapshot (auto/manual creation)
- 🔄 Quarter-over-quarter comparison (value, cost, P&L, holding changes)
- 📝 Per-holding investment notes (buy/sell/hold reasoning per quarter)
- 📋 Quarterly overall summary (Markdown editor)
- 📈 Multi-quarter trend charts (stacked area, bar, line)

### Phase 6 — 增强功能 & 优化
- 📥📤 Data import/export (CSV/Excel) with validation & preview
- 📄 Quarterly report export (Markdown / PDF)
- 🔔 Price alerts & notifications (price threshold, change %, P&L triggers)
- 🔍 Historical decision review & tracking (per-stock timeline, decision quality stats)
- 🤖 AI-powered investment analysis (OpenAI integration, portfolio analysis, risk assessment)

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop Framework | Tauri 2.0 |
| Frontend | React 18 + TypeScript + Vite |
| Styling | TailwindCSS + Ant Design |
| Charts | ECharts (echarts-for-react) |
| State Management | Zustand |
| Backend | Rust (Tauri Core) |
| Database | SQLite (rusqlite) |
| HTTP Client | reqwest |
| Async Runtime | tokio |
| Date Handling | chrono (Rust) / dayjs (Frontend) |

## Project Structure

```
stock-portfolio-manager/
├── src-tauri/                        # Rust backend (Tauri Core)
│   ├── src/
│   │   ├── main.rs                   # Entry point
│   │   ├── lib.rs                    # App setup + command registration
│   │   ├── db/                       # Database init & migrations
│   │   ├── models/                   # Data models
│   │   ├── commands/                 # Tauri IPC commands
│   │   ├── services/                 # Business logic services
│   │   │   ├── quote_service.rs      # Real-time stock quotes
│   │   │   ├── exchange_rate_service.rs  # Exchange rates
│   │   │   ├── snapshot_service.rs   # Daily & quarterly snapshots
│   │   │   ├── performance_service.rs    # Performance analysis
│   │   │   └── ai_service.rs         # AI analysis
│   │   └── utils/
│   └── Cargo.toml
├── src/                              # React frontend
│   ├── pages/
│   │   ├── Dashboard/                # Overview dashboard
│   │   ├── Accounts/                 # Account management
│   │   ├── Holdings/                 # Holdings management
│   │   ├── Transactions/             # Transaction records
│   │   ├── Categories/               # Category management
│   │   ├── Statistics/               # Multi-dimensional statistics
│   │   ├── Performance/              # Performance analysis
│   │   ├── Quarterly/                # Quarterly analysis & notes
│   │   ├── Review/                   # Historical decision review
│   │   ├── Alerts/                   # Price alerts
│   │   ├── Import/                   # Data import
│   │   ├── AI/                       # AI analysis
│   │   └── Settings/                 # App settings
│   ├── components/
│   │   ├── charts/                   # Reusable chart components (Pie, Bar, Line, etc.)
│   │   └── layout/                   # Layout components
│   ├── hooks/                        # Custom React hooks
│   ├── stores/                       # Zustand state stores
│   ├── types/                        # TypeScript type definitions
│   ├── utils/                        # Utility functions
│   └── styles/                       # Global CSS
├── docs/
│   └── PRD.md                        # Product Requirements Document (v3.0)
├── package.json
├── vite.config.ts
├── tailwind.config.js
└── tsconfig.json
```

## Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) >= 18
- [Rust](https://rustup.rs/) >= 1.70
- Tauri system dependencies ([guide](https://tauri.app/v2/guides/getting-started/prerequisites))

### Development

```bash
# Install frontend dependencies
npm install

# Run in development mode (starts both Vite dev server and Tauri)
npm run tauri dev
```

### Build & Package

```bash
# Build for production (generates platform-specific installers)
npm run tauri build
```

Build output is located in `src-tauri/target/release/bundle/`:

| Platform | Output | Path |
|----------|--------|------|
| macOS | `.dmg` installer | `bundle/dmg/stock-portfolio-manager_<version>_<arch>.dmg` |
| macOS | `.app` bundle | `bundle/macos/stock-portfolio-manager.app` |
| Windows | `.msi` installer | `bundle/msi/` |
| Linux | `.deb` / `.AppImage` | `bundle/deb/` / `bundle/appimage/` |

#### Generating .dmg (macOS)

On a Mac, simply run:

```bash
npm run tauri build
```

The `.dmg` file will be at:
- Apple Silicon: `src-tauri/target/release/bundle/dmg/stock-portfolio-manager_0.1.0_aarch64.dmg`
- Intel Mac: `src-tauri/target/release/bundle/dmg/stock-portfolio-manager_0.1.0_x64.dmg`

To build for a specific architecture:

```bash
# Apple Silicon (M1/M2/M3)
npm run tauri build -- --target aarch64-apple-darwin

# Intel
npm run tauri build -- --target x86_64-apple-darwin
```

#### Automated Builds (CI/CD)

Push a version tag to trigger the GitHub Actions workflow that builds installers for all platforms:

```bash
git tag v0.1.0
git push origin v0.1.0
```

The workflow produces `.dmg` (macOS), `.msi` (Windows), and `.deb`/`.AppImage` (Linux) as draft release assets. You can also trigger it manually from the **Actions** tab.

### Tests

```bash
# Run Rust backend tests
cd src-tauri && cargo test

# Type-check frontend
npx tsc --noEmit
```

## Database

SQLite database is stored in the system app data directory:
- macOS: `~/Library/Application Support/com.portfolio.manager/portfolio.db`

### Tables

| Table | Description |
|-------|-------------|
| **accounts** | Securities accounts (US/CN/HK markets) |
| **categories** | Investment categories (4 system presets + custom) |
| **holdings** | Current positions |
| **transactions** | Buy/sell transaction records |
| **daily_portfolio_values** | Daily portfolio net value snapshots |
| **daily_holding_snapshots** | Daily per-holding snapshots (close price, market value) |
| **quarterly_snapshots** | Quarterly portfolio snapshots with notes |
| **quarterly_holding_snapshots** | Quarterly per-holding snapshots with investment notes |
| **benchmark_daily_prices** | Cached benchmark index historical prices |
| **price_alerts** | Price alert rules and trigger history |
| **ai_config** | AI service configuration |

### System Categories

| Icon | Name | Color |
|------|------|-------|
| 💵 | 现金类 | `#22C55E` |
| 💰 | 分红股 | `#3B82F6` |
| 🚀 | 成长股 | `#F97316` |
| 🔄 | 套利 | `#8B5CF6` |

## Data Sources

| Data | Source | Markets |
|------|--------|---------|
| US Stock Quotes | Xueqiu API, Yahoo Finance API | 🇺🇸 US |
| US & HK Stock Quotes | Xueqiu API, Yahoo Finance API | 🇭🇰 HK |
| CN A-Share Quotes | Xueqiu API, EastMoney API | 🇨🇳 CN |
| Exchange Rates | ExchangeRate-API | USD/CNY/HKD |
| Benchmark Indices | Yahoo Finance API | S&P 500, NASDAQ, CSI 300, HSI |

You need to configure Xueqiu cookie in the setting in order to access Xueqiu API.

## License

It is released under the GPL-3.0 license. Use it at your own risk.
