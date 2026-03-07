# 📊 Stock Portfolio Manager

A desktop application for managing stock portfolios across US, CN, and HK markets, built with Tauri 2.0, React 18, and SQLite.

## Features (Phase 1)

- **Securities Account Management** — Create and manage multiple brokerage accounts per market (US/CN/HK)
- **Investment Categories** — 4 preset categories (现金类, 分红股, 成长股, 套利) + custom categories
- **Holdings Management** — Track stock positions with average cost, shares, and category
- **Transaction Management** — Record buy/sell transactions with automatic holding updates (weighted average cost)

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop Framework | Tauri 2.0 |
| Frontend | React 18 + TypeScript + Vite |
| UI Components | Ant Design |
| State Management | Zustand |
| Backend | Rust |
| Database | SQLite (rusqlite) |
| Date Handling | chrono (Rust) / dayjs (Frontend) |

## Getting Started

### Prerequisites

- [Node.js](https://nodejs.org/) (v18+)
- [Rust](https://www.rust-lang.org/tools/install)
- Tauri system dependencies ([see docs](https://v2.tauri.app/start/prerequisites/))

### Development

```bash
# Install frontend dependencies
npm install

# Run in development mode (starts both frontend dev server and Tauri)
npm run tauri dev

# Run Rust backend tests
cd src-tauri && cargo test
```

### Build

```bash
# Build the application
npm run tauri build
```

## Project Structure

```
stock-portfolio-manager/
├── src/                    # React frontend
│   ├── api/                # Tauri command API wrappers
│   ├── pages/              # Page components (Accounts, Categories, Holdings, Transactions)
│   ├── types/              # TypeScript type definitions
│   └── App.tsx             # Main app with routing
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── commands/       # Tauri command handlers
│   │   ├── db/             # Database initialization & migrations
│   │   ├── models/         # Data models
│   │   └── services/       # Business logic with tests
│   └── Cargo.toml
└── docs/
    └── PRD.md              # Product Requirements Document
```

## License

GPL-3.0
