// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  filterActiveOverviewHoldings,
  filterActiveStockHoldings,
  filterCategoryHoldings,
} from "./categoryHoldings.ts";

test("filterActiveOverviewHoldings keeps positive cash and stock details", () => {
  const holdings = [
    { symbol: "$CASH-USD", shares: 100.25 },
    { symbol: "AAPL", shares: 1.5 },
    { symbol: "$CASH-HKD", shares: 0 },
  ];

  assert.deepEqual(filterActiveOverviewHoldings(holdings), holdings.slice(0, 2));
});

test("cash overdrafts remain active in overview and cash category but not stock categories", () => {
  const holdings = [
    { symbol: "$CASH-CNY", category_id: "cash", shares: 100000 },
    { symbol: "$CASH-HKD", category_id: "growth", shares: -7800 },
    { symbol: "700.HK", category_id: "growth", shares: 100 },
    { symbol: "CLOSED", category_id: "growth", shares: 0 },
    { symbol: "SHORT", category_id: "growth", shares: -10 },
    { symbol: "$CASH-USD", category_id: "cash", shares: 0 },
  ];

  assert.deepEqual(filterActiveOverviewHoldings(holdings), holdings.slice(0, 3));
  assert.deepEqual(filterActiveStockHoldings(holdings, true), holdings.slice(0, 3));
  assert.deepEqual(filterActiveStockHoldings(holdings), [holdings[2]]);
  assert.deepEqual(filterCategoryHoldings(holdings, "growth"), [holdings[2]]);
});

test("filterCategoryHoldings keeps only active stock positions in the selected category", () => {
  const holdings = [
    { symbol: "AAPL", category_id: "growth", shares: 2 },
    { symbol: "MSFT", category_id: "quality", shares: 3 },
    { symbol: "$CASH-USD", category_id: "growth", shares: 100 },
    { symbol: "NVDA", category_id: "growth", shares: 0 },
  ];

  assert.deepEqual(filterCategoryHoldings(holdings, "growth"), [holdings[0]]);
});

test("filterActiveStockHoldings keeps cash and other assets in the system cash category", () => {
  const holdings = [
    { symbol: "$CASH-USD", shares: 100 },
    { symbol: "$CASH-CNY", shares: 700 },
    { symbol: "$CASH-HKD", shares: 780 },
    { symbol: "AAPL", shares: 2 },
  ];

  assert.deepEqual(
    filterActiveStockHoldings(holdings, true),
    holdings,
  );
  assert.deepEqual(filterActiveStockHoldings(holdings, false), [holdings[3]]);
});
