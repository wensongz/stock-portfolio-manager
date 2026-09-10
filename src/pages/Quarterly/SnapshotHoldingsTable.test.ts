// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

test("quarterly holdings table honors the user's selected page size", () => {
  const probe = String.raw`
    globalThis.localStorage = {
      getItem: (key) => key === "holdings_table_page_size" ? "50" : null,
      setItem: () => {},
    };

    const React = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { default: SnapshotHoldingsTable } = await import(
      "./src/pages/Quarterly/SnapshotHoldingsTable.tsx"
    );

    const holdings = Array.from({ length: 25 }, (_, index) => ({
      id: "holding-" + index,
      quarterly_snapshot_id: "snapshot-1",
      account_id: "account-1",
      account_name: "Main",
      symbol: "STOCK" + String(index).padStart(2, "0"),
      name: "Stock " + index,
      market: "US",
      category_name: "成长股",
      category_color: "#1677ff",
      shares: 10,
      avg_cost: 100,
      close_price: 130,
      market_value: 1300,
      cost_value: 1000,
      pnl: 300,
      pnl_percent: 30,
      weight: 4,
      notes: null,
    }));

    const html = renderToStaticMarkup(
      React.createElement(SnapshotHoldingsTable, {
        holdings,
        snapshotId: "snapshot-1",
      }),
    );

    process.stdout.write(String(html.includes("ant-pagination-item-2")));
  `;

  const hasSecondPage = execFileSync("bun", ["--eval", probe], {
    cwd: projectRoot,
    encoding: "utf8",
  }).trim();

  assert.equal(hasSecondPage, "false");
});

test("quarterly holdings renders actual cash currency and signed totals, including cash-only and zero balances", () => {
  const probe = String.raw`
    globalThis.localStorage = { getItem: () => null, setItem: () => {} };
    const React = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { default: SnapshotHoldingsTable } = await import("./src/pages/Quarterly/SnapshotHoldingsTable.tsx");
    const cash = {
      id: "cash", quarterly_snapshot_id: "quarter", account_id: "account", account_name: "HK Account",
      symbol: "$CASH-USD", name: "美元现金", market: "HK", currency: "USD", category_name: "现金类", category_color: "#999",
      shares: -200, avg_cost: 1, close_price: 1, market_value: -200, cost_value: -200, pnl: 0, pnl_percent: null, weight: -25, notes: null,
    };
    const stock = { ...cash, id: "stock", symbol: "AAPL", name: "Apple", market: "US", shares: 10, avg_cost: 100, close_price: 100, market_value: 1000, cost_value: 1000 };
    const render = (holdings) => renderToStaticMarkup(React.createElement(SnapshotHoldingsTable, {
      holdings, snapshotId: "quarter", snap: { exchange_rates: JSON.stringify({ usd_cny: 7, usd_hkd: 7.8, cny_hkd: 7.8 / 7 }) },
    })).replace(/<[^>]*>/g, "");
    process.stdout.write(JSON.stringify([render([stock, cash]), render([cash]), render([{ ...cash, shares: 0, market_value: 0, cost_value: 0 }])]));
  `;
  const [debit, cashOnly, zero] = JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
  assert.match(debit, /\$-200\.00/);
  assert.match(debit, /合计市值 \(USD\)：\$800\.00/);
  assert.match(cashOnly, /合计市值 \(USD\)：\$-200\.00/);
  assert.match(zero, /\$CASH-USD/);
  assert.match(zero, /合计市值 \(USD\)：\$0\.00/);
});

test("quarterly holdings renders USD, CNY, and HKD cash quantities as integers while preserving fractional stock quantities", () => {
  const probe = String.raw`
    globalThis.localStorage = { getItem: () => null, setItem: () => {} };
    const React = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { default: SnapshotHoldingsTable } = await import("./src/pages/Quarterly/SnapshotHoldingsTable.tsx");
    const holding = (id, symbol, shares) => ({
      id, quarterly_snapshot_id: "quarter", account_id: "account-" + id, account_name: "Account " + id,
      symbol, name: symbol, market: "US", currency: symbol.startsWith("$CASH-") ? symbol.slice(6) : "USD",
      category_name: "现金类", category_color: "#999", shares, avg_cost: 1, close_price: 1,
      market_value: shares, cost_value: shares, pnl: 0, pnl_percent: null, weight: 0, notes: null,
    });
    const holdings = [
      holding("usd", "$CASH-USD", 1234.56),
      holding("cny", "$CASH-CNY", 2345.67),
      holding("hkd", "$CASH-HKD", 3456.78),
      holding("stock", "AAPL", 12.345),
    ];
    const html = renderToStaticMarkup(React.createElement(SnapshotHoldingsTable, {
      holdings, snapshotId: "quarter", snap: { exchange_rates: JSON.stringify({ usd_cny: 7, usd_hkd: 7.8, cny_hkd: 7.8 / 7 }) },
    })).replace(/<[^>]*>/g, "");
    process.stdout.write(html);
  `;

  const html = execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" });
  assert.match(html, /\$CASH-USD\$CASH-USD现金类1,235\$1\.000/);
  assert.match(html, /\$CASH-CNY\$CASH-CNY现金类2,346¥1\.000/);
  assert.match(html, /\$CASH-HKD\$CASH-HKD现金类3,457HK\$1\.000/);
  assert.match(html, /AAPLAAPL现金类12\.345\$1\.000/);
});
