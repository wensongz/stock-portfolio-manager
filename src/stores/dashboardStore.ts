import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  DashboardSummary,
  HoldingDetail,
  StatisticsOverview,
  MarketStatistics,
  AccountStatistics,
  CategoryStatistics,
} from "../types";

interface DashboardState {
  summary: DashboardSummary | null;
  holdingDetails: HoldingDetail[];
  loadingSummary: boolean;
  loadingHoldings: boolean;
  errorSummary: string | null;
  errorHoldings: string | null;
  fetchSummary: (baseCurrency?: string) => Promise<void>;
  fetchHoldingDetails: () => Promise<void>;
}

interface StatisticsState {
  overview: StatisticsOverview | null;
  marketStats: Record<string, MarketStatistics>;
  accountStats: Record<string, AccountStatistics>;
  categoryStats: Record<string, CategoryStatistics>;
  loadingOverview: boolean;
  errorOverview: string | null;
  fetchOverview: (baseCurrency?: string) => Promise<void>;
  fetchMarketStats: (market: string) => Promise<void>;
  fetchAccountStats: (accountId: string) => Promise<void>;
  fetchCategoryStats: (categoryId: string, baseCurrency?: string) => Promise<void>;
}

export const useDashboardStore = create<DashboardState>((set) => ({
  summary: null,
  holdingDetails: [],
  loadingSummary: false,
  loadingHoldings: false,
  errorSummary: null,
  errorHoldings: null,

  fetchSummary: async (baseCurrency) => {
    set({ loadingSummary: true, errorSummary: null });
    try {
      const summary = await invoke<DashboardSummary>("get_dashboard_summary", {
        baseCurrency: baseCurrency ?? null,
      });
      set({ summary, loadingSummary: false });
    } catch (err) {
      set({ errorSummary: String(err), loadingSummary: false });
    }
  },

  fetchHoldingDetails: async () => {
    set({ loadingHoldings: true, errorHoldings: null });
    try {
      const holdingDetails = await invoke<HoldingDetail[]>("get_holdings_with_quotes");
      set({ holdingDetails, loadingHoldings: false });
    } catch (err) {
      set({ errorHoldings: String(err), loadingHoldings: false });
    }
  },
}));

export const useStatisticsStore = create<StatisticsState>((set) => ({
  overview: null,
  marketStats: {},
  accountStats: {},
  categoryStats: {},
  loadingOverview: false,
  errorOverview: null,

  fetchOverview: async (baseCurrency?: string) => {
    set({ loadingOverview: true, errorOverview: null });
    try {
      const overview = await invoke<StatisticsOverview>("get_statistics_overview", {
        baseCurrency: baseCurrency ?? null,
      });
      set({ overview, loadingOverview: false });
    } catch (err) {
      set({ errorOverview: String(err), loadingOverview: false });
    }
  },

  fetchMarketStats: async (market: string) => {
    try {
      const stats = await invoke<MarketStatistics>("get_statistics_by_market", { market });
      set((state) => ({ marketStats: { ...state.marketStats, [market]: stats } }));
    } catch (err) {
      console.error("fetchMarketStats error:", err);
    }
  },

  fetchAccountStats: async (accountId: string) => {
    try {
      const stats = await invoke<AccountStatistics>("get_statistics_by_account", { accountId });
      set((state) => ({ accountStats: { ...state.accountStats, [accountId]: stats } }));
    } catch (err) {
      console.error("fetchAccountStats error:", err);
    }
  },

  fetchCategoryStats: async (categoryId: string, baseCurrency?: string) => {
    try {
      const stats = await invoke<CategoryStatistics>("get_statistics_by_category", {
        categoryId,
        baseCurrency: baseCurrency ?? null,
      });
      set((state) => ({ categoryStats: { ...state.categoryStats, [categoryId]: stats } }));
    } catch (err) {
      console.error("fetchCategoryStats error:", err);
    }
  },
}));
