// @ts-nocheck -- This test runs directly in Node 26.
import test from "node:test";
import assert from "node:assert/strict";
import { startPortfolioAlertEventListener } from "./portfolioAlertEventListener.ts";
import { createPortfolioAlertStore } from "../../stores/portfolioAlertStore.ts";

function overallScope() {
  return { kind: "OVERALL", market: null, accountId: null };
}

function notification() {
  const firstTriggeredAt = "2026-09-06T00:00:00Z";
  return {
    configId: "config-us",
    scope: overallScope(),
    breach: {
      configId: "config-us",
      breachKey: "category:growth",
      breachKind: "CATEGORY_DEVIATION",
      direction: "OVERWEIGHT",
      firstTriggeredAt,
      lastSeenAt: firstTriggeredAt,
    },
    message: "portfolio alert",
    triggeredAt: firstTriggeredAt,
  };
}

test("app-lifetime listener presents an event received while the alerts tab is unmounted exactly once", async () => {
  const store = createPortfolioAlertStore();
  let emit;
  const dispose = await startPortfolioAlertEventListener(
    async (_eventName, handler) => {
      emit = (payload) => handler({ payload });
      return () => {};
    },
    (incoming) => store.getState().ingestNotification(incoming),
  );

  emit(notification());

  assert.deepEqual(
    store.getState().takePendingNotifications().map((breach) => breach.breachKey),
    ["category:growth"],
  );
  assert.deepEqual(store.getState().takePendingNotifications(), []);
  dispose();
});

test("app-lifetime listener does not duplicate a transition already returned by a command", async () => {
  const transition = notification();
  const store = createPortfolioAlertStore(async () => ({
    config: null,
    evaluation: {
      status: "READY",
      snapshot: null,
      stale: false,
      missingData: [],
      activeBreaches: [],
      newlyTriggered: [transition.breach],
    },
  }));
  let emit;
  const dispose = await startPortfolioAlertEventListener(
    async (_eventName, handler) => {
      emit = (payload) => handler({ payload });
      return () => {};
    },
    (incoming) => store.getState().ingestNotification(incoming),
  );

  await store.getState().evaluate("config-us", overallScope());
  emit(transition);

  assert.deepEqual(
    store.getState().takePendingNotifications().map((breach) => breach.breachKey),
    ["category:growth"],
  );
  dispose();
});
