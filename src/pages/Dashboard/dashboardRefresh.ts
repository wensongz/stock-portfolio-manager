import type { DashboardRequestMode } from "../../stores/dashboardStore";
import type { Currency } from "../../types";

interface DashboardRefreshDependencies {
  fetchHoldingQuotes: () => Promise<unknown>;
  fetchReport: (
    baseCurrency: Currency,
    mode?: DashboardRequestMode,
  ) => Promise<void>;
  getBaseCurrency: () => Currency;
}

export async function refreshDashboardQuotes({
  fetchHoldingQuotes,
  fetchReport,
  getBaseCurrency,
}: DashboardRefreshDependencies): Promise<void> {
  await fetchHoldingQuotes();
  await fetchReport(getBaseCurrency(), "reload-after-in-flight");
}
