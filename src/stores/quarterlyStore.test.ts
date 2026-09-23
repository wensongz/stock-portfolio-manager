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

test("initializing quarterly analysis fills gaps once and preserves existing snapshots", async () => {
  const existing = { id: "existing", quarter: "2025-Q2", overall_notes: "Keep my notes" };
  const snapshots = [existing];
  const created = [];
  let listReads = 0;
  const store = createQuarterlyStore(async (command, args) => {
    switch (command) {
      case "ensure_current_quarter_snapshot": return null;
      case "check_missing_snapshots":
        return ["2024-Q4", "2025-Q1", "2025-Q3"].filter(q => !snapshots.some(s => s.quarter === q));
      case "create_quarterly_snapshot": {
        created.push(args.quarter);
        const snapshot = { id: args.quarter, quarter: args.quarter };
        snapshots.push(snapshot);
        return snapshot;
      }
      case "get_quarterly_snapshots": listReads++; return [...snapshots];
      default: throw new Error(`Unexpected command: ${command}`);
    }
  });

  await store.getState().initializeSnapshots();
  assert.deepEqual(created, ["2024-Q4", "2025-Q1", "2025-Q3"]);
  assert.equal(listReads, 2);
  assert.equal(store.getState().snapshots.length, 4);
  assert.equal(store.getState().snapshots[0], existing);
  assert.deepEqual(store.getState().missingQuarters, []);
  assert.equal(store.getState().initializationError, null);
  assert.equal(store.getState().initializationLoading, false);

  await store.getState().initializeSnapshots();
  assert.equal(created.length, 3);
  assert.equal(listReads, 3);
  assert.equal(store.getState().snapshots[0].overall_notes, "Keep my notes");
});

test("existing snapshots become usable before gap detection or backfill finishes", async () => {
  const pendingScan = deferred();
  const pendingCreate = deferred();
  const existing = { id: "existing", quarter: "2025-Q2" };
  const added = { id: "added", quarter: "2025-Q1" };
  let created = false;
  const store = createQuarterlyStore(async (command) => {
    switch (command) {
      case "check_missing_snapshots": return pendingScan.promise;
      case "create_quarterly_snapshot":
        await pendingCreate.promise;
        created = true;
        return added;
      case "get_quarterly_snapshots": return created ? [existing, added] : [existing];
      default: throw new Error(`Unexpected command: ${command}`);
    }
  });
  const initializing = store.getState().initializeSnapshots();
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(store.getState().snapshots, [existing]);
  assert.equal(store.getState().listLoading, false);
  assert.equal(store.getState().initializationLoading, true);

  pendingScan.resolve(["2025-Q1"]);
  await new Promise(resolve => setImmediate(resolve));
  assert.deepEqual(store.getState().snapshots, [existing]);
  assert.equal(store.getState().listLoading, false);
  const loadingChanges = [];
  const unsubscribe = store.subscribe(state => loadingChanges.push(state.listLoading));
  pendingCreate.resolve();
  await initializing;
  unsubscribe();
  assert.deepEqual(store.getState().snapshots, [existing, added]);
  assert.ok(loadingChanges.every(loading => !loading));
  assert.equal(store.getState().initializationLoading, false);
});

test("overlapping page initialization shares one backfill and stays busy until the list loads", async () => {
  const pendingCreate = deferred();
  const pendingList = deferred();
  let created = 0;
  const store = createQuarterlyStore(async (command) => {
    switch (command) {
      case "ensure_current_quarter_snapshot": return null;
      case "check_missing_snapshots": return ["2025-Q1"];
      case "create_quarterly_snapshot": created++; return pendingCreate.promise;
      case "get_quarterly_snapshots": return pendingList.promise;
      default: throw new Error(`Unexpected command: ${command}`);
    }
  });
  const first = store.getState().initializeSnapshots();
  const second = store.getState().initializeSnapshots();
  assert.equal(store.getState().initializationLoading, true);
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(created, 1);
  pendingCreate.resolve({ id: "new", quarter: "2025-Q1" });
  await new Promise(resolve => setImmediate(resolve));
  assert.equal(store.getState().initializationLoading, true);
  pendingList.resolve([{ id: "new", quarter: "2025-Q1" }]);
  await Promise.all([first, second]);
  assert.equal(created, 1);
  assert.equal(store.getState().initializationLoading, false);
  assert.equal(store.getState().snapshots[0].id, "new");
});

test("a failed quarter does not block later quarters and can be retried", async () => {
  const snapshots = [];
  const attempted = [];
  let fail = true;
  const store = createQuarterlyStore(async (command, args) => {
    switch (command) {
      case "ensure_current_quarter_snapshot": return null;
      case "check_missing_snapshots":
        return ["2025-Q1", "2025-Q2"].filter(q => !snapshots.some(s => s.quarter === q));
      case "create_quarterly_snapshot": {
        attempted.push(args.quarter);
        if (fail && args.quarter === "2025-Q1") throw new Error("missing historical exchange rates");
        const snapshot = { id: args.quarter, quarter: args.quarter };
        snapshots.push(snapshot);
        return snapshot;
      }
      case "get_quarterly_snapshots": return [...snapshots];
      default: throw new Error(`Unexpected command: ${command}`);
    }
  });
  await store.getState().initializeSnapshots();
  assert.deepEqual(attempted, ["2025-Q1", "2025-Q2"]);
  assert.deepEqual(store.getState().missingQuarters, ["2025-Q1"]);
  assert.deepEqual(store.getState().snapshots.map(s => s.quarter), ["2025-Q2"]);
  assert.match(store.getState().initializationError, /2025-Q1.*missing historical exchange rates/);
  assert.equal(store.getState().initializationLoading, false);

  fail = false;
  await store.getState().initializeSnapshots();
  assert.deepEqual(attempted, ["2025-Q1", "2025-Q2", "2025-Q1"]);
  assert.deepEqual(store.getState().missingQuarters, []);
  assert.equal(store.getState().initializationError, null);
});

test("a failed gap scan still loads existing snapshots and reports the error", async () => {
  const store = createQuarterlyStore(async (command) => {
    switch (command) {
      case "ensure_current_quarter_snapshot": return null;
      case "check_missing_snapshots": throw new Error("Bad transaction date");
      case "get_quarterly_snapshots": return [{ id: "existing", quarter: "2025-Q2" }];
      default: throw new Error(`Unexpected command: ${command}`);
    }
  });
  await store.getState().initializeSnapshots();
  assert.equal(store.getState().snapshots[0].id, "existing");
  assert.match(store.getState().initializationError, /Bad transaction date/);
  assert.equal(store.getState().initializationLoading, false);
});

test("initialization still ensures the current quarter when there is no transaction history", async () => {
  let current = null;
  const store = createQuarterlyStore(async (command) => {
    switch (command) {
      case "ensure_current_quarter_snapshot": current = { id: "current", quarter: "2026-Q3" }; return current;
      case "check_missing_snapshots": return [];
      case "get_quarterly_snapshots": return [current];
      default: throw new Error(`Unexpected command: ${command}`);
    }
  });
  await store.getState().initializeSnapshots();
  assert.equal(store.getState().snapshots[0].id, "current");
  assert.deepEqual(store.getState().missingQuarters, []);
});

test("current-quarter creation errors remain visible without transaction history", async () => {
  let fail = true;
  const store = createQuarterlyStore(async (command) => {
    switch (command) {
      case "ensure_current_quarter_snapshot":
        if (fail) throw new Error("current quotes unavailable");
        return null;
      case "check_missing_snapshots": return [];
      case "get_quarterly_snapshots": return [];
      default: throw new Error(`Unexpected command: ${command}`);
    }
  });
  await store.getState().initializeSnapshots();
  assert.match(store.getState().initializationError, /当前季度.*current quotes unavailable/);
  assert.equal(store.getState().initializationLoading, false);
  fail = false;
  await store.getState().initializeSnapshots();
  assert.equal(store.getState().initializationError, null);
});

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

test("loading another snapshot clears the previous snapshot mutation error", async () => {
  const store = createQuarterlyStore(async (command) => command === "get_quarterly_transactions" ? [] : detail("B"));
  store.setState({ detailSnapshotId: "A", detail: detail("A"), mutationError: "old failure" });
  await store.getState().fetchDetail("B");
  assert.equal(store.getState().detail.snapshot.id, "B");
  assert.equal(store.getState().mutationError, null);
});
