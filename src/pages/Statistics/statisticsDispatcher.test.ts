// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { createStatisticsStore } from "../../stores/statisticsStore.ts";
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
  const requestModes = [];
  const quoteRequests = [];
  const dispatcher = createStatisticsDispatcher({
    getSelection: () => selection,
    updateSelection: (next) => {
      selection = next;
    },
    fetchView: options.fetchView ?? (async (view, mode) => {
      requestedViews.push(view);
      requestModes.push(mode);
    }),
    fetchHoldingQuotes: options.fetchHoldingQuotes ?? (async (symbols) => {
      quoteRequests.push(symbols);
    }),
    getAccountHoldings: options.getAccountHoldings ?? (() => []),
  });
  return {
    dispatcher,
    requestedViews,
    requestModes,
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
  ], undefined]);
  assert.deepEqual(harness.requestedViews, [
    { kind: "category", categoryId: "growth", baseCurrency: "USD" },
    { kind: "category", categoryId: "growth", baseCurrency: "CNY" },
    { kind: "category", categoryId: "growth", baseCurrency: "CNY" },
  ]);
  assert.equal(
    harness.requestedViews.some((view) => view.kind === "account"),
    false,
  );
  assert.equal(harness.requestModes.at(-1), "reload-after-in-flight");
});

for (const scenario of [
  {
    name: "overview",
    transition: (dispatcher) => dispatcher.changeTab("overview"),
    expectedView: { kind: "overview", baseCurrency: "USD" },
    expectedSupplement: undefined,
  },
  {
    name: "category",
    transition: (dispatcher) => dispatcher.changeTab("category"),
    expectedView: {
      kind: "category",
      categoryId: "growth",
      baseCurrency: "USD",
    },
    expectedSupplement: undefined,
  },
  {
    name: "market",
    transition: (dispatcher) => dispatcher.changeTab("market"),
    expectedView: { kind: "market", market: "US" },
    expectedSupplement: undefined,
  },
  {
    name: "another account",
    transition: (dispatcher) => dispatcher.changeAccount("acct-hk"),
    expectedView: { kind: "account", accountId: "acct-hk" },
    expectedSupplement: [["0700.HK", "HK"]],
  },
]) {
  test(`account quote refresh supplements ${scenario.name} coverage and reloads only that active view`, async () => {
    const firstQuoteRefresh = deferred();
    const accountSymbols = {
      "acct-us": [
        { symbol: "AAPL", market: "US" },
        { symbol: "AAPL", market: "US" },
        { symbol: "MSFT", market: "US" },
      ],
      "acct-hk": [{ symbol: "0700.HK", market: "HK" }],
    };
    const harness = createHarness(
      {
        ...initialSelection,
        activeTab: "account",
        selectedAccountId: "acct-us",
        selectedCategoryId: "growth",
      },
      {
        fetchHoldingQuotes: async (symbols) => {
          harness.quoteRequests.push(symbols);
          if (harness.quoteRequests.length === 1) {
            await firstQuoteRefresh.promise;
          }
        },
        getAccountHoldings: (accountId) => accountSymbols[accountId] ?? [],
      },
    );

    const refreshing = harness.dispatcher.refresh();
    await scenario.transition(harness.dispatcher);
    firstQuoteRefresh.resolve();
    await refreshing;

    assert.deepEqual(harness.quoteRequests, [
      [
        ["AAPL", "US"],
        ["MSFT", "US"],
      ],
      scenario.expectedSupplement,
    ]);
    assert.deepEqual(harness.requestedViews, [
      scenario.expectedView,
      scenario.expectedView,
    ]);
    assert.deepEqual(harness.requestModes, [
      "join-in-flight",
      "reload-after-in-flight",
    ]);
  });
}

test("real statistics store shares StrictMode initialization but post-refresh reload wins after selection changes", async () => {
  const staleOverview = deferred();
  const freshOverview = deferred();
  const overviewResponses = [staleOverview, freshOverview];
  const invokedViews = [];
  let overviewCalls = 0;
  const store = createStatisticsStore(async (command, args) => {
    invokedViews.push([command, args]);
    if (command === "get_statistics_overview") {
      const response = overviewResponses[overviewCalls];
      overviewCalls += 1;
      return response.promise;
    }
    return { market: args.market };
  });
  const quoteRefresh = deferred();
  let selection = { ...initialSelection };
  const dispatcher = createStatisticsDispatcher({
    getSelection: () => selection,
    updateSelection: (next) => {
      selection = next;
    },
    fetchView: store.getState().fetchView,
    fetchHoldingQuotes: () => quoteRefresh.promise,
    getAccountHoldings: () => [],
  });

  const firstMount = dispatcher.initialize();
  const strictModeMount = dispatcher.initialize();
  assert.equal(overviewCalls, 1);

  const refreshing = dispatcher.refresh();
  await dispatcher.changeTab("market");
  const switchedBack = dispatcher.changeTab("overview");
  quoteRefresh.resolve();
  await Promise.resolve();

  staleOverview.resolve({ currency: "stale-USD" });
  await Promise.all([firstMount, strictModeMount, switchedBack]);
  await Promise.resolve();

  assert.equal(overviewCalls, 2);
  assert.equal(
    invokedViews.filter(([command]) => command === "get_statistics_by_market")
      .length,
    1,
  );
  assert.equal(store.getState().overviewByCurrency.USD, undefined);

  freshOverview.resolve({ currency: "fresh-USD" });
  await refreshing;

  assert.deepEqual(store.getState().overviewByCurrency.USD, {
    currency: "fresh-USD",
  });
});
