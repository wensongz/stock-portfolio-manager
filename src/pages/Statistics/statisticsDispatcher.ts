import type { Currency } from "../../types";
import type {
  StatisticsRequestMode,
  StatisticsView,
} from "../../stores/statisticsStore";
import {
  resolveStatisticsView,
  type StatisticsSelection,
} from "./statisticsView.ts";
import type { AccountHoldingsCoverage } from "./statisticsAccountHoldings";

interface StatisticsDispatcherDependencies {
  getSelection: () => StatisticsSelection;
  updateSelection: (selection: StatisticsSelection) => void;
  fetchView: (
    view: StatisticsView,
    mode?: StatisticsRequestMode,
  ) => Promise<void>;
  fetchHoldingQuotes: (symbols?: [string, string][]) => Promise<unknown>;
  getAccountHoldings: (
    accountId: string,
    baseCurrency: Currency,
  ) => AccountHoldingsCoverage;
}

export function createStatisticsDispatcher({
  getSelection,
  updateSelection,
  fetchView,
  fetchHoldingQuotes,
  getAccountHoldings,
}: StatisticsDispatcherDependencies) {
  const requestSelection = (
    selection: StatisticsSelection,
    mode: StatisticsRequestMode = "join-in-flight",
  ) => {
    const view = resolveStatisticsView(selection);
    return view ? fetchView(view, mode) : Promise.resolve();
  };

  const refreshAccountQuotes = async (selection: StatisticsSelection) => {
    const coverage = getAccountHoldings(
      selection.selectedAccountId,
      selection.baseCurrency,
    );
    if (coverage.status === "unknown") {
      await fetchHoldingQuotes();
      return "all" as const;
    }
    if (coverage.status === "known-empty") return "account" as const;

    const seen = new Set<string>();
    const symbols: [string, string][] = [];
    for (const holding of coverage.holdings) {
      if (!seen.has(holding.symbol)) {
        seen.add(holding.symbol);
        symbols.push([holding.symbol, holding.market]);
      }
    }
    await fetchHoldingQuotes(symbols);
    return "account" as const;
  };

  const changeSelection = (
    patch: Partial<StatisticsSelection>,
    shouldRequest: (selection: StatisticsSelection) => boolean,
  ) => {
    const selection = { ...getSelection(), ...patch };
    updateSelection(selection);
    return shouldRequest(selection)
      ? requestSelection(selection)
      : Promise.resolve();
  };

  return {
    initialize: () => {
      const { baseCurrency } = getSelection();
      return fetchView({ kind: "overview", baseCurrency });
    },
    loadCurrentView: () => requestSelection(getSelection()),
    changeTab: (activeTab: StatisticsSelection["activeTab"]) =>
      changeSelection({ activeTab }, () => true),
    changeMarket: (selectedMarket: string) =>
      changeSelection(
        { selectedMarket },
        (selection) => selection.activeTab === "market",
      ),
    changeAccount: (selectedAccountId: string) =>
      changeSelection(
        { selectedAccountId },
        (selection) => selection.activeTab === "account",
      ),
    changeCategory: (selectedCategoryId: string) =>
      changeSelection(
        { selectedCategoryId },
        (selection) => selection.activeTab === "category",
      ),
    changeCurrency: (baseCurrency: Currency) =>
      changeSelection(
        { baseCurrency },
        (selection) =>
          selection.activeTab === "overview" ||
          selection.activeTab === "category",
      ),
    refresh: async () => {
      const refreshedAccountIds = new Set<string>();
      let hasFullCoverage = false;
      const initialSelection = getSelection();
      if (
        initialSelection.activeTab === "account" &&
        initialSelection.selectedAccountId
      ) {
        const coverage = await refreshAccountQuotes(initialSelection);
        if (coverage === "all") {
          hasFullCoverage = true;
        } else {
          refreshedAccountIds.add(initialSelection.selectedAccountId);
        }
      } else {
        await fetchHoldingQuotes();
        hasFullCoverage = true;
      }

      while (!hasFullCoverage) {
        const selection = getSelection();
        if (
          selection.activeTab === "account" &&
          selection.selectedAccountId &&
          refreshedAccountIds.has(selection.selectedAccountId)
        ) {
          break;
        }
        if (selection.activeTab === "account" && selection.selectedAccountId) {
          const coverage = await refreshAccountQuotes(selection);
          if (coverage === "all") {
            hasFullCoverage = true;
          } else {
            refreshedAccountIds.add(selection.selectedAccountId);
          }
        } else {
          await fetchHoldingQuotes();
          hasFullCoverage = true;
        }
      }

      await requestSelection(getSelection(), "reload-after-in-flight");
    },
  };
}
