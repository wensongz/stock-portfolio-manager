import assert from "node:assert/strict";
import test from "node:test";
import type { Transaction, TransactionType } from "../types/index.ts";
import * as pnl from "./transactionPnl.ts";

function transaction(id: string, type: TransactionType, shares: number, price: number, commission = 0): Transaction {
  return {
    id, holding_id: "holding", account_id: "account", symbol: "DDOG", name: "Datadog",
    market: "US", currency: "USD", transaction_type: type, shares, price,
    total_amount: shares * price, commission, traded_at: `2026-10-${id}T10:00:00`,
    created_at: `2026-10-${id}T10:00:00`, notes: null,
  };
}

test("replays newest-first trades chronologically with weighted costs and both sides' fees", () => {
  const transactions = [
    transaction("03", "SELL", 5, 40, 2),
    transaction("02", "BUY", 10, 30, 1),
    transaction("01", "BUY", 10, 20, 1),
  ];
  const result = pnl.calculateTransactionPnl(transactions, 35);
  assert.deepEqual(result, { currency: "USD", shares: 15, remainingCost: 376.5, realizedPnl: 72.5, unrealizedPnl: 148.5 });
  assert.deepEqual(transactions.map(row => row.id), ["03", "02", "01"], "the table keeps its original order");
});

test("an opening position, dividends and repurchases retain lifetime realized profit", () => {
  const dividend = { ...transaction("03", "PAY", 0, 0, 2), total_amount: 30 };
  const result = pnl.calculateTransactionPnl([
    transaction("05", "SELL", 2, 15, 1),
    transaction("04", "BUY", 5, 10, 1),
    dividend,
    transaction("02", "SELL", 10, 25, 1),
    transaction("01", "OPEN", 10, 20),
  ], 12);
  assert.equal(result?.shares, 3);
  assert.ok(Math.abs(result!.remainingCost - 30.6) < 1e-10);
  assert.ok(Math.abs(result!.realizedPnl - 85.6) < 1e-10);
  assert.ok(Math.abs(result!.unrealizedPnl! - 5.4) < 1e-10);
});

test("stock withdrawals remove proportional cost without realizing a sale", () => {
  const result = pnl.calculateTransactionPnl([
    transaction("03", "STOCK_OUT", 5, 0),
    transaction("02", "STOCK_IN", 10, 40),
    transaction("01", "STOCK_IN", 10, 20),
  ], 35);
  assert.deepEqual(result, { currency: "USD", shares: 15, remainingCost: 450, realizedPnl: 0, unrealizedPnl: 75 });
});

test("closed positions have zero floating profit even without a quote and preserve losses", () => {
  const result = pnl.calculateTransactionPnl([
    transaction("02", "SELL", 10, 15, 1),
    transaction("01", "BUY", 10, 20, 1),
  ], null);
  assert.deepEqual(result, { currency: "USD", shares: 0, remainingCost: 0, realizedPnl: -52, unrealizedPnl: 0 });
});

test("broker fee refunds remain part of acquisition cost and realized proceeds", () => {
  const result = pnl.calculateTransactionPnl([
    transaction("02", "SELL", 4, 30, -1),
    transaction("01", "BUY", 10, 20, -1),
  ], 35);
  assert.ok(result);
  assert.ok(Math.abs(result.remainingCost - 119.4) < 1e-10);
  assert.ok(Math.abs(result.realizedPnl - 41.4) < 1e-10);
  assert.ok(Math.abs(result.unrealizedPnl! - 90.6) < 1e-10);
});

test("missing or invalid quotes leave floating profit unavailable while zero is a valid price", () => {
  const rows = [transaction("01", "BUY", 10, 20, 1)];
  for (const quote of [null, undefined, NaN, Infinity, -1]) {
    const result = pnl.calculateTransactionPnl(rows, quote);
    assert.equal(result?.realizedPnl, 0);
    assert.equal(result?.unrealizedPnl, null);
  }
  assert.equal(pnl.calculateTransactionPnl(rows, 0)?.unrealizedPnl, -201);
});

test("equal trade times use creation time and then id, matching position replay", () => {
  const buy = transaction("01", "BUY", 10, 20);
  const sell = { ...transaction("02", "SELL", 5, 30), traded_at: buy.traded_at };
  const otherBuy = { ...buy, id: "00" };
  assert.equal(pnl.calculateTransactionPnl([sell, buy], 25)?.realizedPnl, 50);
  assert.equal(pnl.calculateTransactionPnl([{ ...sell, created_at: buy.created_at }, otherBuy], 25)?.realizedPnl, 50);
});

test("unusable history never produces plausible zero profit", () => {
  const buy = transaction("01", "BUY", 10, 20);
  assert.equal(pnl.calculateTransactionPnl([], 30), null);
  assert.equal(pnl.calculateTransactionPnl([transaction("02", "SELL", 11, 30), buy], 30), null);
  assert.equal(pnl.calculateTransactionPnl([{ ...buy, price: NaN }], 30), null);
});

test("roundoff at full liquidation is tolerated without erasing small real positions", () => {
  const closed = pnl.calculateTransactionPnl([
    transaction("03", "SELL", 0.8, 25),
    transaction("02", "BUY", 0.7, 20),
    transaction("01", "BUY", 0.1, 20),
  ], null);
  assert.equal(closed?.shares, 0);
  assert.equal(closed?.unrealizedPnl, 0);
  const tiny = pnl.calculateTransactionPnl([
    transaction("02", "SELL", 5e-11, 25),
    transaction("01", "BUY", 1e-10, 20),
  ], 30);
  assert.equal(tiny?.shares, 5e-11);
  assert.ok(Math.abs(tiny!.unrealizedPnl! - 5e-10) < 1e-24);
  assert.equal(pnl.calculateTransactionPnl([
    transaction("02", "SELL", 2e-10, 25),
    transaction("01", "BUY", 1e-10, 20),
  ], 30), null);
});
