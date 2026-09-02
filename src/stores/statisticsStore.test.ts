// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import {
  createStatisticsStore,
  statisticsViewKey,
} from "./statisticsStore.ts";

const views = [
  { kind: "overview", baseCurrency: "USD" },
  { kind: "market", market: "US" },
  { kind: "account", accountId: "acct-us" },
  { kind: "category", categoryId: "growth", baseCurrency: "CNY" },
];

const responses = {
  get_statistics_overview: { result: "overview" },
  get_statistics_by_market: { result: "market" },
  get_statistics_by_account: { result: "account" },
  get_statistics_by_category: { result: "category" },
};

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

test("each statistics view invokes exactly one matching command", async () => {
  const calls = [];
  const store = createStatisticsStore(async (command, args) => {
    calls.push([command, args]);
    return responses[command];
  });

  for (const view of views) {
    await store.getState().fetchView(view);
  }

  assert.deepEqual(calls, [
    ["get_statistics_overview", { baseCurrency: "USD" }],
    ["get_statistics_by_market", { market: "US" }],
    ["get_statistics_by_account", { accountId: "acct-us" }],
    [
      "get_statistics_by_category",
      { categoryId: "growth", baseCurrency: "CNY" },
    ],
  ]);
});

test("a failed view preserves every cached result and records only its error", async () => {
  let failedCommand = null;
  const store = createStatisticsStore(async (command) => {
    if (command === failedCommand) throw new Error("offline");
    return responses[command];
  });
  for (const view of views) {
    await store.getState().fetchView(view);
  }
  const before = store.getState();

  failedCommand = "get_statistics_by_category";
  await store.getState().fetchView(views[3]);

  const after = store.getState();
  assert.equal(after.overviewByCurrency.USD, before.overviewByCurrency.USD);
  assert.equal(after.marketStats.US, before.marketStats.US);
  assert.equal(after.accountStats["acct-us"], before.accountStats["acct-us"]);
  assert.equal(after.categoryStats["growth:CNY"], before.categoryStats["growth:CNY"]);
  assert.match(after.errorByView[statisticsViewKey(views[3])] ?? "", /offline/);
  assert.equal(after.errorByView[statisticsViewKey(views[0])] ?? null, null);
  assert.equal(after.errorByView[statisticsViewKey(views[1])] ?? null, null);
  assert.equal(after.errorByView[statisticsViewKey(views[2])] ?? null, null);
});

test("same statistics view key shares one in-flight command", async () => {
  const pending = deferred();
  let calls = 0;
  const store = createStatisticsStore(async () => {
    calls += 1;
    return pending.promise;
  });
  const view = { kind: "overview", baseCurrency: "USD" };

  const first = store.getState().fetchView(view);
  const second = store.getState().fetchView(view);
  pending.resolve({ currency: "USD" });
  await Promise.all([first, second]);

  assert.equal(calls, 1);
  assert.equal(store.getState().loadingByView[statisticsViewKey(view)], false);

  await store.getState().fetchView(view);
  assert.equal(calls, 2);
});

test("reload-after-in-flight starts a fresh statistics generation after a stale failure", async () => {
  const firstResponse = deferred();
  const secondResponse = deferred();
  const responses = [firstResponse, secondResponse];
  let calls = 0;
  const store = createStatisticsStore(async () => {
    const response = responses[calls];
    calls += 1;
    return response.promise;
  });
  const view = { kind: "overview", baseCurrency: "USD" };
  const viewKey = statisticsViewKey(view);

  const initial = store.getState().fetchView(view);
  const refreshed = store
    .getState()
    .fetchView(view, "reload-after-in-flight");

  assert.equal(calls, 1);
  firstResponse.reject(new Error("stale offline"));
  await initial;
  await Promise.resolve();

  assert.equal(calls, 2);
  assert.equal(store.getState().loadingByView[viewKey], true);
  assert.equal(store.getState().errorByView[viewKey], null);

  secondResponse.resolve({ currency: "fresh-USD" });
  await refreshed;

  assert.deepEqual(store.getState().overviewByCurrency.USD, {
    currency: "fresh-USD",
  });
  assert.equal(store.getState().loadingByView[viewKey], false);
  assert.equal(store.getState().errorByView[viewKey], null);
});

test("a statistics reload requested after the queued generation starts schedules another generation", async () => {
  const responses = [deferred(), deferred(), deferred()];
  let calls = 0;
  const store = createStatisticsStore(async () => {
    const response = responses[calls];
    calls += 1;
    return response.promise;
  });
  const view = { kind: "overview", baseCurrency: "USD" };

  const initial = store.getState().fetchView(view);
  const firstRefresh = store
    .getState()
    .fetchView(view, "reload-after-in-flight");
  responses[0].resolve({ currency: "initial" });
  await initial;
  await Promise.resolve();
  assert.equal(calls, 2);

  const secondRefresh = store
    .getState()
    .fetchView(view, "reload-after-in-flight");
  responses[1].resolve({ currency: "superseded-refresh" });
  await firstRefresh;
  await Promise.resolve();

  assert.equal(calls, 3);
  assert.equal(store.getState().overviewByCurrency.USD, undefined);
  responses[2].resolve({ currency: "latest-refresh" });
  await secondRefresh;
  assert.deepEqual(store.getState().overviewByCurrency.USD, {
    currency: "latest-refresh",
  });
});

test("out-of-order overviews remain cached under their requested currencies", async () => {
  const requests = new Map([
    ["USD", deferred()],
    ["CNY", deferred()],
  ]);
  const store = createStatisticsStore(async (_command, args) =>
    requests.get(args.baseCurrency).promise,
  );

  const usd = store
    .getState()
    .fetchView({ kind: "overview", baseCurrency: "USD" });
  const cny = store
    .getState()
    .fetchView({ kind: "overview", baseCurrency: "CNY" });
  requests.get("CNY").resolve({ currency: "CNY" });
  await cny;
  requests.get("USD").resolve({ currency: "USD" });
  await usd;

  assert.deepEqual(store.getState().overviewByCurrency, {
    USD: { currency: "USD" },
    CNY: { currency: "CNY" },
  });
});

test("an overview failure is isolated from another currency success", async () => {
  const requests = new Map([
    ["USD", deferred()],
    ["CNY", deferred()],
  ]);
  const store = createStatisticsStore(async (_command, args) =>
    requests.get(args.baseCurrency).promise,
  );
  const usdView = { kind: "overview", baseCurrency: "USD" };
  const cnyView = { kind: "overview", baseCurrency: "CNY" };

  const usd = store.getState().fetchView(usdView);
  const cny = store.getState().fetchView(cnyView);
  requests.get("CNY").resolve({ currency: "CNY" });
  await cny;
  requests.get("USD").reject(new Error("USD offline"));
  await usd;

  assert.deepEqual(store.getState().overviewByCurrency, {
    CNY: { currency: "CNY" },
  });
  assert.match(
    store.getState().errorByView[statisticsViewKey(usdView)] ?? "",
    /USD offline/,
  );
  assert.equal(
    store.getState().errorByView[statisticsViewKey(cnyView)] ?? null,
    null,
  );
});
