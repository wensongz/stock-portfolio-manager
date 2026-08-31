// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import { createPerformanceStore } from "./performanceStore.ts";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function report(label: string) {
  return {
    summary: {
      start_date: label,
      end_date: label,
      start_value: 100,
      end_value: 110,
      total_return: 10,
      annualized_return: 10,
      total_pnl: 10,
      max_drawdown: -1,
      volatility: 2,
      sharpe_ratio: 1,
      return_series: [{
        date: label,
        cumulative_return: 10,
        daily_return: 10,
        portfolio_value: 110,
        daily_pnl: 10,
      }],
    },
    drawdown: {
      max_drawdown: -1,
      peak_date: label,
      trough_date: label,
      recovery_date: null,
      drawdown_duration: 0,
      recovery_duration: null,
      drawdown_series: [],
    },
    attribution: { total_pnl: 10, by_market: [], by_category: [], by_holding: [] },
    monthly_returns: [],
    holding_performances: [],
    risk_metrics: {
      daily_volatility: 1,
      annualized_volatility: 2,
      sharpe_ratio: 1,
      risk_free_rate: 4.5,
      max_drawdown: -1,
      calmar_ratio: 1,
    },
  };
}

test("a stale performance response cannot overwrite the newest filters", async () => {
  const oldResponse = deferred();
  const newResponse = deferred();
  const reportCalls = [];
  const invoke = async (command, args) => {
    if (command === "backfill_snapshots") return 0;
    assert.equal(command, "get_performance_report");
    reportCalls.push(args);
    return reportCalls.length === 1 ? oldResponse.promise : newResponse.promise;
  };
  const store = createPerformanceStore(invoke);

  const oldFetch = store.getState().fetchAll();
  await Promise.resolve();
  await Promise.resolve();
  store.getState().setTimeRange("CUSTOM", "2025-02-01", "2025-02-28");
  const newFetch = store.getState().fetchAll();
  await Promise.resolve();
  await Promise.resolve();

  newResponse.resolve(report("new"));
  await newFetch;
  oldResponse.resolve(report("old"));
  await oldFetch;

  assert.equal(store.getState().summary?.start_date, "new");
  assert.equal(store.getState().returnSeries[0]?.date, "new");
  assert.equal(store.getState().loading, false);
  assert.equal(store.getState().error, null);
  assert.equal(reportCalls.length, 2);
  assert.equal(reportCalls[1].startDate, "2025-02-01");
  assert.equal(reportCalls[1].rankingLimit, 10_000);
});

test("a failed refresh preserves the last successful report", async () => {
  let reportAttempt = 0;
  const invoke = async (command) => {
    if (command === "backfill_snapshots") return 0;
    assert.equal(command, "get_performance_report");
    reportAttempt += 1;
    if (reportAttempt === 1) return report("saved");
    throw new Error("refresh failed");
  };
  const store = createPerformanceStore(invoke);

  await store.getState().fetchAll();
  await store.getState().fetchAll(true);

  assert.equal(store.getState().summary?.start_date, "saved");
  assert.equal(store.getState().returnSeries[0]?.date, "saved");
  assert.match(store.getState().error ?? "", /refresh failed/);
  assert.equal(store.getState().loading, false);
});
