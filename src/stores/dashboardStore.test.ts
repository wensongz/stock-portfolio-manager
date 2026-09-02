// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { createDashboardStore } from "./dashboardStore.ts";

function report(baseCurrency = "USD") {
  return {
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
      base_currency: baseCurrency,
    },
    holdings: [{ id: `holding-${baseCurrency}`, symbol: "AAPL" }],
  };
}

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

test("dashboard refresh uses one report command and updates atomically", async () => {
  const response = report();
  const calls = [];
  const store = createDashboardStore(async (command, args) => {
    calls.push([command, args]);
    return response;
  });

  await store.getState().fetchReport("USD");

  assert.deepEqual(calls, [
    ["get_dashboard_report", { baseCurrency: "USD" }],
  ]);
  assert.equal(store.getState().summary, response.summary);
  assert.equal(store.getState().holdingDetails, response.holdings);
});

test("failed dashboard refresh preserves the last complete report", async () => {
  const response = report();
  let attempt = 0;
  const store = createDashboardStore(async () => {
    attempt += 1;
    if (attempt === 1) return response;
    throw new Error("offline");
  });

  await store.getState().fetchReport("USD");
  await store.getState().fetchReport("CNY");

  assert.equal(store.getState().summary, response.summary);
  assert.equal(store.getState().holdingDetails, response.holdings);
  assert.match(store.getState().error ?? "", /offline/);
});

test("same-currency concurrent dashboard loads share one in-flight command", async () => {
  const pending = deferred();
  let calls = 0;
  const store = createDashboardStore(async () => {
    calls += 1;
    return pending.promise;
  });

  const first = store.getState().fetchReport("USD");
  const second = store.getState().fetchReport("USD");
  pending.resolve(report("USD"));
  await Promise.all([first, second]);

  assert.equal(calls, 1);
  assert.equal(store.getState().loading, false);

  await store.getState().fetchReport("USD");
  assert.equal(calls, 2);
});

test("latest dashboard currency wins when successes resolve out of order", async () => {
  const requests = new Map([
    ["USD", deferred()],
    ["CNY", deferred()],
  ]);
  const store = createDashboardStore(async (_command, args) =>
    requests.get(args.baseCurrency).promise,
  );

  const usd = store.getState().fetchReport("USD");
  const cny = store.getState().fetchReport("CNY");
  requests.get("CNY").resolve(report("CNY"));
  await cny;
  requests.get("USD").resolve(report("USD"));
  await usd;

  assert.equal(store.getState().summary?.base_currency, "CNY");
  assert.equal(store.getState().holdingDetails[0]?.id, "holding-CNY");
  assert.equal(store.getState().loading, false);
  assert.equal(store.getState().error, null);
});

test("stale dashboard failure cannot overwrite a newer success", async () => {
  const requests = new Map([
    ["USD", deferred()],
    ["HKD", deferred()],
  ]);
  const store = createDashboardStore(async (_command, args) =>
    requests.get(args.baseCurrency).promise,
  );

  const usd = store.getState().fetchReport("USD");
  const hkd = store.getState().fetchReport("HKD");
  requests.get("HKD").resolve(report("HKD"));
  await hkd;
  requests.get("USD").reject(new Error("stale offline"));
  await usd;

  assert.equal(store.getState().summary?.base_currency, "HKD");
  assert.equal(store.getState().loading, false);
  assert.equal(store.getState().error, null);
});
