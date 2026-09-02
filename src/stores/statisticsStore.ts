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
  overviewByCurrency: Partial<Record<Currency, StatisticsOverview>>;
  marketStats: Record<string, MarketStatistics>;
  accountStats: Record<string, AccountStatistics>;
  categoryStats: Record<string, CategoryStatistics>;
  resultRevisionByView: Record<string, number>;
  loadingByView: Record<string, boolean>;
  errorByView: Record<string, string | null>;
  fetchView: (
    view: StatisticsView,
    mode?: StatisticsRequestMode,
  ) => Promise<void>;
}

export type StatisticsRequestMode =
  | "join-in-flight"
  | "reload-after-in-flight";

export type StatisticsInvoke = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export const createStatisticsStore = (invokeFn: StatisticsInvoke = invoke) => {
  type RequestEntry = { token: symbol; promise: Promise<void> };

  const inFlight = new Map<string, RequestEntry>();
  const queuedReloads = new Map<string, RequestEntry>();
  const latestTokenByView = new Map<string, symbol>();
  let nextRequestRevision = 0;

  return create<StatisticsState>((set) => {
    const setLoading = (viewKey: string) => {
      set((state) => ({
        loadingByView: { ...state.loadingByView, [viewKey]: true },
        errorByView: { ...state.errorByView, [viewKey]: null },
      }));
    };

    const startRequest = (
      view: StatisticsView,
      viewKey: string,
      token: symbol,
      makeLatest: boolean,
    ): Promise<void> => {
      const requestRevision = ++nextRequestRevision;
      if (makeLatest) latestTokenByView.set(viewKey, token);
      if (latestTokenByView.get(viewKey) === token) setLoading(viewKey);
      const request = (async () => {
        try {
          switch (view.kind) {
            case "overview": {
              const overview = await invokeFn<StatisticsOverview>(
                "get_statistics_overview",
                { baseCurrency: view.baseCurrency },
              );
              set((state) =>
                latestTokenByView.get(viewKey) === token
                  ? {
                      overviewByCurrency: {
                        ...state.overviewByCurrency,
                        [view.baseCurrency]: overview,
                      },
                      resultRevisionByView: {
                        ...state.resultRevisionByView,
                        [viewKey]: requestRevision,
                      },
                      loadingByView: {
                        ...state.loadingByView,
                        [viewKey]: false,
                      },
                    }
                  : state,
              );
              return;
            }
            case "market": {
              const statistics = await invokeFn<MarketStatistics>(
                "get_statistics_by_market",
                { market: view.market },
              );
              set((state) =>
                latestTokenByView.get(viewKey) === token
                  ? {
                      marketStats: {
                        ...state.marketStats,
                        [view.market]: statistics,
                      },
                      resultRevisionByView: {
                        ...state.resultRevisionByView,
                        [viewKey]: requestRevision,
                      },
                      loadingByView: {
                        ...state.loadingByView,
                        [viewKey]: false,
                      },
                    }
                  : state,
              );
              return;
            }
            case "account": {
              const statistics = await invokeFn<AccountStatistics>(
                "get_statistics_by_account",
                { accountId: view.accountId },
              );
              set((state) =>
                latestTokenByView.get(viewKey) === token
                  ? {
                      accountStats: {
                        ...state.accountStats,
                        [view.accountId]: statistics,
                      },
                      resultRevisionByView: {
                        ...state.resultRevisionByView,
                        [viewKey]: requestRevision,
                      },
                      loadingByView: {
                        ...state.loadingByView,
                        [viewKey]: false,
                      },
                    }
                  : state,
              );
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
              set((state) =>
                latestTokenByView.get(viewKey) === token
                  ? {
                      categoryStats: {
                        ...state.categoryStats,
                        [`${view.categoryId}:${view.baseCurrency}`]: statistics,
                      },
                      resultRevisionByView: {
                        ...state.resultRevisionByView,
                        [viewKey]: requestRevision,
                      },
                      loadingByView: {
                        ...state.loadingByView,
                        [viewKey]: false,
                      },
                    }
                  : state,
              );
              return;
            }
          }
        } catch (error) {
          if (latestTokenByView.get(viewKey) === token) {
            set((state) => ({
              loadingByView: { ...state.loadingByView, [viewKey]: false },
              errorByView: {
                ...state.errorByView,
                [viewKey]: String(error),
              },
            }));
          }
        }
      })();
      const entry = { token, promise: request };
      inFlight.set(viewKey, entry);
      void request.finally(() => {
        if (inFlight.get(viewKey) === entry) {
          inFlight.delete(viewKey);
        }
      });
      return request;
    };

    return {
      overviewByCurrency: {},
      marketStats: {},
      accountStats: {},
      categoryStats: {},
      resultRevisionByView: {},
      loadingByView: {},
      errorByView: {},

      fetchView: (
        view,
        mode: StatisticsRequestMode = "join-in-flight",
      ) => {
        const viewKey = statisticsViewKey(view);
        const queued = queuedReloads.get(viewKey);
        if (queued) {
          latestTokenByView.set(viewKey, queued.token);
          setLoading(viewKey);
          return queued.promise;
        }

        const existing = inFlight.get(viewKey);
        if (existing && mode === "join-in-flight") {
          latestTokenByView.set(viewKey, existing.token);
          setLoading(viewKey);
          return existing.promise;
        }

        if (existing) {
          const token = Symbol(viewKey);
          latestTokenByView.set(viewKey, token);
          setLoading(viewKey);
          const promise = existing.promise.then(() => {
            if (queuedReloads.get(viewKey)?.token === token) {
              queuedReloads.delete(viewKey);
            }
            return startRequest(view, viewKey, token, false);
          });
          const entry = { token, promise };
          queuedReloads.set(viewKey, entry);
          void promise.finally(() => {
            if (queuedReloads.get(viewKey) === entry) {
              queuedReloads.delete(viewKey);
            }
          });
          return promise;
        }

        return startRequest(view, viewKey, Symbol(viewKey), true);
      },
    };
  });
};

export const useStatisticsStore = createStatisticsStore();
