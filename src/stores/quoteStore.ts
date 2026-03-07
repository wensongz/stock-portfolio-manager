import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { HoldingWithQuote, StockQuote } from "../types";

const DEFAULT_REFRESH_INTERVAL_MS = 30_000; // 30 seconds

interface QuoteState {
  holdingQuotes: HoldingWithQuote[];
  quotes: Record<string, StockQuote>;
  loading: boolean;
  error: string | null;
  lastUpdatedAt: string | null;
  refreshIntervalMs: number;
  fetchHoldingQuotes: () => Promise<void>;
  fetchQuotes: (symbols: [string, string][]) => Promise<void>;
  setRefreshInterval: (ms: number) => void;
  startAutoRefresh: () => () => void;
}

export const useQuoteStore = create<QuoteState>((set, get) => ({
  holdingQuotes: [],
  quotes: {},
  loading: false,
  error: null,
  lastUpdatedAt: null,
  refreshIntervalMs: DEFAULT_REFRESH_INTERVAL_MS,

  fetchHoldingQuotes: async () => {
    set({ loading: true, error: null });
    try {
      const holdingQuotes = await invoke<HoldingWithQuote[]>("get_holding_quotes");
      const quotes: Record<string, StockQuote> = {};
      holdingQuotes.forEach((h) => {
        if (h.quote) {
          quotes[h.symbol] = h.quote;
        }
      });
      set({
        holdingQuotes,
        quotes,
        loading: false,
        lastUpdatedAt: new Date().toISOString(),
      });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  fetchQuotes: async (symbols: [string, string][]) => {
    set({ loading: true, error: null });
    try {
      const quoteList = await invoke<StockQuote[]>("get_real_time_quotes", { symbols });
      const quotes: Record<string, StockQuote> = { ...get().quotes };
      quoteList.forEach((q) => {
        quotes[q.symbol] = q;
      });
      set({
        quotes,
        loading: false,
        lastUpdatedAt: new Date().toISOString(),
      });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  setRefreshInterval: (ms: number) => {
    set({ refreshIntervalMs: ms });
  },

  startAutoRefresh: () => {
    const { fetchHoldingQuotes, refreshIntervalMs } = get();
    fetchHoldingQuotes();
    const id = setInterval(() => {
      get().fetchHoldingQuotes();
    }, refreshIntervalMs);
    return () => clearInterval(id);
  },
}));
