// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import {
  createPortfolioAlertStore,
  portfolioAlertScopeKey,
  selectCurrentPortfolioAlertView,
} from "./portfolioAlertStore.ts";

function overallScope() {
  return { kind: "OVERALL", market: null, accountId: null };
}

function marketScope(market) {
  return { kind: "MARKET", market, accountId: null };
}

function accountScope(accountId) {
  return { kind: "ACCOUNT", market: null, accountId };
}

function breach(breachKey = "category:growth", firstTriggeredAt = "2026-09-06T00:00:00Z") {
  return {
    configId: "config-us",
    breachKey,
    breachKind: "CATEGORY_DEVIATION",
    direction: "OVERWEIGHT",
    firstTriggeredAt,
    lastSeenAt: firstTriggeredAt,
  };
}

function evaluation(overrides = {}) {
  return {
    status: "READY",
    snapshot: {
      configId: "config-us",
      scope: marketScope("US"),
      baseCurrency: "USD",
      evaluatedAt: "2026-09-06T00:00:00Z",
      totalMarketValue: 100,
      categories: [{
        categoryId: "growth",
        categoryName: "Growth",
        categoryColor: "#00AA00",
        categoryIcon: "rocket",
        targetPercent: 60,
        currentPercent: 70,
        relativeDeviationPercent: 16.67,
        currentMarketValue: 70,
        targetMarketValue: 60,
        rebalanceAmount: -10,
        direction: "OVERWEIGHT",
      }],
      concentrations: [{
        market: "US",
        symbol: "AAPL",
        normalizedSymbol: "AAPL",
        name: "Apple",
        categoryId: "growth",
        marketValue: 70,
        positionPercent: 70,
        thresholdPercent: 60,
      }],
    },
    stale: false,
    missingData: [],
    activeBreaches: [],
    newlyTriggered: [],
    ...overrides,
  };
}

function viewFor(market, overrides = {}) {
  const scope = market === "overall" ? overallScope() : marketScope(market);
  const configId = `config-${market}`;
  return {
    config: {
      id: configId,
      scope,
      baseCurrency: market === "CN" ? "CNY" : "USD",
      deviationThreshold: 10,
      concentrationThreshold: 60,
      isActive: true,
      targets: [{ categoryId: "growth", targetPercent: 100 }],
      lastSnapshot: null,
      lastEvaluatedAt: null,
    },
    evaluation: evaluation({
      snapshot: {
        ...evaluation().snapshot,
        configId,
        scope,
        baseCurrency: market === "CN" ? "CNY" : "USD",
      },
    }),
    ...overrides,
  };
}

function validDraft() {
  return {
    id: null,
    scope: marketScope("US"),
    baseCurrency: "USD",
    deviationThreshold: 10,
    concentrationThreshold: 60,
    isActive: true,
    targets: [{ categoryId: "growth", targetPercent: 100 }],
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

test("scope keys separate overall, market, and account state", () => {
  assert.equal(portfolioAlertScopeKey(overallScope()), "overall");
  assert.equal(portfolioAlertScopeKey(marketScope("US")), "market:US");
  assert.equal(portfolioAlertScopeKey(accountScope("acct-1")), "account:acct-1");
});

test("a stale cross-scope response cannot overwrite the selected scope", async () => {
  const us = deferred();
  const cn = deferred();
  const store = createPortfolioAlertStore((command, { scope }) => {
    assert.equal(command, "get_portfolio_alert_view");
    return scope.market === "US" ? us.promise : cn.promise;
  });

  const first = store.getState().loadScope(marketScope("US"));
  const second = store.getState().loadScope(marketScope("CN"));
  cn.resolve(viewFor("CN"));
  await second;
  us.resolve(viewFor("US"));
  await first;

  assert.equal(store.getState().selectedScopeKey, "market:CN");
  assert.equal(selectCurrentPortfolioAlertView(store.getState())?.config?.scope.market, "CN");
  assert.equal(store.getState().viewsByScope["market:US"]?.config?.scope.market, "US");
});

test("an older same-scope generation cannot overwrite a newer response", async () => {
  const firstResponse = deferred();
  const secondResponse = deferred();
  let calls = 0;
  const store = createPortfolioAlertStore(() => {
    calls += 1;
    return calls === 1 ? firstResponse.promise : secondResponse.promise;
  });

  const first = store.getState().loadScope(marketScope("US"));
  const second = store.getState().loadScope(marketScope("US"));
  secondResponse.resolve(viewFor("US", { config: { ...viewFor("US").config, id: "fresh-us" } }));
  await second;
  firstResponse.resolve(viewFor("US", { config: { ...viewFor("US").config, id: "stale-us" } }));
  await first;

  assert.equal(store.getState().viewsByScope["market:US"]?.config?.id, "fresh-us");
  assert.equal(store.getState().loadingByScope["market:US"], false);
  assert.equal(store.getState().errorsByScope["market:US"], undefined);
});

test("save stores its returned view in the supplied scope without a second evaluation", async () => {
  const calls = [];
  const savedView = viewFor("US", { config: { ...viewFor("US").config, id: "saved-config" } });
  const store = createPortfolioAlertStore(async (command, args) => {
    calls.push({ command, args });
    return savedView;
  });

  await store.getState().saveConfig(validDraft());

  assert.deepEqual(calls.map((call) => call.command), ["save_portfolio_alert_config"]);
  assert.equal(store.getState().selectedScopeKey, "market:US");
  assert.equal(selectCurrentPortfolioAlertView(store.getState()), savedView);
  assert.deepEqual(calls[0].args, { input: validDraft() });
});

test("activation stores its returned view without a second evaluation", async () => {
  const calls = [];
  const inactiveView = viewFor("US", {
    config: { ...viewFor("US").config, isActive: false },
    evaluation: null,
  });
  const store = createPortfolioAlertStore(async (command, args) => {
    calls.push({ command, args });
    return inactiveView;
  });
  store.getState().selectScope(marketScope("US"));

  await store.getState().setActive("config-US", marketScope("US"), false);

  assert.deepEqual(calls.map((call) => call.command), ["set_portfolio_alert_active"]);
  assert.equal(selectCurrentPortfolioAlertView(store.getState()), inactiveView);
  assert.deepEqual(calls[0].args, { configId: "config-US", isActive: false });
});

test("evaluation replaces only the evaluation of its existing scoped view", async () => {
  const initial = viewFor("US", { evaluation: null });
  const evaluated = evaluation({ status: "EMPTY", snapshot: null });
  const store = createPortfolioAlertStore(async (command) =>
    command === "get_portfolio_alert_view" ? initial : evaluated,
  );
  await store.getState().loadScope(marketScope("US"));

  await store.getState().evaluate("config-US", marketScope("US"));

  assert.equal(store.getState().viewsByScope["market:US"]?.config, initial.config);
  assert.equal(store.getState().viewsByScope["market:US"]?.evaluation, evaluated);
});

test("late load, save, and activation responses cannot overwrite a newer evaluation", async () => {
  const pendingLoad = deferred();
  const pendingSave = deferred();
  const pendingActivation = deferred();
  const pendingEvaluation = deferred();
  const store = createPortfolioAlertStore((command) => {
    if (command === "get_portfolio_alert_view") return pendingLoad.promise;
    if (command === "save_portfolio_alert_config") return pendingSave.promise;
    if (command === "set_portfolio_alert_active") return pendingActivation.promise;
    return pendingEvaluation.promise;
  });

  const load = store.getState().loadScope(marketScope("US"));
  const save = store.getState().saveConfig(validDraft());
  const activation = store.getState().setActive("config-US", marketScope("US"), false);
  const evaluate = store.getState().evaluate("config-US", marketScope("US"));
  const newestEvaluation = evaluation({ status: "INVALID_CONFIG", snapshot: null });
  pendingEvaluation.resolve(newestEvaluation);
  await evaluate;
  pendingActivation.resolve(viewFor("US", { evaluation: null }));
  pendingSave.resolve(viewFor("US"));
  pendingLoad.resolve(viewFor("US"));
  await Promise.all([load, save, activation]);

  assert.equal(store.getState().viewsByScope["market:US"]?.evaluation, newestEvaluation);
  assert.equal(store.getState().loadingByScope["market:US"], false);
  assert.equal(store.getState().errorsByScope["market:US"], undefined);
});

test("loading and errors remain isolated by scope", async () => {
  const us = deferred();
  const cn = deferred();
  const store = createPortfolioAlertStore((_command, { scope }) =>
    scope.market === "US" ? us.promise : cn.promise,
  );

  const usLoad = store.getState().loadScope(marketScope("US"));
  const cnLoad = store.getState().loadScope(marketScope("CN"));
  us.reject(new Error("US unavailable"));
  await usLoad;

  assert.equal(store.getState().loadingByScope["market:US"], false);
  assert.match(store.getState().errorsByScope["market:US"] ?? "", /US unavailable/);
  assert.equal(store.getState().loadingByScope["market:CN"], true);
  assert.equal(store.getState().errorsByScope["market:CN"], undefined);
  cn.resolve(viewFor("CN"));
  await cnLoad;
});

test("newly triggered breaches are queued once across command and event ingestion", async () => {
  const triggered = breach();
  const loaded = viewFor("overall", {
    config: { ...viewFor("overall").config, id: "config-us" },
    evaluation: evaluation({ newlyTriggered: [triggered] }),
  });
  const store = createPortfolioAlertStore(async () => loaded);

  await store.getState().loadScope(overallScope());
  store.getState().ingestNotification({
    configId: "config-us",
    scope: overallScope(),
    breach: triggered,
    message: "portfolio alert",
    triggeredAt: triggered.firstTriggeredAt,
  });

  assert.deepEqual(store.getState().pendingNotifications.map((item) => item.breachKey), ["category:growth"]);
  const consumed = store.getState().takePendingNotifications();
  assert.deepEqual(consumed, [triggered]);
  assert.deepEqual(store.getState().pendingNotifications, []);
});
