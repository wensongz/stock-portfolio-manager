// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { createQuoteStore } from "./quoteStore.ts";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function outcome(id, refreshedAt, warning = null) {
  return {
    data: [{ id, symbol: "ACME" }],
    warning,
    refreshedAt,
  };
}

test("cache-only quote outcomes preserve the backend refresh timestamp", async () => {
  const persisted = "2026-09-01T10:00:00Z";
  const store = createQuoteStore(
    async () => outcome("cached", persisted),
    async () => () => {},
  );

  await store.getState().fetchHoldingQuotes([]);

  assert.equal(store.getState().lastUpdatedAt, persisted);
  assert.equal(store.getState().holdingQuotes[0].id, "cached");
});

test("background refresh payload is applied directly without another fetch", async () => {
  let handler;
  let invokes = 0;
  const store = createQuoteStore(
    async () => {
      invokes += 1;
      return outcome("initial", "2026-09-01T10:00:00Z");
    },
    async (_event, callback) => {
      handler = callback;
      return () => {};
    },
  );

  const stop = store.getState().startQuoteSync();
  await Promise.resolve();
  await Promise.resolve();
  handler({ payload: outcome("background", "2026-09-02T10:00:00Z", "fallback") });

  assert.equal(invokes, 1);
  assert.equal(store.getState().holdingQuotes[0].id, "background");
  assert.equal(store.getState().quoteWarning, "fallback");
  stop();
});

test("an older quote request cannot overwrite the newest outcome", async () => {
  const first = deferred();
  const second = deferred();
  const calls = [first, second];
  let index = 0;
  const store = createQuoteStore(
    () => calls[index++].promise,
    async () => () => {},
  );

  const oldRequest = store.getState().fetchHoldingQuotes();
  const newRequest = store.getState().fetchHoldingQuotes([["ACME", "US"]]);
  second.resolve(outcome("new", "2026-09-02T10:00:00Z"));
  await newRequest;
  first.resolve(outcome("old", "2026-09-01T10:00:00Z"));
  await oldRequest;

  assert.equal(store.getState().holdingQuotes[0].id, "new");
  assert.equal(store.getState().loading, false);
});
