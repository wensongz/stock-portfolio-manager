// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
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
