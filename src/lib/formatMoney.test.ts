// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import { formatHoldingShares } from "./formatMoney.ts";

test("formatHoldingShares renders the three cash balances as integers", () => {
  assert.equal(formatHoldingShares(1_234.56, "$CASH-USD"), "1,235");
  assert.equal(formatHoldingShares(2_345.67, "$CASH-CNY"), "2,346");
  assert.equal(formatHoldingShares(3_456.78, "$CASH-HKD"), "3,457");
});

test("formatHoldingShares preserves fractional stock quantities", () => {
  assert.equal(formatHoldingShares(12.345, "AAPL"), "12.345");
});
