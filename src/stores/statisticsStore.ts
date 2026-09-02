import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type {
  AccountStatistics,
  CategoryStatistics,
  Currency,
  MarketStatistics,
  StatisticsOverview,
} from "../types";

export type StatisticsView =
  | { kind: "overview"; baseCurrency: Currency }
  | { kind: "market"; market: string }
  | { kind: "account"; accountId: string }
  | { kind: "category"; categoryId: string; baseCurrency: Currency };

export function statisticsViewKey(view: StatisticsView): string {
  switch (view.kind) {
    case "overview":
      return `overview:${view.baseCurrency}`;
    case "market":
      return `market:${view.market}`;
    case "account":
      return `account:${view.accountId}`;
    case "category":
      return `category:${view.categoryId}:${view.baseCurrency}`;
  }
}

interface StatisticsState {
  overview: StatisticsOverview | null;
  marketStats: Record<string, MarketStatistics>;
  accountStats: Record<string, AccountStatistics>;
  categoryStats: Record<string, CategoryStatistics>;
  loadingByView: Record<string, boolean>;
  errorByView: Record<string, string | null>;
  fetchView: (view: StatisticsView) => Promise<void>;
}

export type StatisticsInvoke = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export const createStatisticsStore = (invokeFn: StatisticsInvoke = invoke) =>
  create<StatisticsState>((set) => ({
    overview: null,
    marketStats: {},
    accountStats: {},
    categoryStats: {},
    loadingByView: {},
    errorByView: {},

    fetchView: async (view) => {
      const viewKey = statisticsViewKey(view);
      set((state) => ({
        loadingByView: { ...state.loadingByView, [viewKey]: true },
        errorByView: { ...state.errorByView, [viewKey]: null },
      }));

      try {
        switch (view.kind) {
          case "overview": {
            const overview = await invokeFn<StatisticsOverview>(
              "get_statistics_overview",
              { baseCurrency: view.baseCurrency },
            );
            set((state) => ({
              overview,
              loadingByView: { ...state.loadingByView, [viewKey]: false },
            }));
            return;
          }
          case "market": {
            const statistics = await invokeFn<MarketStatistics>(
              "get_statistics_by_market",
              { market: view.market },
            );
            set((state) => ({
              marketStats: {
                ...state.marketStats,
                [view.market]: statistics,
              },
              loadingByView: { ...state.loadingByView, [viewKey]: false },
            }));
            return;
          }
          case "account": {
            const statistics = await invokeFn<AccountStatistics>(
              "get_statistics_by_account",
              { accountId: view.accountId },
            );
            set((state) => ({
              accountStats: {
                ...state.accountStats,
                [view.accountId]: statistics,
              },
              loadingByView: { ...state.loadingByView, [viewKey]: false },
            }));
            return;
          }
          case "category": {
            const statistics = await invokeFn<CategoryStatistics>(
              "get_statistics_by_category",
              {
                categoryId: view.categoryId,
                baseCurrency: view.baseCurrency,
              },
            );
            set((state) => ({
              categoryStats: {
                ...state.categoryStats,
                [`${view.categoryId}:${view.baseCurrency}`]: statistics,
              },
              loadingByView: { ...state.loadingByView, [viewKey]: false },
            }));
            return;
          }
        }
      } catch (error) {
        set((state) => ({
          loadingByView: { ...state.loadingByView, [viewKey]: false },
          errorByView: {
            ...state.errorByView,
            [viewKey]: String(error),
          },
        }));
      }
    },
  }));

export const useStatisticsStore = createStatisticsStore();
