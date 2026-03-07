# Stock Portfolio Manager

A macOS desktop application for managing personal stock portfolios across US, CN (A-shares), and HK markets.

Built with **Tauri 2.0** + **React 18** + **TypeScript** + **Rust** + **SQLite**.

## Features (Phase 1)

- 📊 Multi-account portfolio management across US / CN / HK markets
- 🗂️ Investment category management (4 system presets + custom)
- 📈 Holdings management (add/edit/delete positions)
- 💱 Transaction recording (BUY/SELL with automatic holding updates)
- 🖥️ Dashboard overview

## Tech Stack

| Layer | Technology |
|-------|-----------|
| Desktop Framework | Tauri 2.0 |
| Frontend | React 18 + TypeScript + Vite |
| Styling | TailwindCSS + Ant Design |
| State Management | Zustand |
| Backend | Rust (Tauri Core) |
| Database | SQLite (rusqlite) |

## Project Structure

```
stock-portfolio-manager/
├── src-tauri/              # Rust backend
│   ├── src/
│   │   ├── main.rs         # Entry point
│   │   ├── lib.rs          # App setup + command registration
│   │   ├── db/             # Database init & migrations
│   │   ├── models/         # Data models (Account, Category, Holding, Transaction)
│   │   └── commands/       # Tauri IPC commands
│   └── Cargo.toml
├── src/                    # React frontend
│   ├── pages/              # Dashboard, Accounts, Holdings, Transactions, Categories
│   ├── components/         # Layout components
│   ├── stores/             # Zustand state stores
│   ├── types/              # TypeScript type definitions
│   └── styles/             # Global CSS
├── docs/
│   └── PRD.md              # Product Requirements Document
├── package.json
└── vite.config.ts
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

### Build

```bash
# Build for production
npm run tauri build
```

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

- **accounts** — Securities accounts (US/CN/HK markets)
- **categories** — Investment categories (4 system presets + custom)
- **holdings** — Current positions
- **transactions** — Buy/sell transaction records

### System Categories

| Icon | Name | Color |
|------|------|-------|
| 💵 | 现金类 | `#22C55E` |
| 💰 | 分红股 | `#3B82F6` |
| 🚀 | 成长股 | `#F97316` |
| 🔄 | 套利 | `#8B5CF6` |
