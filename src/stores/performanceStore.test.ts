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

test("only an explicit refresh forces snapshot rebuilding for the selected dates", async () => {
  const backfillCalls = [];
  const invoke = async (command, args) => {
    if (command === "backfill_snapshots") {
      backfillCalls.push(args);
      return 0;
    }
    assert.equal(command, "get_performance_report");
    return report("saved");
  };
  const store = createPerformanceStore(invoke);
  store.getState().setTimeRange("CUSTOM", "2025-02-01", "2025-02-28");

  await store.getState().fetchAll();
  await store.getState().fetchAll(true);

  assert.deepEqual(backfillCalls, [
    { startDate: "2025-02-01", endDate: "2025-02-28", force: false },
    { startDate: "2025-02-01", endDate: "2025-02-28", force: true },
  ]);
});

for (const forceRefresh of [false, true]) {
  test(`a failed ${forceRefresh ? "forced" : "automatic"} backfill preserves the report and exposes the error`, async () => {
    let backfillAttempt = 0;
    let reportAttempt = 0;
    const savedReport = report("saved");
    const invoke = async (command) => {
      if (command === "backfill_snapshots") {
        backfillAttempt += 1;
        if (backfillAttempt > 1) throw new Error("history changed; retry refresh");
        return 0;
      }
      assert.equal(command, "get_performance_report");
      reportAttempt += 1;
      return reportAttempt === 1 ? savedReport : report("outdated");
    };
    const store = createPerformanceStore(invoke);

    await store.getState().fetchAll();
    await store.getState().fetchAll(forceRefresh);

    assert.equal(reportAttempt, 1, "a failed backfill must not query an outdated report");
    assert.equal(store.getState().summary, savedReport.summary);
    assert.equal(store.getState().returnSeries, savedReport.summary.return_series);
    assert.equal(store.getState().drawdown, savedReport.drawdown);
    assert.equal(store.getState().attribution, savedReport.attribution);
    assert.equal(store.getState().monthlyReturns, savedReport.monthly_returns);
    assert.equal(store.getState().holdingPerformances, savedReport.holding_performances);
    assert.equal(store.getState().riskMetrics, savedReport.risk_metrics);
    assert.match(store.getState().error ?? "", /history changed; retry refresh/);
    assert.equal(store.getState().loading, false);
  });
}

for (const { label, changeScope } of [
  { label: "market", changeScope: (state) => state.setMarket("CN") },
  { label: "account", changeScope: (state) => state.setAccountId("account-cn") },
  { label: "preset range", changeScope: (state) => {
    state.setTimeRange("1Y");
    return state.fetchAll();
  } },
  { label: "custom start", changeScope: (state) => {
    state.setTimeRange("CUSTOM", "2025-02-02", "2025-02-28");
    return state.fetchAll();
  } },
  { label: "custom end", changeScope: (state) => {
    state.setTimeRange("CUSTOM", "2025-02-01", "2025-02-27");
    return state.fetchAll();
  } },
]) {
  test(`changing the ${label} clears the previous report while loading and after failure`, async () => {
    const changedBackfill = deferred();
    let backfillAttempt = 0;
    const savedReport = report("saved");
    savedReport.monthly_returns = [{
      year: 2025, month: 2, return_rate: 10, pnl: 10, start_value: 100, end_value: 110,
    }];
    savedReport.holding_performances = [{
      symbol: "AAPL", name: "Apple", market: "US", category_name: "Technology",
      return_rate: 10, pnl: 10, start_value: 100, end_value: 110,
    }];
    const invoke = async (command) => {
      if (command === "backfill_snapshots") {
        backfillAttempt += 1;
        return backfillAttempt === 1 ? 0 : changedBackfill.promise;
      }
      assert.equal(command, "get_performance_report");
      return savedReport;
    };
    const store = createPerformanceStore(invoke);
    store.getState().setTimeRange("CUSTOM", "2025-02-01", "2025-02-28");
    await store.getState().fetchAll();
    assert.equal(store.getState().summary, savedReport.summary);

    const refresh = changeScope(store.getState());
    const pending = store.getState();
    changedBackfill.reject(new Error("new scope unavailable"));
    await refresh;
    const failed = store.getState();

    assert.equal(pending.loading, true);
    assert.equal(pending.error, null);
    assert.equal(failed.loading, false);
    assert.match(failed.error ?? "", /new scope unavailable/);
    for (const state of [pending, failed]) {
      assert.equal(state.summary, null);
      assert.deepEqual(state.returnSeries, []);
      assert.equal(state.drawdown, null);
      assert.equal(state.attribution, null);
      assert.deepEqual(state.monthlyReturns, []);
      assert.deepEqual(state.holdingPerformances, []);
      assert.equal(state.riskMetrics, null);
    }
  });
}

for (const latestReportFinishesFirst of [false, true]) {
  test(`a stale backfill failure cannot override the latest ${latestReportFinishesFirst ? "completed" : "pending"} request`, async () => {
    const oldBackfill = deferred();
    const latestReport = deferred();
    const latestReportStarted = deferred();
    let backfillAttempt = 0;
    let reportAttempt = 0;
    const invoke = async (command) => {
      if (command === "backfill_snapshots") {
        backfillAttempt += 1;
        return backfillAttempt === 1 ? oldBackfill.promise : 0;
      }
      assert.equal(command, "get_performance_report");
      reportAttempt += 1;
      latestReportStarted.resolve();
      return latestReport.promise;
    };
    const store = createPerformanceStore(invoke);
    const oldFetch = store.getState().fetchAll();
    const newFetch = store.getState().fetchAll(true);
    await latestReportStarted.promise;

    if (latestReportFinishesFirst) {
      latestReport.resolve(report("new"));
      await newFetch;
    }
    oldBackfill.reject(new Error("stale backfill failed"));
    await oldFetch;

    assert.equal(store.getState().error, null);
    assert.equal(store.getState().loading, !latestReportFinishesFirst);
    if (!latestReportFinishesFirst) {
      latestReport.resolve(report("new"));
      await newFetch;
    }
    assert.equal(reportAttempt, 1);
    assert.equal(store.getState().summary?.start_date, "new");
    assert.equal(store.getState().loading, false);
  });
}

test("market and account selections refresh with the newly written filter", async () => {
  const reportCalls = [];
  const invoke = async (command, args) => {
    if (command === "backfill_snapshots") return 0;
    assert.equal(command, "get_performance_report");
    reportCalls.push(args);
    return report(`result-${reportCalls.length}`);
  };
  const store = createPerformanceStore(invoke);

  await store.getState().setMarket("HK");
  assert.equal(reportCalls[0].market, "HK");
  assert.equal(reportCalls[0].accountId, undefined);

  await store.getState().setAccountId("account-1");
  assert.equal(reportCalls[1].accountId, "account-1");
  assert.equal(reportCalls[1].market, undefined);
});

test("the latest request for one benchmark wins when responses arrive out of order", async () => {
  const oldResponse = deferred();
  const newResponse = deferred();
  let benchmarkAttempt = 0;
  const invoke = async (command) => {
    assert.equal(command, "get_benchmark_return_series");
    benchmarkAttempt += 1;
    return benchmarkAttempt === 1 ? oldResponse.promise : newResponse.promise;
  };
  const store = createPerformanceStore(invoke);

  const oldFetch = store.getState().fetchBenchmark("^GSPC");
  const newFetch = store.getState().fetchBenchmark("^GSPC");

  newResponse.resolve([{ date: "new", cumulative_return: 2 }]);
  await newFetch;
  oldResponse.resolve([{ date: "old", cumulative_return: 1 }]);
  await oldFetch;

  assert.equal(store.getState().benchmarkSeries["^GSPC"]?.[0]?.date, "new");
});

test("changing the time range clears benchmark series immediately", async () => {
  const invoke = async (command) => {
    assert.equal(command, "get_benchmark_return_series");
    return [{ date: "saved", cumulative_return: 1 }];
  };
  const store = createPerformanceStore(invoke);

  await store.getState().fetchBenchmark("^GSPC");
  store.getState().setTimeRange("CUSTOM", "2025-02-01", "2025-02-28");

  assert.deepEqual(store.getState().benchmarkSeries, {});
});

test("changing the time range invalidates an in-flight benchmark response", async () => {
  const oldResponse = deferred();
  const invoke = async (command) => {
    assert.equal(command, "get_benchmark_return_series");
    return oldResponse.promise;
  };
  const store = createPerformanceStore(invoke);

  const oldFetch = store.getState().fetchBenchmark("^GSPC");
  store.getState().setTimeRange("CUSTOM", "2025-02-01", "2025-02-28");
  oldResponse.resolve([{ date: "old", cumulative_return: 1 }]);
  await oldFetch;

  assert.equal(store.getState().benchmarkSeries["^GSPC"], undefined);
});

test("different benchmark symbols can finish independently", async () => {
  const sp500Response = deferred();
  const nasdaqResponse = deferred();
  const invoke = async (command, args) => {
    assert.equal(command, "get_benchmark_return_series");
    return args.symbol === "^GSPC" ? sp500Response.promise : nasdaqResponse.promise;
  };
  const store = createPerformanceStore(invoke);

  const sp500Fetch = store.getState().fetchBenchmark("^GSPC");
  const nasdaqFetch = store.getState().fetchBenchmark("^IXIC");

  nasdaqResponse.resolve([{ date: "nasdaq", cumulative_return: 2 }]);
  await nasdaqFetch;
  sp500Response.resolve([{ date: "sp500", cumulative_return: 1 }]);
  await sp500Fetch;

  assert.equal(store.getState().benchmarkSeries["^GSPC"]?.[0]?.date, "sp500");
  assert.equal(store.getState().benchmarkSeries["^IXIC"]?.[0]?.date, "nasdaq");
});
