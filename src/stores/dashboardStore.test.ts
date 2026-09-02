// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { createDashboardStore } from "./dashboardStore.ts";

const report = {
  summary: {
    total_market_value: 120,
    total_cost: 100,
    total_pnl: 20,
    total_pnl_percent: 20,
    daily_pnl: 10,
    us_market_value: 120,
    cn_market_value: 0,
    hk_market_value: 0,
    exchange_rates: {
      usd_cny: 7,
      usd_hkd: 7.8,
      cny_hkd: 1.114,
      updated_at: "now",
    },
    base_currency: "USD",
  },
  holdings: [{ id: "holding-aapl", symbol: "AAPL" }],
};

test("dashboard refresh uses one report command and updates atomically", async () => {
  const calls = [];
  const store = createDashboardStore(async (command, args) => {
    calls.push([command, args]);
    return report;
  });

  await store.getState().fetchReport("USD");

  assert.deepEqual(calls, [
    ["get_dashboard_report", { baseCurrency: "USD" }],
  ]);
  assert.equal(store.getState().summary, report.summary);
  assert.equal(store.getState().holdingDetails, report.holdings);
});

test("failed dashboard refresh preserves the last complete report", async () => {
  let attempt = 0;
  const store = createDashboardStore(async () => {
    attempt += 1;
    if (attempt === 1) return report;
    throw new Error("offline");
  });

  await store.getState().fetchReport("USD");
  await store.getState().fetchReport("CNY");

  assert.equal(store.getState().summary, report.summary);
  assert.equal(store.getState().holdingDetails, report.holdings);
  assert.match(store.getState().error ?? "", /offline/);
});
