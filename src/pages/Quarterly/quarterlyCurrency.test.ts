import test from "node:test";
import assert from "node:assert/strict";
import type { QuarterComparison, QuarterlySnapshot, QuarterlyTrends } from "../../types";
import { convertQuarterComparison, convertQuarterlyTrends } from "./quarterlyCurrency.ts";

const snapshots = [
  { quarter: "2025-Q1", exchange_rates: '{"usd_cny":7,"usd_hkd":7.8}' },
  { quarter: "2025-Q2", exchange_rates: '{"usd_cny":8,"usd_hkd":7.9}' },
] as QuarterlySnapshot[];
const comparison = {
  quarter1: "2025-Q1", quarter2: "2025-Q2",
  overview: { q1_total_value: 100, q2_total_value: 120, value_change: 20, value_change_percent: 20, q1_total_cost: 80, q2_total_cost: 90, q1_pnl: 20, q2_pnl: 30, q1_holding_count: 1, q2_holding_count: 2 },
  by_market: [{ market: "US", q1_value: 100, q2_value: 120, value_change: 20, value_change_percent: 20, q1_cost: 80, q2_cost: 90, q1_pnl: 20, q2_pnl: 30 }],
  by_category: [{ category_name: "股票", category_color: "red", q1_value: 100, q2_value: 120, value_change: 20, value_change_percent: 20, q1_cost: 80, q2_cost: 90, q1_pnl: 20, q2_pnl: 30 }],
  holding_changes: { new_holdings: [], closed_holdings: [], increased: [], decreased: [], unchanged: [] },
} satisfies QuarterComparison;
const trends = { quarters: ["2025-Q1", "2025-Q2"], total_values: [100, 120], total_costs: [80, 90], total_pnls: [20, 30], market_values: { US: [100, 120] }, category_values: { 股票: [100, 120] }, holding_counts: [1, 2] } satisfies QuarterlyTrends;

test("comparison applies each quarter's saved rate and recomputes changes without mutating source", () => {
  const result = convertQuarterComparison(comparison, snapshots, "CNY");
  assert.ok(result);
  assert.equal(result.overview.q1_total_value, 700);
  assert.equal(result.overview.q2_total_value, 960);
  assert.equal(result.overview.value_change, 260);
  assert.equal(result.overview.value_change_percent, 260 / 700 * 100);
  assert.equal(result.by_market[0].q1_pnl, 140);
  assert.equal(result.by_category[0].q2_cost, 720);
  assert.equal(comparison.overview.value_change, 20);
  assert.equal(comparison.by_market[0].q2_value, 120);
});

test("trends converts signed values at each historical quarter rate", () => {
  const source = { ...trends, total_pnls: [-20, 30] };
  const result = convertQuarterlyTrends(source, snapshots, "CNY");
  assert.ok(result);
  assert.deepEqual(result.total_values, [700, 960]);
  assert.deepEqual(result.total_pnls, [-140, 240]);
  assert.deepEqual(result.market_values.US, [700, 960]);
  assert.deepEqual(result.holding_counts, [1, 2]);
  assert.deepEqual(source.total_pnls, [-20, 30]);
});

test("USD works without saved rates while target currency requires them", () => {
  const missing = [{ quarter: "2025-Q1", exchange_rates: "{}" }, snapshots[1]] as QuarterlySnapshot[];
  assert.deepEqual(convertQuarterComparison(comparison, missing, "USD"), comparison);
  assert.deepEqual(convertQuarterlyTrends(trends, missing, "USD"), trends);
  assert.equal(convertQuarterComparison(comparison, missing, "CNY"), null);
  assert.equal(convertQuarterlyTrends(trends, missing, "CNY"), null);
  assert.equal(convertQuarterComparison(comparison, snapshots.slice(1), "CNY"), null);
  const invalid = [{ ...snapshots[0], exchange_rates: '{"usd_cny":-7}' }, snapshots[1]];
  assert.equal(convertQuarterlyTrends(trends, invalid, "CNY"), null);
});

test("unrepresentable converted amounts are unavailable instead of leaking null into chart data", () => {
  const hugeComparison = { ...comparison, overview: { ...comparison.overview, q2_total_value: 1e308 } };
  const hugeTrends = { ...trends, total_values: [100, 1e308] };
  assert.equal(convertQuarterComparison(hugeComparison, snapshots, "CNY"), null);
  assert.equal(convertQuarterlyTrends(hugeTrends, snapshots, "CNY"), null);
});
