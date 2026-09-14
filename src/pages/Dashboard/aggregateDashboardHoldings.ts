import type { HoldingDetail } from "../../types/index.ts";

export interface DashboardAccountHolding extends HoldingDetail {
  key: string;
  position_pct: number;
}

export interface DashboardStockHolding extends Omit<DashboardAccountHolding, "id" | "account_id" | "account_name"> {
  daily_change_percent: number | null;
  accountRows: DashboardAccountHolding[];
}

export function filterDashboardHoldings(
  holdings: readonly HoldingDetail[], accountIds: readonly string[], markets: readonly string[],
): HoldingDetail[] {
  return holdings.filter((holding) =>
    (accountIds.length === 0 || accountIds.includes(holding.account_id))
    && (markets.length === 0 || markets.includes(holding.market)),
  );
}

function addHolding(target: HoldingDetail, holding: HoldingDetail) {
  target.shares += holding.shares;
  target.cost_value += holding.cost_value;
  target.market_value += holding.market_value;
  target.market_value_usd += holding.market_value_usd;
  target.pnl += holding.pnl;
  target.daily_pnl += holding.daily_pnl;
}

function totals(holding: HoldingDetail, totalUsd: number) {
  return {
    avg_cost: holding.shares > 0 ? holding.cost_value / holding.shares : 0,
    pnl_percent: holding.cost_value > 0 ? (holding.pnl / holding.cost_value) * 100 : null,
    position_pct: totalUsd > 0 ? (holding.market_value_usd / totalUsd) * 100 : 0,
  };
}

export function aggregateDashboardHoldings(holdings: readonly HoldingDetail[]): DashboardStockHolding[] {
  const groups = new Map<string, { holding: HoldingDetail; accounts: Map<string, HoldingDetail> }>();
  let totalUsd = 0;

  for (const holding of holdings) {
    if (holding.shares <= 0) continue;
    const symbol = holding.symbol.trim().toUpperCase();
    // Cash of the same currency can live in accounts from different markets.
    // Securities retain market/currency identity so native amounts stay comparable.
    const key = JSON.stringify([
      symbol.startsWith("$CASH-") ? "CASH" : holding.market.trim().toUpperCase(),
      symbol, holding.currency,
    ]);
    totalUsd += holding.market_value_usd;
    const existing = groups.get(key);
    if (!existing) {
      groups.set(key, {
        holding: { ...holding, symbol },
        accounts: new Map([[holding.account_id, { ...holding, symbol }]]),
      });
      continue;
    }
    addHolding(existing.holding, holding);
    const account = existing.accounts.get(holding.account_id);
    if (account) addHolding(account, holding);
    else existing.accounts.set(holding.account_id, { ...holding, symbol });
  }

  return Array.from(groups, ([key, { holding, accounts }]) => ({
    ...holding,
    ...totals(holding, totalUsd),
    key,
    daily_change_percent: !holding.symbol.startsWith("$CASH-")
      && holding.daily_change_percent != null && Number.isFinite(holding.daily_change_percent)
      ? holding.daily_change_percent : null,
    accountRows: Array.from(accounts, ([accountId, account]) => ({
      ...account,
      ...totals(account, totalUsd),
      key: JSON.stringify([key, accountId]),
    })).sort((a, b) => b.market_value_usd - a.market_value_usd),
  })).sort((a, b) => b.market_value_usd - a.market_value_usd);
}
