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
  assert.equal(after.overview, before.overview);
  assert.equal(after.marketStats.US, before.marketStats.US);
  assert.equal(after.accountStats["acct-us"], before.accountStats["acct-us"]);
  assert.equal(after.categoryStats["growth:CNY"], before.categoryStats["growth:CNY"]);
  assert.match(after.errorByView[statisticsViewKey(views[3])] ?? "", /offline/);
  assert.equal(after.errorByView[statisticsViewKey(views[0])] ?? null, null);
  assert.equal(after.errorByView[statisticsViewKey(views[1])] ?? null, null);
  assert.equal(after.errorByView[statisticsViewKey(views[2])] ?? null, null);
});
