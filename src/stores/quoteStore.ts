import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { HoldingWithQuote, StockQuote } from "../types";

const DEFAULT_REFRESH_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes
const STORAGE_KEY = "quote_refresh_interval_ms";

const MAX_REFRESH_INTERVAL_MS = 30 * 60 * 1000; // 30 minutes

function loadRefreshInterval(): number {
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved) {
      const parsed = Number(saved);
      if (!isNaN(parsed) && parsed > 0 && parsed <= MAX_REFRESH_INTERVAL_MS) return parsed;
    }
  } catch {
    // ignore
  }
  return DEFAULT_REFRESH_INTERVAL_MS;
}

interface QuoteState {
  holdingQuotes: HoldingWithQuote[];
  quotes: Record<string, StockQuote>;
  loading: boolean;
  error: string | null;
  lastUpdatedAt: string | null;
  refreshIntervalMs: number;
  fetchHoldingQuotes: (refreshSymbols?: [string, string][]) => Promise<void>;
  fetchQuotes: (symbols: [string, string][], forceRefresh?: boolean) => Promise<void>;
  setRefreshInterval: (ms: number) => void;
  startAutoRefresh: () => () => void;
}

export const useQuoteStore = create<QuoteState>((set, get) => ({
  holdingQuotes: [],
  quotes: {},
  loading: false,
  error: null,
  lastUpdatedAt: null,
  refreshIntervalMs: loadRefreshInterval(),

  fetchHoldingQuotes: async (refreshSymbols?: [string, string][]) => {
    set({ loading: true, error: null });
    try {
      const holdingQuotes = await invoke<HoldingWithQuote[]>("get_holding_quotes", {
        ...(refreshSymbols !== undefined ? { refreshSymbols } : {}),
      });
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

  fetchQuotes: async (symbols: [string, string][], forceRefresh?: boolean) => {
    set({ loading: true, error: null });
    try {
      const quoteList = await invoke<StockQuote[]>("get_real_time_quotes", {
        symbols,
        forceRefresh: forceRefresh ?? false,
      });
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
    try {
      localStorage.setItem(STORAGE_KEY, String(ms));
    } catch {
      // ignore
    }
    set({ refreshIntervalMs: ms });
  },

  startAutoRefresh: () => {
    const { fetchHoldingQuotes, refreshIntervalMs } = get();
    // First call with empty list: loads holdings with DB-cached quotes instantly
    // (no API calls), then follow up with a full refresh from the API.
    fetchHoldingQuotes([]).then(() => {
      fetchHoldingQuotes();
    });
    const id = setInterval(() => {
      get().fetchHoldingQuotes();
    }, refreshIntervalMs);
    return () => clearInterval(id);
  },
}));
