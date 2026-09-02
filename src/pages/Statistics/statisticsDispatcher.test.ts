// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { createStatisticsStore } from "../../stores/statisticsStore.ts";
import { resolveAccountHoldingsCoverage } from "./statisticsAccountHoldings.ts";
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
    getAccountHoldings: options.getAccountHoldings ?? (() => ({
      status: "known-empty",
    })),
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
      getAccountHoldings: () => ({
        status: "known-with-symbols",
        holdings: [
          { symbol: "AAPL", market: "US" },
          { symbol: "AAPL", market: "US" },
          { symbol: "MSFT", market: "US" },
        ],
      }),
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
        getAccountHoldings: (accountId) => {
          const holdings = accountSymbols[accountId] ?? [];
          return holdings.length > 0
            ? { status: "known-with-symbols", holdings }
            : { status: "known-empty" };
        },
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
    getAccountHoldings: () => ({ status: "known-empty" }),
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

test("unknown target account holdings fall back to a full quote refresh before only the active account is reloaded", async () => {
  const firstQuoteRefresh = deferred();
  const accountResponses = [deferred(), deferred()];
  const invokedViews = [];
  let accountCalls = 0;
  const store = createStatisticsStore(async (command, args) => {
    invokedViews.push([command, args]);
    const response = accountResponses[accountCalls];
    accountCalls += 1;
    return response.promise;
  });
  store.setState({
    accountStats: {
      "acct-a": {
        holdings: [
          { account_id: "acct-a", symbol: "AAPL", market: "US" },
        ],
      },
    },
    resultRevisionByView: { "account:acct-a": 1 },
  });
  const quoteRequests = [];
  let selection = {
    ...initialSelection,
    activeTab: "account",
    selectedAccountId: "acct-a",
  };
  const dispatcher = createStatisticsDispatcher({
    getSelection: () => selection,
    updateSelection: (next) => {
      selection = next;
    },
    fetchView: store.getState().fetchView,
    fetchHoldingQuotes: async (symbols) => {
      quoteRequests.push(symbols);
      if (quoteRequests.length === 1) await firstQuoteRefresh.promise;
    },
    getAccountHoldings: (accountId, currency) =>
      resolveAccountHoldingsCoverage(store.getState(), accountId, currency),
  });

  const refreshing = dispatcher.refresh();
  const switching = dispatcher.changeAccount("acct-b");
  firstQuoteRefresh.resolve();
  await Promise.resolve();
  await Promise.resolve();
  accountResponses[0].resolve({ holdings: [], marker: "stale" });
  await switching;
  await Promise.resolve();

  assert.equal(accountCalls, 2);
  accountResponses[1].resolve({ holdings: [], marker: "fresh" });
  await refreshing;

  assert.deepEqual(quoteRequests, [
    [["AAPL", "US"]],
    undefined,
  ]);
  assert.equal(quoteRequests.some((symbols) => symbols?.length === 0), false);
  assert.deepEqual(invokedViews, [
    ["get_statistics_by_account", { accountId: "acct-b" }],
    ["get_statistics_by_account", { accountId: "acct-b" }],
  ]);
  assert.equal(store.getState().accountStats["acct-b"].marker, "fresh");
});

test("real store and page adapter refresh from a newer overview instead of stale account holdings", async () => {
  const staleAccount = deferred();
  const newerOverview = deferred();
  const invokedViews = [];
  let accountCalls = 0;
  const store = createStatisticsStore(async (command, args) => {
    invokedViews.push([command, args]);
    if (command === "get_statistics_overview") {
      return newerOverview.promise;
    }
    accountCalls += 1;
    if (accountCalls === 1) return staleAccount.promise;
    return {
      holdings: [
        { account_id: "acct-a", symbol: "AAPL", market: "US" },
        { account_id: "acct-a", symbol: "MSFT", market: "US" },
      ],
      marker: "fresh-account",
    };
  });

  const accountLoad = store
    .getState()
    .fetchView({ kind: "account", accountId: "acct-a" });
  const overviewLoad = store
    .getState()
    .fetchView({ kind: "overview", baseCurrency: "USD" });
  staleAccount.resolve({
    holdings: [
      { account_id: "acct-a", symbol: "AAPL", market: "US" },
    ],
    marker: "stale-account",
  });
  await accountLoad;
  newerOverview.resolve({
    holdings: [
      { account_id: "acct-a", symbol: "AAPL", market: "US" },
      { account_id: "acct-a", symbol: "MSFT", market: "US" },
    ],
    marker: "newer-overview",
  });
  await overviewLoad;

  const quoteRequests = [];
  let selection = {
    ...initialSelection,
    activeTab: "account",
    selectedAccountId: "acct-a",
  };
  const dispatcher = createStatisticsDispatcher({
    getSelection: () => selection,
    updateSelection: (next) => {
      selection = next;
    },
    fetchView: store.getState().fetchView,
    fetchHoldingQuotes: async (symbols) => {
      quoteRequests.push(symbols);
    },
    getAccountHoldings: (accountId, currency) =>
      resolveAccountHoldingsCoverage(store.getState(), accountId, currency),
  });

  const setupViewCount = invokedViews.length;
  await dispatcher.refresh();

  assert.deepEqual(quoteRequests, [[
    ["AAPL", "US"],
    ["MSFT", "US"],
  ]]);
  assert.deepEqual(invokedViews.slice(setupViewCount), [
    ["get_statistics_by_account", { accountId: "acct-a" }],
  ]);
  assert.equal(store.getState().accountStats["acct-a"].marker, "fresh-account");
});

test("real store and page adapter retain account holdings when that result is newer", async () => {
  const olderOverview = deferred();
  const newerAccount = deferred();
  let accountCalls = 0;
  const store = createStatisticsStore(async (command) => {
    if (command === "get_statistics_overview") {
      return olderOverview.promise;
    }
    accountCalls += 1;
    if (accountCalls === 1) return newerAccount.promise;
    return { holdings: [], marker: "post-refresh" };
  });

  const overviewLoad = store
    .getState()
    .fetchView({ kind: "overview", baseCurrency: "USD" });
  const accountLoad = store
    .getState()
    .fetchView({ kind: "account", accountId: "acct-a" });
  olderOverview.resolve({
    holdings: [
      { account_id: "acct-a", symbol: "AAPL", market: "US" },
    ],
  });
  await overviewLoad;
  newerAccount.resolve({
    holdings: [
      { account_id: "acct-a", symbol: "TSLA", market: "US" },
    ],
  });
  await accountLoad;

  const quoteRequests = [];
  let selection = {
    ...initialSelection,
    activeTab: "account",
    selectedAccountId: "acct-a",
  };
  const dispatcher = createStatisticsDispatcher({
    getSelection: () => selection,
    updateSelection: (next) => {
      selection = next;
    },
    fetchView: store.getState().fetchView,
    fetchHoldingQuotes: async (symbols) => {
      quoteRequests.push(symbols);
    },
    getAccountHoldings: (accountId, currency) =>
      resolveAccountHoldingsCoverage(store.getState(), accountId, currency),
  });

  await dispatcher.refresh();

  assert.deepEqual(quoteRequests, [[
    ["TSLA", "US"],
  ]]);
  assert.equal(accountCalls, 2);
});

test("known-empty account holdings finish without quote provider calls or a coverage loop", async () => {
  const requestedViews = [];
  const quoteRequests = [];
  let selection = {
    ...initialSelection,
    activeTab: "account",
    selectedAccountId: "acct-empty",
  };
  const dispatcher = createStatisticsDispatcher({
    getSelection: () => selection,
    updateSelection: (next) => {
      selection = next;
    },
    fetchView: async (view, mode) => {
      requestedViews.push([view, mode]);
    },
    fetchHoldingQuotes: async (symbols) => {
      quoteRequests.push(symbols);
    },
    getAccountHoldings: () => ({ status: "known-empty" }),
  });

  await dispatcher.refresh();

  assert.deepEqual(quoteRequests, []);
  assert.deepEqual(requestedViews, [
    [
      { kind: "account", accountId: "acct-empty" },
      "reload-after-in-flight",
    ],
  ]);
});
