// @ts-nocheck -- Node drives a Bun SSR probe against the actual page and store.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

test("snapshot errors remain visible and successful refresh immediately renders restored cash and totals", () => {
  const probe = String.raw`
    globalThis.localStorage = { getItem: () => null, setItem: () => {} };
    const React = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { MemoryRouter, Routes, Route } = await import("react-router-dom");
    const { createQuarterlyStore } = await import("./src/stores/quarterlyStore.ts");
    const { mock } = await import("bun:test");
    let activeState;
    mock.module("./src/stores/quarterlyStore.ts", () => ({ createQuarterlyStore, useQuarterlyStore: () => activeState }));
    const { default: SnapshotDetail } = await import("./src/pages/Quarterly/SnapshotDetail.tsx");
    const render = (state) => {
      activeState = state;
      return renderToStaticMarkup(React.createElement(MemoryRouter, { initialEntries: ["/quarterly/quarter-test"] },
        React.createElement(Routes, null, React.createElement(Route, { path: "/quarterly/:snapshotId", element: React.createElement(SnapshotDetail) }))
      )).replace(/<[^>]*>/g, "");
    };
    const stock = { id: "stock", quarterly_snapshot_id: "quarter-test", account_id: "account-test", account_name: "Test Account", symbol: "TEST", name: "Example", market: "US", currency: "USD", category_name: "股票", category_color: "#999", shares: 10, avg_cost: 100, close_price: 100, market_value: 1000, cost_value: 1000, pnl: 0, pnl_percent: 0, weight: 100, notes: null };
    const original = { snapshot: { id: "quarter-test", quarter: "2026Q2", total_value: 1000, total_cost: 1000, total_pnl: 0, us_value: 1000, us_cost: 1000, cn_value: 0, cn_cost: 0, hk_value: 0, hk_cost: 0, holding_count: 1, exchange_rates: JSON.stringify({ usd_cny: 7, usd_hkd: 7.8 }), overall_notes: null }, holdings: [stock] };
    const base = { detailSnapshotId: "quarter-test", detail: original, detailLoading: false, mutationLoading: false, detailError: null, mutationError: null, quarterlyTransactions: [] };
    const failed = createQuarterlyStore(async (command) => {
      if (command === "refresh_quarterly_snapshot") throw new Error("测试：历史汇率读取失败");
      return [];
    });
    failed.setState(base);
    await failed.getState().refreshSnapshot("quarter-test");
    const failureText = render(failed.getState());
    const retained = failed.getState().detail === original;
    const initialErrorText = render({ ...base, detail: null, detailError: "测试：无法读取快照", mutationError: null });
    const mutationErrorText = render({ ...base, mutationError: "测试：快照写入失败" });
    const restored = { snapshot: { ...original.snapshot, total_value: 1300, total_cost: 1300, us_value: 1100, us_cost: 1100, cn_value: 700, cn_cost: 700, hk_value: 780, hk_cost: 780, holding_count: 4 }, holdings: [stock, ...[["USD", "US", 100], ["CNY", "CN", 700], ["HKD", "HK", 780]].map(([currency, market, amount]) => ({ ...stock, id: "cash-" + currency, symbol: "$CASH-" + currency, name: currency + " Cash", market, currency, category_name: "现金", shares: amount, avg_cost: 1, close_price: 1, market_value: amount, cost_value: amount, weight: 10 }))] };
    const success = createQuarterlyStore(async (command) => command === "refresh_quarterly_snapshot" ? restored : []);
    success.setState(base);
    await success.getState().refreshSnapshot("quarter-test");
    const successText = render(success.getState());
    process.stdout.write(JSON.stringify({ failureText, retained, initialErrorText, mutationErrorText, successText, rowCount: success.getState().detail.holdings.length }));
  `;
  const result = JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
  assert.equal(result.retained, true);
  assert.match(result.failureText, /历史汇率读取失败/);
  assert.match(result.failureText, /仍显示上次加载的快照/);
  assert.match(result.failureText, /重试刷新/);
  assert.match(result.initialErrorText, /无法读取快照/);
  assert.doesNotMatch(result.initialErrorText, /不存在或已删除/);
  assert.match(result.initialErrorText, /重新加载/);
  assert.match(result.mutationErrorText, /快照写入失败/);
  assert.equal(result.rowCount, 4);
  for (const currency of ["USD", "CNY", "HKD"]) assert.ok(result.successText.includes("$CASH-" + currency));
  assert.match(result.successText, /总市值 \(USD\)\$1,300\.00/);
  assert.match(result.successText, /合计市值 \(USD\)：\$1,300\.00/);
  assert.doesNotMatch(result.successText, /仍显示上次加载的快照|读取失败/);
});
