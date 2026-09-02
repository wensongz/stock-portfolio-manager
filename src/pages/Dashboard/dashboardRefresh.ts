import type { Currency } from "../../types";

interface DashboardRefreshDependencies {
  fetchHoldingQuotes: () => Promise<unknown>;
  fetchReport: (baseCurrency: Currency) => Promise<void>;
  getBaseCurrency: () => Currency;
}

export async function refreshDashboardQuotes({
  fetchHoldingQuotes,
  fetchReport,
  getBaseCurrency,
}: DashboardRefreshDependencies): Promise<void> {
  await fetchHoldingQuotes();
  await fetchReport(getBaseCurrency());
}
