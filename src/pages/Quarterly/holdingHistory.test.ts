// @ts-nocheck -- Run directly with Node's TypeScript support.
import test from "node:test";
import assert from "node:assert/strict";
import { createHoldingHistoryEditor, formatQuarterlyOperation, isHoldingEditorTargetCurrent } from "./holdingHistory.ts";

const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };
const target = { snapshotId: "quarter-a", holdingSnapshotId: "row-a", notes: "Original", showHistory: true };
const history = (snapshot = "quarter-a", row = "row-a") => ({ snapshot_id: snapshot, holding_snapshot_id: row, account_id: row, symbol: "$CASH-USD", currency: "USD", quarter: "2026Q1", start_date: "2026-01-01", end_date: "2026-03-31", rows: [] });

test("history requests exact row and quarter, clears on retry and ignores stale quarter/account results", async () => {
  const calls = [], states = [], pending = [];
  const editor = createHoldingHistoryEditor(async (command, args) => { calls.push({ command, args }); const p = deferred(); pending.push(p); return p.promise; }, async () => {}, state => states.push(state));
  const first = editor.open(target);
  assert.equal(states.at(-1).mode, "history");
  const second = editor.open({ ...target, holdingSnapshotId: "row-b" });
  assert.equal(states.at(-1).history, null);
  pending[1].resolve(history("quarter-a", "row-b")); await second;
  pending[0].resolve(history()); await first;
  assert.equal(states.at(-1).history.holding_snapshot_id, "row-b");
  const retry = editor.retry();
  assert.equal(states.at(-1).history, null);
  const switched = editor.open({ ...target, snapshotId: "quarter-b", holdingSnapshotId: "row-a" });
  pending[3].resolve(history("quarter-b")); await switched;
  pending[2].reject(new Error("old failure")); await retry;
  assert.equal(states.at(-1).history.snapshot_id, "quarter-b");
  assert.equal(states.at(-1).historyError, null);
  assert.deepEqual(calls[0], { command: "get_quarterly_holding_history", args: { snapshotId: "quarter-a", holdingSnapshotId: "row-a" } });
  assert.deepEqual(calls[1].args, { snapshotId: "quarter-a", holdingSnapshotId: "row-b" });
});

test("history errors are visible, retries preserve notes, and closing ignores late data", async () => {
  let pending = deferred(); const states = [];
  const editor = createHoldingHistoryEditor(() => pending.promise, async () => {}, state => states.push(state));
  const opening = editor.open({ ...target, showHistory: false }); await opening;
  assert.equal(states.at(-1).mode, "edit");
  editor.setNotes("Unsaved draft");
  const load = editor.setMode("history");
  pending.reject(new Error("history offline")); await load;
  assert.match(states.at(-1).historyError, /history offline/);
  assert.equal(states.at(-1).notes, "Unsaved draft");
  pending = deferred(); const retry = editor.retry();
  editor.close(); const closed = states.at(-1);
  pending.resolve(history()); await retry;
  assert.equal(states.at(-1), closed);
  assert.equal(states.at(-1).history, null);
});

test("save rejects duplicates, preserves failed draft, and old completion cannot close another editor", async () => {
  const states = [], saves = []; let pending = deferred();
  const editor = createHoldingHistoryEditor(async () => history(), (...args) => { saves.push(args); return pending.promise; }, state => states.push(state));
  await editor.open({ ...target, showHistory: false }); editor.setNotes("Draft A");
  const saving = editor.save();
  assert.equal(await editor.save(), false);
  assert.deepEqual(saves, [["quarter-a", "row-a", "Draft A"]]);
  pending.reject(new Error("save failed")); assert.equal(await saving, false);
  assert.equal(states.at(-1).notes, "Draft A");
  assert.match(states.at(-1).saveError, /save failed/);
  pending = deferred(); const retry = editor.save();
  await editor.open({ ...target, holdingSnapshotId: "row-b", notes: "B", showHistory: false });
  pending.resolve(); assert.equal(await retry, false);
  assert.equal(states.at(-1).notes, "B");
  assert.equal(states.at(-1).saving, false);
});

test("operation presentation uses UTC quarter dates, finite precision and real signed backend cash rows", () => {
  const row = { id: "trade", symbol: "ABC", name: "Example", transaction_type: "BUY", traded_at: "2026-04-01T07:30:00+08:00", shares: 0.30000000000000004, price: 10.123456789, total_amount: 3.0370370367, commission: 0.010000000000000002, currency: "USD", notes: "Original note", cash_delta: -3.0470370367, running_balance: -0.30000000000000004 };
  const display = formatQuarterlyOperation(row);
  assert.equal(display.date, "2026-03-31 23:30:00");
  assert.equal(display.shares, "0.3");
  assert.equal(display.price, "$10.1235");
  assert.equal(display.amount, "$3.04");
  assert.equal(display.commission, "$0.01");
  assert.equal(display.cashDelta, "-$3.05");
  assert.equal(display.runningBalance, "$-0.30");
  assert.equal(display.notes, "Original note");
  assert.equal(formatQuarterlyOperation({ ...row, traded_at: "2026-03-31 23:30:00", transaction_type: "STOCK_IN" }).date, display.date);
  assert.equal(formatQuarterlyOperation({ ...row, symbol: "$CASH-USD", transaction_type: "OPEN", cash_delta: 0, running_balance: 0 }).type, "期初");
  assert.equal(formatQuarterlyOperation({ ...row, cash_delta: null, running_balance: null }).cashDelta, "—");
  assert.equal(formatQuarterlyOperation({ ...row, symbol: "$cash-usd", transaction_type: "BUY", traded_at: "2026-01-01" }).type, "存入");
  assert.equal(formatQuarterlyOperation({ ...row, traded_at: "2026-01-01" }).date, "2026-01-01 00:00:00");
});

test("rebuilding a quarter invalidates an old row without rebinding to the same symbol in another account", () => {
  const selected = { snapshotId: "quarter-a", holding: { id: "old-row", symbol: "$CASH-USD", account_id: "account-a" } };
  assert.equal(isHoldingEditorTargetCurrent(selected, "quarter-a", [{ id: "old-row", quarterly_snapshot_id: "quarter-a" }]), true);
  assert.equal(isHoldingEditorTargetCurrent(selected, "quarter-b", [{ id: "old-row", quarterly_snapshot_id: "quarter-a" }]), false);
  assert.equal(isHoldingEditorTargetCurrent(selected, "quarter-a", [{ id: "new-row", quarterly_snapshot_id: "quarter-a", symbol: "$CASH-USD", account_id: "account-b" }]), false);
});
