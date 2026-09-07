// @ts-nocheck -- Node runs this file directly; browser-focused TypeScript
// configuration intentionally excludes Node's ambient types.
import test from "node:test";
import assert from "node:assert/strict";
import { createQuarterlyStore } from "./quarterlyStore.ts";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

function detail(id) {
  return { snapshot: { id, quarter: id }, holdings: [] };
}

function transactions(id) {
  return [{ symbol: id, transactions: [] }];
}

test("switching snapshots clears the detail bundle and ignores stale results", async () => {
  const requests = new Map();
  const invoke = (command, args) => {
    const key = `${command}:${args.snapshotId}`;
    const pending = deferred();
    requests.set(key, pending);
    return pending.promise;
  };
  const store = createQuarterlyStore(invoke);

  const loadA = store.getState().fetchDetail("A");
  requests.get("get_quarterly_snapshot_detail:A").resolve(detail("A"));
  requests.get("get_quarterly_transactions:A").resolve(transactions("A"));
  await loadA;
  assert.equal(store.getState().detail.snapshot.id, "A");
  assert.equal(store.getState().quarterlyTransactions[0].symbol, "A");

  const loadAAgain = store.getState().fetchDetail("A");
  const loadB = store.getState().fetchDetail("B");
  assert.equal(store.getState().detail, null);
  assert.deepEqual(store.getState().quarterlyTransactions, []);
  assert.equal(store.getState().detailSnapshotId, "B");

  requests.get("get_quarterly_snapshot_detail:B").resolve(detail("B"));
  requests.get("get_quarterly_transactions:B").resolve(transactions("B"));
  await loadB;
  requests.get("get_quarterly_snapshot_detail:A").resolve(detail("stale-A"));
  requests.get("get_quarterly_transactions:A").resolve(transactions("stale-A"));
  await loadAAgain;

  assert.equal(store.getState().detail.snapshot.id, "B");
  assert.equal(store.getState().quarterlyTransactions[0].symbol, "B");
  assert.equal(store.getState().detailLoading, false);
  assert.equal(store.getState().detailError, null);
});

test("a stale detail failure cannot replace a newer snapshot success", async () => {
  const pendingA = deferred();
  const pendingB = deferred();
  const invoke = (command, args) => {
    if (command === "get_quarterly_transactions") {
      return Promise.resolve(transactions(args.snapshotId));
    }
    return args.snapshotId === "A" ? pendingA.promise : pendingB.promise;
  };
  const store = createQuarterlyStore(invoke);

  const loadA = store.getState().fetchDetail("A");
  const loadB = store.getState().fetchDetail("B");
  pendingB.resolve(detail("B"));
  await loadB;
  pendingA.reject(new Error("stale failure"));
  await loadA;

  assert.equal(store.getState().detail.snapshot.id, "B");
  assert.equal(store.getState().detailError, null);
});

test("the latest ordered comparison pair wins", async () => {
  const oldComparison = deferred();
  const newComparison = deferred();
  let calls = 0;
  const store = createQuarterlyStore(async () => {
    calls += 1;
    return calls === 1 ? oldComparison.promise : newComparison.promise;
  });

  const oldLoad = store.getState().compareQuarters("2025Q1", "2025Q2");
  const newLoad = store.getState().compareQuarters("2025Q2", "2025Q3");
  newComparison.resolve({ quarter1: "2025Q2", quarter2: "2025Q3" });
  await newLoad;
  oldComparison.resolve({ quarter1: "2025Q1", quarter2: "2025Q2" });
  await oldLoad;

  assert.equal(store.getState().comparison.quarter1, "2025Q2");
  assert.equal(store.getState().comparison.quarter2, "2025Q3");
  assert.equal(store.getState().comparisonLoading, false);
});

test("list, detail, comparison, and trend loading states are independent", async () => {
  const requests = new Map();
  const invoke = (command) => {
    const pending = deferred();
    requests.set(command, pending);
    return pending.promise;
  };
  const store = createQuarterlyStore(invoke);

  const list = store.getState().fetchSnapshots();
  const detailLoad = store.getState().fetchDetail("A");
  const comparison = store.getState().compareQuarters("2025Q1", "2025Q2");
  const trends = store.getState().fetchTrends();

  assert.equal(store.getState().listLoading, true);
  assert.equal(store.getState().detailLoading, true);
  assert.equal(store.getState().comparisonLoading, true);
  assert.equal(store.getState().trendsLoading, true);

  requests.get("get_quarterly_snapshots").resolve([]);
  await list;
  assert.equal(store.getState().listLoading, false);
  assert.equal(store.getState().detailLoading, true);
  assert.equal(store.getState().comparisonLoading, true);
  assert.equal(store.getState().trendsLoading, true);

  requests.get("get_quarterly_snapshot_detail").resolve(detail("A"));
  requests.get("get_quarterly_transactions").resolve(transactions("A"));
  requests.get("compare_quarters").resolve({ quarter1: "2025Q1", quarter2: "2025Q2" });
  requests.get("get_quarterly_trends").resolve({ quarters: [] });
  await Promise.all([detailLoad, comparison, trends]);
});

test("holding notes save targets one snapshot row and leaves the same security in another account unchanged", async () => {
  const calls = [];
  const store = createQuarterlyStore(async (command, args) => { calls.push({ command, args }); return true; });
  store.setState({ detailSnapshotId: "quarter-a", detail: { snapshot: { id: "quarter-a" }, holdings: [
    { id: "row-a", symbol: "$CASH-USD", account_id: "account-a", notes: "A" },
    { id: "row-b", symbol: "$CASH-USD", account_id: "account-b", notes: "B" },
  ] } });
  await store.getState().updateHoldingNotes("quarter-a", "row-a", "Updated A");
  assert.deepEqual(calls, [{ command: "update_holding_notes", args: { snapshotId: "quarter-a", holdingSnapshotId: "row-a", notes: "Updated A" } }]);
  assert.deepEqual(store.getState().detail.holdings.map(row => row.notes), ["Updated A", "B"]);
});

test("a rejected or false notes save propagates failure and preserves the cached notes", async () => {
  for (const outcome of [false, new Error("write failed")]) {
    const store = createQuarterlyStore(async () => { if (outcome instanceof Error) throw outcome; return outcome; });
    store.setState({ detailSnapshotId: "quarter-a", detail: { snapshot: { id: "quarter-a" }, holdings: [{ id: "row-a", notes: "Original" }] } });
    await assert.rejects(store.getState().updateHoldingNotes("quarter-a", "row-a", "Draft"));
    assert.equal(store.getState().detail.holdings[0].notes, "Original");
    assert.equal(store.getState().mutationLoading, false);
    assert.ok(store.getState().mutationError);
  }
});

test("a late notes save never replaces the newly selected quarter", async () => {
  const pending = deferred();
  const store = createQuarterlyStore(() => pending.promise);
  const saving = store.getState().updateHoldingNotes("quarter-a", "row-a", "A");
  const next = { snapshot: { id: "quarter-b" }, holdings: [{ id: "row-b", notes: "B" }] };
  store.setState({ detailSnapshotId: "quarter-b", detail: next });
  pending.resolve(true);
  await saving;
  assert.equal(store.getState().detail, next);
});
