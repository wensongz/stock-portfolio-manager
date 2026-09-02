import type { Currency } from "../../types";
import type { StatisticsView } from "../../stores/statisticsStore";

export interface StatisticsSelection {
  activeTab: "overview" | "market" | "account" | "category";
  baseCurrency: Currency;
  selectedMarket: string;
  selectedAccountId: string;
  selectedCategoryId: string;
}

export function resolveStatisticsView(
  selection: StatisticsSelection,
): StatisticsView | null {
  switch (selection.activeTab) {
    case "overview":
      return { kind: "overview", baseCurrency: selection.baseCurrency };
    case "market":
      return { kind: "market", market: selection.selectedMarket };
    case "account":
      return selection.selectedAccountId
        ? { kind: "account", accountId: selection.selectedAccountId }
        : null;
    case "category":
      return selection.selectedCategoryId
        ? {
            kind: "category",
            categoryId: selection.selectedCategoryId,
            baseCurrency: selection.baseCurrency,
          }
        : null;
  }
}
