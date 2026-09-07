// @ts-nocheck -- Run directly with Node's TypeScript support.
import test from "node:test";
import assert from "node:assert/strict";
import { createHoldingRequest, cashBalanceEditDecision, cashBalanceSaveCommand, createEditSession, mergeHoldingQuote, formatCashDelta } from "./cashBalanceEditing.ts";

const holding = { id: "cash-a", account_id: "account-a", symbol: "$CASH-USD", name: "Cash", market: "US", currency: "USD", category_id: null, shares: 80, avg_cost: 1 };
const reconciliation = { holding_id: holding.id, account_id: holding.account_id, currency: "USD", current_balance: 80, recommended_balance: -20, difference: -100, revision: 4, opening_count: 1, rows: [{ id: "trade", cash_delta: -100, running_balance: -20 }] };
const ready = (data = reconciliation) => ({ holdingId: holding.id, status: "ready", data, error: null });
const deferred = () => { let resolve, reject; const promise = new Promise((a, b) => { resolve = a; reject = b; }); return { promise, resolve, reject }; };

test("cash with trades accepts zero and negative corrections but never unavailable or nonfinite recommendations", () => {
  for (const balance of [0, -20, -150]) assert.equal(cashBalanceEditDecision(holding, balance, ready()).canSubmit, true);
  for (const balance of [null, undefined, NaN, Infinity, -Infinity]) assert.equal(cashBalanceEditDecision(holding, balance, ready()).canSubmit, false);
  for (const status of ["idle", "loading", "error"]) {
    const state = { holdingId: holding.id, status, data: null, error: "offline" };
    assert.equal(cashBalanceEditDecision(holding, 80, state).canSubmit, true, "metadata can still be saved");
    assert.equal(cashBalanceEditDecision(holding, 0, state).canSubmit, false, "unverified draft must not reach metadata API");
  }
  assert.equal(cashBalanceEditDecision(holding, 0, { ...ready(), holdingId: "cash-b" }).canSubmit, false);
  assert.equal(cashBalanceEditDecision(holding, 0, ready({ ...reconciliation, holding_id: "cash-b" })).canSubmit, false);
  const empty = ready({ ...reconciliation, rows: [], recommended_balance: null, difference: null, opening_count: 0 });
  assert.equal(cashBalanceEditDecision(holding, 0, empty).canSubmit, true, "empty history allows a custom opening");
  assert.equal(cashBalanceEditDecision(holding, 0, empty).canAdopt, false);
});

test("multiple openings permit the recorded recommendation and metadata, not a custom opening rewrite", () => {
  const state = ready({ ...reconciliation, opening_count: 2 });
  assert.equal(cashBalanceEditDecision(holding, -20, state).canSubmit, true);
  assert.equal(cashBalanceEditDecision(holding, 80, state).canSubmit, true);
  assert.equal(cashBalanceEditDecision(holding, 0, state).canSubmit, false);
  assert.equal(cashBalanceEditDecision(holding, 0, ready({ ...reconciliation, recommended_balance: 0 })).canAdopt, true);
  assert.equal(cashBalanceEditDecision(holding, 10, ready({ ...reconciliation, opening_count: 2, recommended_balance: 10.004 })).canSubmit, true, "the two-decimal input may round a full-precision recommendation");
  assert.equal(cashBalanceEditDecision(holding, -10, ready({ ...reconciliation, opening_count: 2, recommended_balance: -10.004 })).canSubmit, true);
  assert.equal(cashBalanceEditDecision(holding, -10, ready({ ...reconciliation, opening_count: 2, recommended_balance: -10.005 })).canSubmit, false, "negative half-cent rounding matches the backend");
});

test("metadata fallback keeps original identity and financial fields; an unverified draft cannot be silently discarded", () => {
  const values = { name: "Renamed", categoryId: "cash", accountId: "wrong", symbol: "WRONG", shares: 80, avgCost: 9, currency: "HKD", market: "HK" };
  const failed = { holdingId: holding.id, status: "error", data: null, error: "offline" };
  assert.deepEqual(cashBalanceSaveCommand(holding, values, failed), {
    kind: "metadata", payload: { id: holding.id, accountId: holding.account_id, symbol: holding.symbol, name: values.name, categoryId: values.categoryId, market: holding.market, shares: holding.shares, avgCost: holding.avg_cost, currency: holding.currency },
  });
  assert.throws(() => cashBalanceSaveCommand(holding, { ...values, shares: 0 }, failed), /核对/);
  assert.deepEqual(cashBalanceSaveCommand(holding, { ...values, shares: -20 }, ready()), {
    kind: "correction", payload: { id: holding.id, balance: -20, expectedRevision: 4, name: values.name, categoryId: values.categoryId },
  });
});

test("a newer preview balance makes the untouched old draft an explicit revision-checked correction", () => {
  const state = ready({ ...reconciliation, current_balance: 100, recommended_balance: 100, difference: 0, revision: 5 });
  const command = cashBalanceSaveCommand(holding, { shares: 80, name: "Cash", categoryId: undefined }, state);
  assert.equal(command.kind, "correction");
  assert.equal(command.payload.balance, 80);
  assert.equal(command.payload.expectedRevision, 5);
  assert.equal(cashBalanceSaveCommand(holding, { shares: 100, name: "Cash" }, state).kind, "correction");
});

test("saving blocks duplicate submissions and a late save cannot close or unlock a new edit session", () => {
  const session = createEditSession();
  session.open();
  const first = session.beginSave();
  assert.notEqual(first, null);
  assert.equal(session.beginSave(), null);
  assert.equal(session.finishSave(first), true, "failure can release current session without changing form values");
  const retried = session.beginSave();
  session.open();
  const other = session.beginSave();
  assert.equal(session.isCurrent(retried), false);
  assert.equal(session.finishSave(retried), false);
  assert.equal(session.beginSave(), null, "old completion does not clear new saving state");
  assert.equal(session.finishSave(other), true);
  session.close();
  assert.equal(session.beginSave(), null, "late form validation after closing cannot submit again");
});

test("request generation isolates retries, holding switches, and close; old errors never replace current rows", async () => {
  const states = [];
  const request = createHoldingRequest(state => states.push(state));
  const first = deferred(), retry = deferred(), other = deferred();
  const p1 = request.load("cash-a", () => first.promise);
  const p2 = request.load("cash-a", () => retry.promise);
  retry.resolve(reconciliation); await p2;
  first.reject(new Error("old failure")); await p1;
  assert.equal(states.at(-1).status, "ready");
  assert.equal(states.at(-1).data, reconciliation);
  const p3 = request.load("cash-b", () => other.promise);
  assert.equal(states.at(-1).data, null);
  request.clear();
  other.resolve({ ...reconciliation, holding_id: "cash-b" }); await p3;
  assert.equal(states.at(-1).status, "idle");
  const switched = deferred();
  const p4 = request.load("cash-a", () => switched.promise);
  await request.load("cash-b", async () => ({ holding_id: "cash-b" }));
  switched.resolve(reconciliation); await p4;
  assert.equal(states.at(-1).holdingId, "cash-b");
});

test("current cash holding wins over stale quote balances, including zero and debit cash", () => {
  const oldQuote = { ...holding, quote: { current_price: 1 }, market_value: 80, total_cost: 80, unrealized_pnl: 0, unrealized_pnl_percent: 0 };
  for (const balance of [0, -40]) {
    const merged = mergeHoldingQuote({ ...holding, shares: balance, name: "Corrected" }, oldQuote);
    assert.equal(merged.shares, balance);
    assert.equal(merged.market_value, balance);
    assert.equal(merged.total_cost, balance);
    assert.equal(merged.name, "Corrected");
    assert.equal(merged.unrealized_pnl, 0);
  }
  assert.equal(formatCashDelta(-12.5, "USD"), "-$12.50");
  assert.equal(formatCashDelta(0, "CNY"), "+¥0.00");
});
