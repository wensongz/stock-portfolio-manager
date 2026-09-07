// @ts-nocheck -- Run directly with Node's TypeScript support.
import test from "node:test";
import assert from "node:assert/strict";
import { createHoldingStore } from "../../stores/holdingStore.ts";

test("cash correction sends the revision and metadata atomically and immediately replaces the holding", async () => {
  const calls = [];
  const old = { id: "cash-test", shares: 50, name: "Cash" };
  const updated = { ...old, shares: -25, name: "Corrected", category_id: "cash-category" };
  const store = createHoldingStore(async (command, args) => { calls.push({ command, args }); return updated; });
  store.setState({ holdings: [old, { id: "other", shares: 3 }] });
  const payload = { id: old.id, balance: -25, expectedRevision: 7, name: "Corrected", categoryId: "cash-category" };
  assert.equal(await store.getState().correctCashBalance(payload), updated);
  assert.deepEqual(calls, [{ command: "correct_cash_balance", args: payload }]);
  assert.deepEqual(store.getState().holdings, [updated, { id: "other", shares: 3 }]);
});

test("failed cash correction retains the prior holding and rejects for the modal to retain its draft", async () => {
  const old = { id: "cash-test", shares: 50 };
  const store = createHoldingStore(async () => { throw new Error("revision changed"); });
  store.setState({ holdings: [old] });
  await assert.rejects(store.getState().correctCashBalance({ id: old.id, balance: 0, expectedRevision: 1, name: "Cash" }), /revision changed/);
  assert.deepEqual(store.getState().holdings, [old]);
});

test("a holdings read started before correction cannot restore the old cash balance", async () => {
  let resolveRead;
  const old = { id: "cash-test", shares: 50 };
  const updated = { ...old, shares: -25 };
  const store = createHoldingStore(async (command) => command === "get_holdings"
    ? new Promise(resolve => { resolveRead = resolve; }) : updated);
  store.setState({ holdings: [old] });
  const pending = store.getState().fetchHoldings();
  await store.getState().correctCashBalance({ id: old.id, balance: -25, expectedRevision: 2, name: "Cash" });
  resolveRead([old]);
  await pending;
  assert.deepEqual(store.getState().holdings, [updated]);
  assert.equal(store.getState().loading, false);
});
