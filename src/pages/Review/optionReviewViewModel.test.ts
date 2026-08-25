// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  formatReviewPercent,
  selectDefaultUnderlying,
  sortUnderlyingReviews,
} from "./optionReviewViewModel.ts";

const underlying = (symbol: string, pnl: number, flags: string[] = []) => ({
  underlying: symbol,
  completed_campaigns: 1,
  active_campaigns: 0,
  gross_premium: 100,
  net_premium_pnl: pnl,
  retention_rate: 0.5,
  annualized_yield_on_notional: 0.05,
  worst_campaign_pnl: pnl,
  flags,
  campaigns: [],
});

test("underlyings sort by absolute net premium, then symbol", () => {
  const sorted = sortUnderlyingReviews([
    underlying("MSFT", 500, ["高留存"]),
    underlying("AAPL", -200, ["净亏损"]),
    underlying("NVDA", 900, ["低留存"]),
  ] as never);
  assert.deepEqual(sorted.map((row) => row.underlying), ["NVDA", "MSFT", "AAPL"]);
});

test("default selection uses the largest absolute net premium", () => {
  const report = { underlyings: [underlying("MSFT", 500), underlying("AAPL", -200, ["净亏损"])] } as never;
  assert.equal(selectDefaultUnderlying(report), "MSFT");
});

test("percentage formatter preserves negative values and missing state", () => {
  assert.equal(formatReviewPercent(-0.31), "-31.0%");
  assert.equal(formatReviewPercent(null), "—");
});
