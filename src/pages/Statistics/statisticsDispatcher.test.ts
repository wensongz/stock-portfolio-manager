// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { createStatisticsDispatcher } from "./statisticsDispatcher.ts";

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function createHarness(initialSelection, options = {}) {
  let selection = initialSelection;
  const requestedViews = [];
  const quoteRequests = [];
  const dispatcher = createStatisticsDispatcher({
    getSelection: () => selection,
    updateSelection: (next) => {
      selection = next;
    },
    fetchView: async (view) => {
      requestedViews.push(view);
    },
    fetchHoldingQuotes: options.fetchHoldingQuotes ?? (async (symbols) => {
      quoteRequests.push(symbols);
    }),
    getAccountHoldings: options.getAccountHoldings ?? (() => []),
  });
  return {
    dispatcher,
    requestedViews,
    quoteRequests,
    getSelection: () => selection,
  };
}

const initialSelection = {
  activeTab: "overview",
  baseCurrency: "USD",
  selectedMarket: "US",
  selectedAccountId: "",
  selectedCategoryId: "",
};

test("statistics dispatcher requests only the active view across every UI event", async () => {
  const harness = createHarness(initialSelection);
  const { dispatcher } = harness;

  await dispatcher.initialize();
  await dispatcher.changeMarket("HK");
  await dispatcher.changeAccount("acct-us");
  await dispatcher.changeCategory("growth");
  await dispatcher.changeCurrency("CNY");
  await dispatcher.changeTab("market");
  await dispatcher.changeCurrency("HKD");
  await dispatcher.changeMarket("CN");
  await dispatcher.changeAccount("acct-cn");
  await dispatcher.changeCategory("income");
  await dispatcher.changeTab("account");
  await dispatcher.changeCategory("growth");
  await dispatcher.changeTab("category");
  await dispatcher.changeCurrency("USD");
  await dispatcher.changeTab("overview");

  assert.deepEqual(harness.requestedViews, [
    { kind: "overview", baseCurrency: "USD" },
    { kind: "overview", baseCurrency: "CNY" },
    { kind: "market", market: "HK" },
    { kind: "market", market: "CN" },
    { kind: "account", accountId: "acct-cn" },
    { kind: "category", categoryId: "growth", baseCurrency: "HKD" },
    { kind: "category", categoryId: "growth", baseCurrency: "USD" },
    { kind: "overview", baseCurrency: "USD" },
  ]);
  assert.deepEqual(harness.getSelection(), {
    activeTab: "overview",
    baseCurrency: "USD",
    selectedMarket: "CN",
    selectedAccountId: "acct-cn",
    selectedCategoryId: "growth",
  });
});

test("statistics refresh resolves its view from the latest post-quote selection", async () => {
  const quoteRefresh = deferred();
  const quoteRequests = [];
  const harness = createHarness(
    {
      ...initialSelection,
      activeTab: "account",
      selectedAccountId: "acct-us",
      selectedCategoryId: "growth",
    },
    {
      fetchHoldingQuotes: async (symbols) => {
        quoteRequests.push(symbols);
        await quoteRefresh.promise;
      },
      getAccountHoldings: () => [
        { symbol: "AAPL", market: "US" },
        { symbol: "AAPL", market: "US" },
        { symbol: "MSFT", market: "US" },
      ],
    },
  );

  const refreshing = harness.dispatcher.refresh();
  await harness.dispatcher.changeTab("category");
  await harness.dispatcher.changeCurrency("CNY");
  quoteRefresh.resolve();
  await refreshing;

  assert.deepEqual(quoteRequests, [[
    ["AAPL", "US"],
    ["MSFT", "US"],
  ]]);
  assert.deepEqual(harness.requestedViews, [
    { kind: "category", categoryId: "growth", baseCurrency: "USD" },
    { kind: "category", categoryId: "growth", baseCurrency: "CNY" },
    { kind: "category", categoryId: "growth", baseCurrency: "CNY" },
  ]);
  assert.equal(
    harness.requestedViews.some((view) => view.kind === "account"),
    false,
  );
});
