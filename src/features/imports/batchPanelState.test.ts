// @ts-nocheck -- Node test fixtures focus on state transitions.
import test from "node:test";
import assert from "node:assert/strict";
import { addExpectedBalance, reconciliationDisplayRows, reconciliationMatches, batchReconciliationArgs, initialBatchSelection, batchApplyArgs, selectionAfterBatchResponse } from "./batchPanelState.ts";
const batch = { id: "batch", status: "preview", rows: ["ready", "suspected", "duplicate", "failed", "imported"].map((status) => ({key: status, status})) };
test("only ready rows default selected; suspected rows need explicit opt-in", () => {
  assert.deepEqual(initialBatchSelection(batch), ["ready"]);
  assert.deepEqual(batchApplyArgs(batch, ["ready", "suspected", "duplicate", "failed", "imported"]), {
    batchId: "batch", rowKeys: ["ready", "suspected", "failed"], allowSuspectedKeys: ["suspected"],
  });
});
test("partial response removes successful rows but retains failed and unsubmitted selections for retry", () => {
  const response = {...batch, status: "applied", rows: [{key:"a", status:"imported"}, {key:"b",status:"failed"}, {key:"c",status:"ready"}]};
  assert.deepEqual(selectionAfterBatchResponse(response, ["a","b","c"]), ["b","c"]);
});
test("undone batches cannot select or retry any rows", () => {
  const undone = {...batch, status:"undone"};
  assert.deepEqual(initialBatchSelection(undone), []);
  assert.deepEqual(batchApplyArgs(undone, ["ready", "failed"]), {batchId:"batch", rowKeys:[], allowSuspectedKeys:[]});
});

test("reconciliation preserves zero and cash balances and omits cleared values", () => {
  const reconciled = {...batch, reconciliation:[{symbol:"AAPL"},{symbol:"CASH_USD"},{symbol:"MSFT"},{symbol:"TSLA"}]};
  assert.deepEqual(batchReconciliationArgs(reconciled, {AAPL:0, CASH_USD:-12.5, MSFT:null}), {
    batchId:"batch", balances:[{symbol:"AAPL",expected_shares:0},{symbol:"CASH_USD",expected_shares:-12.5}],
  });
});

test("commit recheck requires fresh consent when a ready row becomes suspected", () => {
  const response = {...batch, status:"applied", rows:[{key:"ready", status:"suspected"}, {key:"retry",status:"failed"}]};
  const retained = selectionAfterBatchResponse(response, ["ready", "retry"]);
  assert.deepEqual(retained, ["retry"]);
  assert.deepEqual(batchApplyArgs(response, retained).allowSuspectedKeys, []);
  assert.deepEqual(batchApplyArgs(response, [...retained, "ready"]).allowSuspectedKeys, ["ready"]);
});
test("conflicted batches cannot submit even previously selected rows", () => {
  assert.deepEqual(batchApplyArgs({...batch, conflict:"Account changed"}, ["ready", "failed"]), {
    batchId:"batch", rowKeys:[], allowSuspectedKeys:[],
  });
});
test("reconciliation labels tolerate rounding but preserve material cash and share differences", () => {
  const row = {symbol:"AAPL", expected_shares:1, difference:1e-10};
  assert.equal(reconciliationMatches(row), true);
  assert.equal(reconciliationMatches({...row, difference:1e-7}), false);
  assert.equal(reconciliationMatches({...row, symbol:"$CASH-USD", difference:-0.004}), true);
  assert.equal(reconciliationMatches({...row, symbol:"$CASH-USD", difference:0.006}), false);
  assert.equal(reconciliationMatches({...row, expected_shares:null, difference:null}), false);
});

test("broker-only balances normalize symbols, preserve pending inputs, and submit missing holdings", () => {
  const current = {...batch, reconciliation:[{symbol:"AAPL", expected_shares:1}]};
  const balances = addExpectedBalance(current, {AAPL:12}, " msft ", 4);
  assert.deepEqual(balances, {AAPL:12, MSFT:4});
  assert.deepEqual(batchReconciliationArgs(current, balances).balances, [
    {symbol:"AAPL",expected_shares:12}, {symbol:"MSFT",expected_shares:4},
  ]);
  const missing = reconciliationDisplayRows(current, balances).find((row)=>row.symbol === "MSFT");
  assert.equal(missing.before_shares, 0);
  assert.equal(missing.after_shares, 0);
  assert.equal(missing.expected_shares, null);
  const cash = addExpectedBalance(current, balances, " $cash-usd ", 150);
  assert.equal(reconciliationDisplayRows(current, cash).find((row)=>row.symbol === "$CASH-USD").currency, "USD");
});
test("broker-only balance entry rejects empty and normalized duplicate symbols", () => {
  const current = {...batch, reconciliation:[{symbol:"AAPL", expected_shares:null}]};
  assert.throws(()=>addExpectedBalance(current, {}, "   ", 1), /证券代码/);
  assert.throws(()=>addExpectedBalance(current, {}, " aapl ", 1), /已存在/);
  assert.throws(()=>addExpectedBalance(current, {MSFT:3}, " msft ", 1), /已存在/);
  assert.throws(()=>addExpectedBalance(current, {}, "MSFT", null), /有效数量/);
});
