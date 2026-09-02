import type { Currency } from "../../types";
import type {
  StatisticsRequestMode,
  StatisticsView,
} from "../../stores/statisticsStore";
import {
  resolveStatisticsView,
  type StatisticsSelection,
} from "./statisticsView.ts";

interface RefreshHolding {
  symbol: string;
  market: string;
}

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
  ) => RefreshHolding[];
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

  const accountSymbols = (selection: StatisticsSelection) => {
    const seen = new Set<string>();
    const symbols: [string, string][] = [];
    for (const holding of getAccountHoldings(
      selection.selectedAccountId,
      selection.baseCurrency,
    )) {
      if (!seen.has(holding.symbol)) {
        seen.add(holding.symbol);
        symbols.push([holding.symbol, holding.market]);
      }
    }
    return symbols;
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
        await fetchHoldingQuotes(accountSymbols(initialSelection));
        refreshedAccountIds.add(initialSelection.selectedAccountId);
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
          await fetchHoldingQuotes(accountSymbols(selection));
          refreshedAccountIds.add(selection.selectedAccountId);
        } else {
          await fetchHoldingQuotes();
          hasFullCoverage = true;
        }
      }

      await requestSelection(getSelection(), "reload-after-in-flight");
    },
  };
}
