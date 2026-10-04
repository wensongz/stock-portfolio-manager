// @ts-nocheck -- Node runs Bun probes against the real TSX pages.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

function renderPages(currency, exchangeRates) {
  const probe = String.raw`
    globalThis.localStorage = {
      getItem: key => key === "base_currency" ? ${JSON.stringify(currency)} : null,
      setItem() {},
    };
    const React = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { MemoryRouter } = await import("react-router-dom");
    const { mock } = await import("bun:test");
    const snapshot = {
      id: "quarter-test", quarter: "2026Q2", snapshot_date: "2026-06-30",
      total_value: 100, total_cost: 120, total_pnl: -20,
      us_value: 100, us_cost: 120, cn_value: 0, cn_cost: 0, hk_value: 0, hk_cost: 0,
      exchange_rates: ${JSON.stringify(exchangeRates)}, overall_notes: null,
      created_at: "2026-06-30", holding_count: 1,
    };
    mock.module("./src/stores/quarterlyStore", () => ({ useQuarterlyStore: () => ({
      snapshots: [snapshot], listLoading: false, listError: null, mutationLoading: false,
      mutationError: null, initializationLoading: false, initializationError: null,
      initializeSnapshots() {}, createSnapshot() {}, deleteSnapshot() {},
    }) }));
    const pages = await Promise.all([
      import("./src/pages/Quarterly/index.tsx"),
      import("./src/pages/Dashboard/index.tsx"),
      import("./src/pages/Statistics/index.tsx"),
    ]);
    process.stdout.write(JSON.stringify(pages.map(({ default: Page }) =>
      renderToStaticMarkup(React.createElement(MemoryRouter, null, React.createElement(Page))).replace(/<[^>]*>/g, "")
    )));
  `;
  return JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
}

test("quarterly, dashboard and statistics use the same saved base currency", () => {
  for (const [currency, label, value, loss] of [
    ["CNY", "CNY 人民币", "¥700.00", "¥-140.00"],
    ["HKD", "HKD 港元", "HK$780.00", "HK$-156.00"],
    ["USD", "USD 美元", "$100.00", "$-20.00"],
  ]) {
    const [quarterly, dashboard, statistics] = renderPages(currency, '{"usd_cny":7,"usd_hkd":7.8}');
    for (const page of [quarterly, dashboard, statistics]) {
      assert.ok(page.includes("基准货币:"));
      assert.ok(page.includes(label));
    }
    assert.ok(quarterly.includes(`总市值 (${currency})`));
    assert.ok(quarterly.includes(value));
    assert.ok(quarterly.includes(loss));
  }
});

test("quarterly list shows missing historical rates instead of relabeling USD amounts", () => {
  const [quarterly] = renderPages("CNY", "{}");
  assert.match(quarterly, /缺少有效快照汇率/);
  assert.ok(!quarterly.includes("¥100.00"));
  assert.ok(!quarterly.includes("¥0.00"));
  const [usd] = renderPages("USD", "{}");
  assert.ok(usd.includes("$100.00"));
});
