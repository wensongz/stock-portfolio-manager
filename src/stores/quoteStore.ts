import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { HoldingWithQuote, QuoteCommandResult } from "../types";
import { toQuoteWarning } from "./quoteErrors.ts";

type QuoteListener = (
  event: "quotes-refreshed",
  handler: (event: { payload: QuoteCommandResult<HoldingWithQuote[]> }) => void,
) => Promise<() => void>;

export interface QuoteState {
  holdingQuotes: HoldingWithQuote[];
  loading: boolean;
  quoteWarning: string | null;
  lastUpdatedAt: string | null;
  fetchHoldingQuotes: (refreshSymbols?: [string, string][]) => Promise<void>;
  applyQuoteMetadata: (
    outcome: Pick<QuoteCommandResult<unknown>, "warning" | "refreshedAt">,
  ) => void;
  setQuoteWarning: (warning: string | null) => void;
  startQuoteSync: () => () => void;
}

function isOlderTimestamp(candidate: string | null, current: string | null) {
  if (!candidate || !current) return false;
  const candidateTime = Date.parse(candidate);
  const currentTime = Date.parse(current);
  return Number.isFinite(candidateTime)
    && Number.isFinite(currentTime)
    && candidateTime < currentTime;
}

export const createQuoteStore = (
  invokeFn: typeof invoke = invoke,
  listenFn: QuoteListener = (event, handler) => listen(event, handler),
) => {
  let latestRequest = 0;

  return create<QuoteState>((set, get) => ({
    holdingQuotes: [],
    loading: false,
    quoteWarning: null,
    lastUpdatedAt: null,

    fetchHoldingQuotes: async (refreshSymbols) => {
      const request = ++latestRequest;
      const isRefresh = refreshSymbols === undefined || refreshSymbols.length > 0;
      set({ loading: true, ...(isRefresh ? { quoteWarning: null } : {}) });
      try {
        const outcome = await invokeFn<QuoteCommandResult<HoldingWithQuote[]>>(
          "get_holding_quotes",
          refreshSymbols !== undefined ? { refreshSymbols } : undefined,
        );
        if (request !== latestRequest) return;
        set((state) => {
          if (isOlderTimestamp(outcome.refreshedAt, state.lastUpdatedAt)) {
            return { loading: false };
          }
          return {
            holdingQuotes: outcome.data,
            loading: false,
            lastUpdatedAt: outcome.refreshedAt,
            ...(isRefresh
              ? { quoteWarning: outcome.warning }
              : outcome.warning
                ? { quoteWarning: outcome.warning }
                : {}),
          };
        });
      } catch (error) {
        if (request === latestRequest) {
          set({ loading: false, quoteWarning: toQuoteWarning(error) });
        }
      }
    },

    applyQuoteMetadata: (outcome) => {
      set((state) => isOlderTimestamp(outcome.refreshedAt, state.lastUpdatedAt)
        ? state
        : {
            quoteWarning: outcome.warning,
            lastUpdatedAt: outcome.refreshedAt,
          });
    },

    setQuoteWarning: (warning) => set({ quoteWarning: warning }),

    startQuoteSync: () => {
      let unlistenFn: (() => void) | null = null;
      let cancelled = false;

      void listenFn("quotes-refreshed", (event) => {
        const outcome = event.payload;
        set((state) => isOlderTimestamp(outcome.refreshedAt, state.lastUpdatedAt)
          ? state
          : {
              holdingQuotes: outcome.data,
              quoteWarning: outcome.warning,
              lastUpdatedAt: outcome.refreshedAt,
            });
      }).then((unlisten) => {
        if (cancelled) {
          unlisten();
          return;
        }
        unlistenFn = unlisten;
        void get().fetchHoldingQuotes([]);
      });

      return () => {
        cancelled = true;
        unlistenFn?.();
      };
    },
  }));
};

export const useQuoteStore = createQuoteStore();
