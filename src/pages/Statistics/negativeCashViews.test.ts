// @ts-nocheck -- Node runs a Bun render probe against the actual statistics tables.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

function renderCashViews(credit: number) {
  const probe = String.raw`
    import { mock } from "bun:test";
    import { Window } from "happy-dom";
    globalThis.localStorage = { getItem: () => null, setItem: () => {} };
    const React = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { MemoryRouter } = await import("react-router-dom");
    const store = await import("./src/stores/statisticsStore.ts");
    const h = React.createElement;
    const cash = (currency, balance, accountId = "account-a") => ({
      id: accountId + currency, account_id: accountId, account_name: accountId,
      symbol: "$CASH-" + currency, name: currency + " Cash", market: currency === "CNY" ? "CN" : "HK",
      category_name: "现金类", category_color: "#999", currency,
      shares: balance, avg_cost: 1, current_price: 1, cost_value: balance, market_value: balance,
      market_value_usd: balance / (currency === "CNY" ? 7 : 7.8),
      pnl: 0, pnl_percent: null, daily_pnl: 0, daily_change_percent: null,
    });
    const credit = ${credit};
    const holdings = [cash("CNY", 7000), cash("HKD", -1560)];
    if (credit) holdings.push(cash("HKD", credit, "account-b"));
    const stats = {
      holdings, total_market_value: 800 + credit / 7.8, total_cost: 800 + credit / 7.8,
      total_pnl: 0, total_pnl_percent: 0, market_distribution: [], category_distribution: [],
      account_distribution: [], stock_distribution: [], top_gainers: [], top_losers: [],
      is_cash_category: true,
    };
    const stock = { ...cash("HKD", 7800), id: "stock", symbol: "700.HK", name: "Tencent",
      shares: 10, avg_cost: 780, current_price: 780, category_name: "stock" };
    store.useStatisticsStore.setState({
      overviewByCurrency: { USD: stats },
      marketStats: { HK: { ...stats, holdings: [stock, ...holdings.slice(1)],
        total_market_value: 6240 + credit, total_cost: 6240 + credit } },
      categoryStats: { "cash:USD": stats },
    });
    // SSR reads a supplied report snapshot; all calculations and table rendering stay real.
    const readState = store.useStatisticsStore.getState;
    mock.module("./src/stores/statisticsStore", () => ({
      ...store, useStatisticsStore: () => readState(),
    }));
    const { default: OverviewTab } = await import("./src/pages/Statistics/OverviewTab.tsx");
    const { default: MarketTab } = await import("./src/pages/Statistics/MarketTab.tsx");
    const { default: CategoryTab } = await import("./src/pages/Statistics/CategoryTab.tsx");
    const views = [
      h(OverviewTab, { baseCurrency: "USD" }),
      h(MarketTab, { selectedMarket: "HK", onMarketChange() {} }),
      h(CategoryTab, { selectedCategoryId: "cash", baseCurrency: "USD", onCategoryChange() {} }),
    ];
    const dom = new Window();
    const result = views.map(view => {
      dom.document.body.innerHTML = renderToStaticMarkup(h(MemoryRouter, null, view));
      const row = dom.document.querySelector('tr[data-row-key="$CASH-HKD"]');
      const headers = [...dom.document.querySelectorAll("thead th")].map(th => th.textContent);
      return row ? Object.fromEntries([...row.querySelectorAll("td")].map((td, i) => [headers[i], td.textContent])) : null;
    });
    await dom.happyDOM.close();
    process.stdout.write(JSON.stringify(result));
  `;
  try {
    return JSON.parse(execFileSync("bun", ["--eval", probe], {
      cwd: projectRoot, encoding: "utf8", timeout: 30000, stdio: ["ignore", "pipe", "pipe"],
    }));
  } catch (error) {
    throw new Error(`Statistics render probe failed: ${error.stderr || error.message}`);
  }
}

test("statistics views render overdrafts with unit cost and net position weights", async (t) => {
  const views = renderCashViews(0);
  for (const [index, row] of views.entries()) {
    await t.test(["overview", "market", "cash category"][index], () => {
      assert.ok(row, "negative HKD cash must be visible");
      assert.equal(row["均价"], "1.000");
      assert.match(row["市值"], /-HK\$1,560\.00/);
      assert.equal(row["仓位%"], "-25.00%");
    });
  }
});

test("offsetting account cash balances retain unit cost when their aggregate is zero", async (t) => {
  for (const [index, row] of renderCashViews(1560).entries()) {
    await t.test(["overview", "market", "cash category"][index], () => {
      assert.ok(row);
      assert.equal(row["均价"], "1.000");
      assert.equal(row["仓位%"], "0.00%");
    });
  }
});
