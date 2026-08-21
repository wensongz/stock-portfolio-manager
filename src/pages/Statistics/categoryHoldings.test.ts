// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import { filterCategoryHoldings } from "./categoryHoldings.ts";

test("filterCategoryHoldings keeps only active stock positions in the selected category", () => {
  const holdings = [
    { symbol: "AAPL", category_id: "growth", shares: 2 },
    { symbol: "MSFT", category_id: "quality", shares: 3 },
    { symbol: "$CASH-USD", category_id: "growth", shares: 100 },
    { symbol: "NVDA", category_id: "growth", shares: 0 },
  ];

  assert.deepEqual(filterCategoryHoldings(holdings, "growth"), [holdings[0]]);
});
