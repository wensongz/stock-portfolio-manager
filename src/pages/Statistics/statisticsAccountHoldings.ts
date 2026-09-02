import type {
  AccountStatistics,
  Currency,
  StatisticsOverview,
} from "../../types";

export interface QuoteRefreshHolding {
  symbol: string;
  market: string;
}

interface AccountHoldingsState {
  accountStats: Record<
    string,
    Pick<AccountStatistics, "holdings"> | undefined
  >;
  overviewByCurrency: Partial<
    Record<Currency, Pick<StatisticsOverview, "holdings">>
  >;
}

export type AccountHoldingsCoverage =
  | { status: "unknown" }
  | { status: "known-empty" }
  | { status: "known-with-symbols"; holdings: QuoteRefreshHolding[] };

export function resolveAccountHoldingsCoverage(
  state: AccountHoldingsState,
  accountId: string,
  currency: Currency,
): AccountHoldingsCoverage {
  const accountStatistics = state.accountStats[accountId];
  const overview = state.overviewByCurrency[currency];
  const holdings = accountStatistics
    ? accountStatistics.holdings
    : overview?.holdings.filter(
        (holding) => holding.account_id === accountId,
      );

  if (!holdings) return { status: "unknown" };
  if (holdings.length === 0) return { status: "known-empty" };
  return { status: "known-with-symbols", holdings };
}
