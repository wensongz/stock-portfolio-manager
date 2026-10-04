import type { Currency, QuarterComparison, QuarterlySnapshot, QuarterlyTrends } from "../../types";
import { convertSnapshotValue, parseSnapshotExchangeRates } from "./aggregateSnapshotHoldings.ts";

const unavailableConversion = Symbol("unavailable quarterly conversion");

function requireAmount(value: number | null): number {
  if (value === null || !Number.isFinite(value)) throw unavailableConversion;
  return value;
}

function snapshotConverter(snapshots: QuarterlySnapshot[], quarter: string, currency: Currency) {
  if (currency === "USD") return (value: number) => value;
  const snapshot = snapshots.find((candidate) => candidate.quarter === quarter);
  if (!snapshot) return null;
  const rates = parseSnapshotExchangeRates(snapshot.exchange_rates);
  const probe = convertSnapshotValue(1, "USD", currency, rates);
  if (probe === null) return null;
  return (value: number) => convertSnapshotValue(value, "USD", currency, rates);
}

function comparisonPercent(before: number, after: number) {
  return before !== 0 ? (after - before) / before * 100 : 0;
}

export function convertQuarterComparison(
  comparison: QuarterComparison,
  snapshots: QuarterlySnapshot[],
  currency: Currency,
): QuarterComparison | null {
  if (currency === "USD") return comparison;
  const q1 = snapshotConverter(snapshots, comparison.quarter1, currency);
  const q2 = snapshotConverter(snapshots, comparison.quarter2, currency);
  if (!q1 || !q2) return null;
  const one = (value: number) => requireAmount(q1(value));
  const two = (value: number) => requireAmount(q2(value));
  try {
    const overview = comparison.overview;
    const q1TotalValue = one(overview.q1_total_value);
    const q2TotalValue = two(overview.q2_total_value);
    const convertRow = <T extends { q1_value: number; q2_value: number; q1_cost: number; q2_cost: number; q1_pnl: number; q2_pnl: number; value_change: number; value_change_percent: number }>(row: T): T => {
      const q1Value = one(row.q1_value);
      const q2Value = two(row.q2_value);
      return {
        ...row,
        q1_value: q1Value,
        q2_value: q2Value,
        q1_cost: one(row.q1_cost),
        q2_cost: two(row.q2_cost),
        q1_pnl: one(row.q1_pnl),
        q2_pnl: two(row.q2_pnl),
        value_change: q2Value - q1Value,
        value_change_percent: comparisonPercent(q1Value, q2Value),
      };
    };
    return {
      ...comparison,
      overview: {
        ...overview,
        q1_total_value: q1TotalValue,
        q2_total_value: q2TotalValue,
        value_change: q2TotalValue - q1TotalValue,
        value_change_percent: comparisonPercent(q1TotalValue, q2TotalValue),
        q1_total_cost: one(overview.q1_total_cost),
        q2_total_cost: two(overview.q2_total_cost),
        q1_pnl: one(overview.q1_pnl),
        q2_pnl: two(overview.q2_pnl),
      },
      by_market: comparison.by_market.map(convertRow),
      by_category: comparison.by_category.map(convertRow),
    };
  } catch (error) {
    if (error === unavailableConversion) return null;
    throw error;
  }
}

export function convertQuarterlyTrends(
  trends: QuarterlyTrends,
  snapshots: QuarterlySnapshot[],
  currency: Currency,
): QuarterlyTrends | null {
  if (currency === "USD") return trends;
  const converters = trends.quarters.map((quarter) => snapshotConverter(snapshots, quarter, currency));
  if (converters.some((converter) => !converter)) return null;
  const values = (source: number[]) => source.map((value, index) => requireAmount(converters[index]?.(value) ?? null));
  const groups = (source: Record<string, number[]>) => Object.fromEntries(
    Object.entries(source).map(([key, amounts]) => [key, values(amounts)]),
  );
  try {
    return {
      ...trends,
      total_values: values(trends.total_values),
      total_costs: values(trends.total_costs),
      total_pnls: values(trends.total_pnls),
      market_values: groups(trends.market_values),
      category_values: groups(trends.category_values),
    };
  } catch (error) {
    if (error === unavailableConversion) return null;
    throw error;
  }
}
