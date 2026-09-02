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
  resultRevisionByView: Record<string, number>;
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
  const accountRevision =
    state.resultRevisionByView[`account:${accountId}`];
  let newestSource =
    accountStatistics && accountRevision != null
      ? {
          revision: accountRevision,
          holdings: accountStatistics.holdings,
        }
      : undefined;

  const currencies: Currency[] = [
    currency,
    ...(["USD", "CNY", "HKD"] as Currency[]).filter(
      (candidate) => candidate !== currency,
    ),
  ];
  for (const candidateCurrency of currencies) {
    const overview = state.overviewByCurrency[candidateCurrency];
    const overviewRevision =
      state.resultRevisionByView[`overview:${candidateCurrency}`];
    if (
      overview &&
      overviewRevision != null &&
      (!newestSource || overviewRevision > newestSource.revision)
    ) {
      newestSource = {
        revision: overviewRevision,
        holdings: overview.holdings.filter(
          (holding) => holding.account_id === accountId,
        ),
      };
    }
  }

  const holdings = newestSource?.holdings;

  if (!holdings) return { status: "unknown" };
  if (holdings.length === 0) return { status: "known-empty" };
  return { status: "known-with-symbols", holdings };
}
