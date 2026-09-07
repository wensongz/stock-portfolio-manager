import type { ExchangeRates, PieSlice, QuarterlyHoldingSnapshot } from "../../types";

type SnapshotRates = Partial<Pick<ExchangeRates, "usd_cny" | "usd_hkd">>;

export interface AggregatedSnapshotHolding extends QuarterlyHoldingSnapshot {
  market_value_base: number | null;
  accountRows: QuarterlyHoldingSnapshot[];
}

export function parseSnapshotExchangeRates(value?: string): SnapshotRates | null {
  if (!value) return null;
  try {
    const parsed: unknown = JSON.parse(value);
    if (!parsed || typeof parsed !== "object" || Array.isArray(parsed)) return null;
    const rates: SnapshotRates = {};
    for (const pair of ["usd_cny", "usd_hkd"] as const) {
      const rate = (parsed as Record<string, unknown>)[pair];
      if (rate == null) continue;
      if (typeof rate !== "number" || !Number.isFinite(rate) || rate <= 0) return null;
      rates[pair] = rate;
    }
    if (rates.usd_cny !== undefined || rates.usd_hkd !== undefined) return rates;
  } catch {
    // Keep native amounts visible without inventing historical exchange rates.
  }
  return null;
}

function marketCurrency(market?: string): string {
  return market === "CN" ? "CNY" : market === "HK" ? "HKD" : "USD";
}

export function snapshotHoldingCurrency(holding: QuarterlyHoldingSnapshot): string {
  const currency = holding.currency?.trim().toUpperCase();
  if (currency) return currency;
  const cashCurrency = holding.symbol.toUpperCase().match(/^\$CASH-(USD|CNY|HKD)$/)?.[1];
  return cashCurrency ?? marketCurrency(holding.market);
}

function convertSnapshotValue(value: number, from: string, to: string, rates: SnapshotRates | null): number | null {
  if (value === 0 || from === to) return value;
  if (!rates) return null;
  const perUsd: Record<string, number | undefined> = { USD: 1, CNY: rates.usd_cny, HKD: rates.usd_hkd };
  const sourceRate = perUsd[from];
  const targetRate = perUsd[to];
  if (typeof sourceRate !== "number" || !Number.isFinite(sourceRate) || sourceRate <= 0
    || typeof targetRate !== "number" || !Number.isFinite(targetRate) || targetRate <= 0) return null;
  const converted = value / sourceRate * targetRate;
  return Number.isFinite(converted) ? converted : null;
}

export function aggregateSnapshotHoldings(
  holdings: QuarterlyHoldingSnapshot[],
  rates: SnapshotRates | null = null,
): AggregatedSnapshotHolding[] {
  const grouped = new Map<string, QuarterlyHoldingSnapshot[]>();

  for (const holding of holdings) {
    const key = JSON.stringify([holding.market, holding.symbol.toUpperCase(), snapshotHoldingCurrency(holding)]);
    const rows = grouped.get(key);
    if (rows) rows.push(holding);
    else grouped.set(key, [holding]);
  }

  return Array.from(grouped.values())
    .map((accountRows) => {
      const first = accountRows[0];
      const currency = snapshotHoldingCurrency(first);
      const shares = accountRows.reduce((sum, row) => sum + row.shares, 0);
      const costValue = accountRows.reduce((sum, row) => sum + row.cost_value, 0);
      const marketValue = accountRows.reduce((sum, row) => sum + row.market_value, 0);
      const pnl = accountRows.reduce((sum, row) => sum + row.pnl, 0);

      return {
        ...first,
        id: JSON.stringify([first.market, first.symbol.toUpperCase(), currency]),
        currency,
        account_id: "",
        account_name: "",
        shares,
        avg_cost: shares !== 0 ? costValue / shares : 0,
        market_value: marketValue,
        cost_value: costValue,
        pnl,
        pnl_percent: costValue > 0 ? (pnl / costValue) * 100 : null,
        weight: accountRows.reduce((sum, row) => sum + row.weight, 0),
        notes: accountRows.find((row) => row.notes)?.notes ?? null,
        market_value_base: convertSnapshotValue(marketValue, currency, "USD", rates),
        accountRows: [...accountRows].sort((a, b) => b.market_value - a.market_value),
      };
    })
    .sort((a, b) => (b.market_value_base ?? -Infinity) - (a.market_value_base ?? -Infinity));
}

export function buildSnapshotComposition(
  holdings: QuarterlyHoldingSnapshot[],
  rates: SnapshotRates | null,
  market?: string,
): { currency: string; categories: PieSlice[]; pieSlices: PieSlice[]; total: number | null; hasNegativeValues: boolean; hasMissingRates: boolean } {
  const currency = marketCurrency(market);
  const subset = market ? holdings.filter((holding) => holding.market === market) : holdings;
  const grouped = new Map<string, PieSlice>();
  const hasNegativeValues = subset.some((holding) => holding.market_value < 0);
  let hasMissingRates = false;
  for (const holding of subset) {
    const value = convertSnapshotValue(holding.market_value, snapshotHoldingCurrency(holding), currency, rates);
    if (value === null) {
      hasMissingRates = true;
      continue;
    }
    const name = holding.category_name || "未分类";
    const previous = grouped.get(name);
    grouped.set(name, { name, value: (previous?.value ?? 0) + value, color: holding.category_color || "#999" });
  }
  const categoryOrder = ["现金类", "分红股", "成长股", "套利"];
  const order = (name: string) => categoryOrder.includes(name) ? categoryOrder.indexOf(name) : categoryOrder.length;
  const categories = hasMissingRates ? [] : [...grouped.values()].sort((a, b) => order(a.name) - order(b.name));
  const total = hasMissingRates ? null : categories.reduce((sum, category) => sum + category.value, 0);
  const pieSlices = total !== null && total > 0 && !hasNegativeValues ? categories.filter((category) => category.value > 0) : [];
  return { currency, categories, pieSlices, total, hasNegativeValues, hasMissingRates };
}
