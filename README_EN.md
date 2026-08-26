# Stock Portfolio Manager

[简体中文](README.md) | **English**

A local-first desktop portfolio manager for individual investors and small investment firms. It brings multi-account holdings, transactions, performance analysis, and investment reviews across the US, mainland China, and Hong Kong markets into one application.

Platforms: **macOS / Windows / Linux**

The application is built with **Tauri 2 + React 19 + TypeScript + Rust + SQLite**. Portfolio data, configuration, and AI conversations are stored locally by default. Market quotes, exchange rates, and configured AI services require network access. Its current single-machine data model is best suited to independent investors and small investment teams that do not require complex member permissions or cloud collaboration.

## ✨ Features

### Accounts, holdings, and transactions

- Multiple brokerage accounts across the US, mainland China, and Hong Kong markets
- Four built-in investment categories—Cash, Dividend, Growth, and Arbitrage—plus custom categories
- Create, edit, delete, filter, and track holdings with real-time profit and loss
- `BUY`, `SELL`, `OPEN`, and `PAY` transactions with automatic position quantity and average-cost updates
- Cash deposits and withdrawals with balance validation; per-market settings control whether sales and dividends adjust the holding cost basis
- Remembered account filters, page sizes, and other table preferences on the Holdings and Transactions pages

### Quotes, dashboard, and portfolio statistics

- Per-market selection of Xueqiu, East Money, or Yahoo Finance quote providers, with caching and fallback behavior
- Background quote refresh after startup, with the latest quote data and refresh time persisted locally
- Live USD / CNY / HKD exchange rates, persistent caching, and base-currency conversion
- Dashboard cards for market value, cost, cumulative P&L, daily P&L, and holding details
- Allocation, P&L, and security-level breakdowns by market, account, and investment category
- China-style or US-style P&L colors, plus light, dark, and system themes

### Performance and quarterly analysis

- Time-Weighted Return (TWR), cumulative and annualized returns, volatility, Sharpe ratio, and Calmar ratio
- Return curves over custom date ranges, with comparisons against the S&P 500, NASDAQ, CSI 300, Shanghai Composite, and Hang Seng Index
- Maximum drawdown, return attribution, monthly returns, and top/bottom holding rankings
- Automatic backfilling of missing historical holding snapshots, with daily portfolio and holding snapshots
- Quarterly snapshot creation and refresh, quarter-over-quarter comparisons, multi-quarter trends, and quarterly transaction reviews
- Per-holding investment notes, quarterly summaries, and a historical notes timeline

### Dividends, options, and operation reviews

- Dividend and interest summaries from `PAY` transactions, grouped by year, market, account, and security
- Options CSV import/export, active and expired contract statistics, and Sell Put / Sell Call simulations
- Chinese and English CSV headers, contract-state validation, stock-split ratios, and configurable shares per contract
- Stock operation reviews based on quarterly snapshots, including decision-quality labels
- Option operation reviews that match opening and closing records into Campaigns and analyze premium income, retention rate, annualized yield on secured notional, the worst Campaign, and data quality
- One-click handoff of deterministic option-review results to the AI assistant for further analysis

### Import, alerts, and data safety

- General CSV import/export and dedicated holding/transaction imports for Interactive Brokers, Moomoo, Firstrade, and Tonghuashun formats
- OCR import for mainland China trade screenshots, with preview and validation before confirmation
- Price, percentage-change, and holding-P&L threshold rules with trigger-state tracking
- Manual SQLite backups and optional startup backups based on database changes and the backup interval
- A factory-reset option that clears the database and locally stored UI preferences

### AI assistant (experimental)

- Streaming responses, multiple conversations, persisted chat history, automatic titles, and token-usage display
- OpenAI, Anthropic Claude, Ollama, OpenRouter, Kimi, GLM, MiMo, and DeepSeek providers
- Optional portfolio-context injection and built-in tools for quotes, holdings, transactions, performance, dividends, and options data
- Reasoning and tool-call cards, Markdown and GFM tables, and syntax-highlighted code blocks
- Markdown skills with keyword-based automatic activation, `/` manual activation, creation, editing, cloning, import/export, and built-in skill restoration

Configure the provider and model under **Settings → AI Configuration**. Most remote providers require your own API key; local Ollama does not. See [AI assistant tools](docs/ai-tools.md) and [AI assistant skills](docs/skills.md). These linked documents are currently maintained in Chinese.

## Technology stack

| Layer | Technology |
| --- | --- |
| Desktop framework | Tauri 2 |
| Frontend | React 19 + TypeScript 7 + Vite 8 |
| UI and styling | Ant Design 6 + Tailwind CSS 4 |
| Charts | ECharts 6 + echarts-for-react |
| State management | Zustand 5 |
| Backend | Rust 1.97.1 (Tauri Core) |
| Database | SQLite + rusqlite |
| Networking and async runtime | reqwest + tokio |
| Date handling | chrono (Rust) + dayjs (frontend) |

## Project structure

```text
stock-portfolio-manager/
├── src/                              # React frontend
│   ├── pages/
│   │   ├── Dashboard/                # Portfolio dashboard
│   │   ├── Statistics/               # Multi-dimensional statistics
│   │   ├── Performance/              # Performance analysis
│   │   ├── Quarterly/                # Quarterly snapshots, comparisons, and notes
│   │   ├── Accounts/                 # Brokerage accounts
│   │   ├── Holdings/                 # Holdings and broker CSV imports
│   │   ├── Transactions/             # Transactions, cash flow, and CSV/OCR imports
│   │   ├── Dividends/                # Dividend analysis
│   │   ├── Options/                  # Option management and statistics
│   │   ├── Review/                   # Stock and option operation reviews
│   │   ├── Import/                   # General import and export
│   │   ├── Alerts/                   # Price alerts
│   │   ├── AiAssistant/              # Conversational AI assistant
│   │   └── Settings/                 # General, category, backup, option, and AI settings
│   ├── components/                   # Charts, layout, and AI display components
│   ├── hooks/                        # Theme, P&L color, pagination, and other hooks
│   ├── stores/                       # Zustand stores
│   ├── types/                        # TypeScript types
│   └── styles/                       # Global styles and theme variables
├── src-tauri/                        # Rust / Tauri backend
│   ├── src/
│   │   ├── commands/                 # Tauri commands exposed to the frontend
│   │   ├── db/                       # SQLite initialization, migrations, and tests
│   │   ├── models/                   # Backend data models
│   │   ├── services/                 # Quotes, performance, AI, reviews, and business logic
│   │   ├── skills/                   # Built-in AI skills in Markdown
│   │   ├── lib.rs                    # Application setup and command registration
│   │   └── main.rs                   # Desktop application entry point
│   ├── capabilities/                 # Tauri permissions
│   ├── Cargo.toml
│   └── tauri.conf.json
├── docs/
│   ├── RELEASE-NOTES.md              # Release notes
│   ├── ai-tools.md                   # AI tools documentation (Chinese)
│   ├── skills.md                     # AI skills documentation (Chinese)
│   └── PRD.md                        # Product requirements document (Chinese)
├── tools/                            # Data repair and normalization utilities
├── package.json
├── bun.lock
├── rust-toolchain.toml
└── vite.config.ts
```

## Development

### Prerequisites

- [Bun](https://bun.sh/), used for frontend package management and builds locally and in CI
- [Node.js](https://nodejs.org/) >= 26, required by `package.json` and used to run frontend tests
- [Rust](https://rustup.rs/) 1.97.1, pinned by `rust-toolchain.toml`
- The [Tauri 2 system dependencies](https://v2.tauri.app/start/prerequisites/) for your platform

### Install and run

```bash
# Install frontend dependencies
bun install

# Start the Vite development server and Tauri desktop application
bun run tauri dev
```

### Tests and checks

```bash
# Frontend unit tests using Node 26 native TypeScript support
node --test

# TypeScript checks and the frontend production build
bun run build

# Rust backend tests
cd src-tauri && cargo test --lib
```

## Build and release

```bash
# Build installers for the current platform
bun run tauri build
```

Without an explicit target, artifacts are written to `src-tauri/target/release/bundle/`. With a target, they are written to `src-tauri/target/<target>/release/bundle/`.

| Platform | Main artifacts |
| --- | --- |
| macOS | `.dmg`, `.app` |
| Windows | `.msi`, `.exe` |
| Linux | `.deb`, `.AppImage` |

Build macOS packages for a specific architecture:

```bash
# Apple Silicon
bun run tauri build -- --target aarch64-apple-darwin

# Intel
bun run tauri build -- --target x86_64-apple-darwin
```

### GitHub Actions

Push a `v*` tag or manually trigger `.github/workflows/build.yml` from the Actions page. CI uses Bun and Rust 1.97.1 to build macOS (Apple Silicon and Intel), Windows, and Linux packages, then creates a draft release containing the artifacts.

```bash
git tag vX.Y.Z
git push origin vX.Y.Z
```

## Data storage

The main database is stored at `{app_data_dir}/portfolio.db` in the Tauri application data directory. The current application identifier is `com.portfolio.manager`. On macOS, the default path is:

```text
~/Library/Application Support/com.portfolio.manager/portfolio.db
```

Main database tables:

| Tables | Purpose |
| --- | --- |
| `accounts`, `categories`, `holdings`, `transactions` | Accounts, categories, holdings, and transactions |
| `daily_portfolio_values`, `daily_holding_snapshots` | Daily portfolio and holding snapshots |
| `quarterly_snapshots`, `quarterly_holding_snapshots` | Quarterly snapshots, notes, and decision quality |
| `benchmark_daily_prices` | Cached historical benchmark prices |
| `price_alerts` | Price, percentage-change, and P&L alert rules |
| `quote_provider_config` | Per-market providers, Xueqiu cookies, and cost-basis settings |
| `ai_config`, `chat_sessions`, `chat_messages` | AI configuration, conversations, messages, reasoning, and tool-call records |
| `cached_quotes`, `cached_exchange_rates`, `cached_quote_refresh_time` | Cached quotes, exchange rates, and quote refresh time |
| `option_records`, `stock_splits`, `option_share_lots` | Option records, stock splits, and shares-per-contract settings |

AI skills are not stored in SQLite. They are Markdown files under `{app_data_dir}/skills/`. Backup settings are stored in `{app_data_dir}/backup_config.json`.

## Data sources and configuration

| Data | Source | Notes |
| --- | --- | --- |
| US / Hong Kong quotes | Xueqiu, East Money, Yahoo Finance | Selectable per market; Xueqiu failures fall back to East Money and then Yahoo Finance |
| Mainland China quotes | Xueqiu, East Money | Selectable per market; Xueqiu failures fall back to East Money |
| USD / CNY / HKD exchange rates | ExchangeRate-API (`open.er-api.com`) | In-memory and SQLite caches; stale data can be used when the network is unavailable |
| Performance benchmarks | Yahoo Finance | S&P 500, NASDAQ, CSI 300, Shanghai Composite, and Hang Seng Index |
| AI models | The user-configured AI provider | API keys are stored only in local SQLite; requests go directly to the selected service |

Xueqiu is the default quote provider and requires valid `xq_a_token` and `u` cookies. Configure them under **Settings → General Settings → Xueqiu Cookie Settings** in one of three ways:

1. Open the Xueqiu login window and capture the cookies automatically after signing in (recommended).
2. Paste a complete browser `Cookie` header and let the application parse it.
3. Enter `xq_a_token` and `u` manually.

Repeat any method after the cookies expire. If the credentials are missing or a request fails, the application follows the fallback rules above and displays an actionable warning.

## Related documentation

- [Release notes](docs/RELEASE-NOTES.md) (Chinese)
- [AI assistant tools](docs/ai-tools.md) (Chinese)
- [AI assistant skills](docs/skills.md) (Chinese)
- [Product requirements document](docs/PRD.md) (Chinese)

## License

This project is released under the [GPL-3.0](LICENSE) license. Investment-analysis features are for informational purposes only and do not constitute investment advice. Users are responsible for their investment decisions and data security.
