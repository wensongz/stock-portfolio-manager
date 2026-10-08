// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import { formatHoldingShares, formatMoney } from "./formatMoney.ts";

test("formatMoney puts negative signs before currency symbols across currencies", () => {
  assert.equal(formatMoney(-740_948.79, "USD"), "-$740,948.79");
  assert.equal(formatMoney(-1234.56, "CNY"), "-¥1,234.56");
  assert.equal(formatMoney(-1234.56, "HKD"), "-HK$1,234.56");
  assert.equal(formatMoney(-1234.56, "EUR"), "-EUR1,234.56");
});

test("formatMoney preserves precision, grouping, and zero while supporting explicit signs", () => {
  assert.equal(formatMoney(1_697_148.22, "USD", 2, { signDisplay: "always" }), "+$1,697,148.22");
  assert.equal(formatMoney(-1234.56, "HK$", 2, { signDisplay: "always", useGrouping: false }), "-HK$1234.56");
  assert.equal(formatMoney(1234.56, "USD"), "$1,234.56");
  assert.equal(formatMoney(-1234.56, "USD", 0), "-$1,235");
  assert.equal(formatMoney(-1.23456, "USD", 4), "-$1.2346");
  assert.equal(formatMoney(0, "USD"), "$0.00");
  assert.equal(formatMoney(-0, "USD"), "-$0.00");
  assert.equal(formatMoney(-0.001, "USD"), "-$0.00");
});

test("formatHoldingShares renders the three cash balances as integers", () => {
  assert.equal(formatHoldingShares(1_234.56, "$CASH-USD"), "1,235");
  assert.equal(formatHoldingShares(2_345.67, "$CASH-CNY"), "2,346");
  assert.equal(formatHoldingShares(3_456.78, "$CASH-HKD"), "3,457");
});

test("formatHoldingShares preserves fractional stock quantities", () => {
  assert.equal(formatHoldingShares(12.345, "AAPL"), "12.345");
});
