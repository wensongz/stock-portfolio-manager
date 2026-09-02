// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { createDashboardStore } from "../../stores/dashboardStore.ts";
import { refreshDashboardQuotes } from "./dashboardRefresh.ts";

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

test("dashboard quote refresh requests the latest currency after awaiting quotes", async () => {
  const quoteRefresh = deferred();
  let baseCurrency = "USD";
  const requestedCurrencies = [];

  const refreshing = refreshDashboardQuotes({
    fetchHoldingQuotes: () => quoteRefresh.promise,
    fetchReport: async (currency) => {
      requestedCurrencies.push(currency);
    },
    getBaseCurrency: () => baseCurrency,
  });
  baseCurrency = "CNY";
  quoteRefresh.resolve();
  await refreshing;

  assert.deepEqual(requestedCurrencies, ["CNY"]);
});

test("dashboard StrictMode loads share work while quote refresh forces a fresh store request", async () => {
  const staleReport = deferred();
  const freshReport = deferred();
  const responses = [staleReport, freshReport];
  let invokeCalls = 0;
  const store = createDashboardStore(async () => {
    const response = responses[invokeCalls];
    invokeCalls += 1;
    return response.promise;
  });

  const firstMount = store.getState().fetchReport("USD");
  const strictModeMount = store.getState().fetchReport("USD");
  assert.equal(invokeCalls, 1);

  const refreshing = refreshDashboardQuotes({
    fetchHoldingQuotes: async () => {},
    fetchReport: store.getState().fetchReport,
    getBaseCurrency: () => "USD",
  });
  await Promise.resolve();
  assert.equal(invokeCalls, 1);

  staleReport.resolve({
    summary: { base_currency: "stale-USD" },
    holdings: [{ id: "stale-holding" }],
  });
  await Promise.all([firstMount, strictModeMount]);
  await Promise.resolve();

  assert.equal(invokeCalls, 2);
  assert.equal(store.getState().summary, null);
  assert.equal(store.getState().loading, true);

  freshReport.resolve({
    summary: { base_currency: "fresh-USD" },
    holdings: [{ id: "fresh-holding" }],
  });
  await refreshing;

  assert.equal(store.getState().summary?.base_currency, "fresh-USD");
  assert.equal(store.getState().holdingDetails[0]?.id, "fresh-holding");
});
