// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  STOCK_REVIEW_FILTERS_STORAGE_KEY,
  createDefaultStockReviewFilters,
  getStockReviewDateRange,
  loadStockReviewFilters,
} from "./stockReviewViewModel.ts";

const now = new Date("2026-08-28T00:00:00+08:00");

function memoryStorage(value: Record<string, unknown>) {
  return {
    getItem(key: string) {
      return key === STOCK_REVIEW_FILTERS_STORAGE_KEY ? JSON.stringify(value) : null;
    },
  };
}

test("1Y clamps the prior anniversary before advancing its inclusive start", () => {
  assert.deepEqual(
    getStockReviewDateRange("1Y", new Date("2024-02-29T23:30:00+08:00")),
    { startDate: "2023-03-01", endDate: "2024-02-29" },
  );
});

test("non-custom persisted dates and saved currency are validated before recomputing YTD", () => {
  const valid = {
    accountId: "account-a",
    periodPreset: "YTD",
    startDate: "2026-01-01",
    endDate: "2026-08-28",
    market: "US",
    benchmarkSymbol: "QQQ",
    baseCurrency: "USD",
  };
  const fallback = createDefaultStockReviewFilters(now, "HKD");

  for (const corrupt of [
    { ...valid, startDate: "2026-02-30" },
    { ...valid, baseCurrency: "EUR" },
  ]) {
    assert.deepEqual(loadStockReviewFilters(memoryStorage(corrupt), now, "HKD"), fallback);
  }
});
