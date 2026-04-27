import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
  warning: string | null;
  // quoteWarning holds a Xueqiu error message to be shown in the UI.
  // It is set immediately after every quote fetch that touches the upstream
  // API (by calling the take_quote_warning Tauri command) and via the
  // quote-warning Tauri event emitted by the backend's background refresh task.
  quoteWarning: string | null;
  lastUpdatedAt: string | null;
  refreshIntervalMs: number;
  fetchHoldingQuotes: (refreshSymbols?: [string, string][]) => Promise<void>;
  fetchQuotes: (symbols: [string, string][], forceRefresh?: boolean) => Promise<void>;
  setQuoteWarning: (w: string | null) => void;
  setRefreshInterval: (ms: number) => void;
  startAutoRefresh: () => () => void;
}

export const useQuoteStore = create<QuoteState>((set, get) => ({
  holdingQuotes: [],
  quotes: {},
  loading: false,
  error: null,
  warning: null,
  quoteWarning: null,
  lastUpdatedAt: null,
  refreshIntervalMs: loadRefreshInterval(),

  fetchHoldingQuotes: async (refreshSymbols?: [string, string][]) => {
    // A "refresh fetch" is one that hits the upstream API: either a full
    // refresh (no refreshSymbols arg) or a targeted refresh (non-empty list).
    // A cache-only call passes an empty array and does NOT touch the API.
    const isRefreshFetch = refreshSymbols === undefined || refreshSymbols.length > 0;
    // Clear any stale warning at the start of a refresh so the UI doesn't
    // keep showing the previous error while the new fetch is in-flight.
    set({ loading: true, error: null, ...(isRefreshFetch ? { quoteWarning: null } : {}) });
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
      // Read any Xueqiu warning produced during this fetch. This is the most
      // reliable delivery path for user-triggered refreshes: the warning is
      // checked immediately after the invoke that may have produced it, with
      // no timing ambiguity and no dependency on startup-only Tauri events.
      const qw = await invoke<string | null>("take_quote_warning").catch(() => null);
      set({
        holdingQuotes,
        quotes,
        loading: false,
        warning: null,
        lastUpdatedAt: new Date().toISOString(),
        // For refresh fetches: always update quoteWarning (null = no issue,
        // which clears a previously-shown warning). For cache-only fetches:
        // only write if there IS a new warning so we don't accidentally clear
        // a warning that was already being displayed.
        ...(isRefreshFetch ? { quoteWarning: qw } : qw ? { quoteWarning: qw } : {}),
      });
    } catch (err) {
      set({ error: String(err), warning: null, loading: false });
    }
  },

  fetchQuotes: async (symbols: [string, string][], forceRefresh?: boolean) => {
    set({ loading: true, error: null, ...(forceRefresh ? { quoteWarning: null } : {}) });
    try {
      const quoteList = await invoke<StockQuote[]>("get_real_time_quotes", {
        symbols,
        forceRefresh: forceRefresh ?? false,
      });
      const quotes: Record<string, StockQuote> = { ...get().quotes };
      quoteList.forEach((q) => {
        quotes[q.symbol] = q;
      });
      const qw = await invoke<string | null>("take_quote_warning").catch(() => null);
      set({
        quotes,
        loading: false,
        warning: null,
        lastUpdatedAt: new Date().toISOString(),
        ...(forceRefresh ? { quoteWarning: qw } : qw ? { quoteWarning: qw } : {}),
      });
    } catch (err) {
      set({ error: String(err), warning: null, loading: false });
    }
  },

  setQuoteWarning: (w) => set({ quoteWarning: w }),

  setRefreshInterval: (ms: number) => {
    try {
      localStorage.setItem(STORAGE_KEY, String(ms));
    } catch {
      // ignore
    }
    set({ refreshIntervalMs: ms });
  },

  startAutoRefresh: () => {
    const { fetchHoldingQuotes } = get();
    // Load holdings with DB-cached quotes only (no API calls).
    // The backend spawns a background task on startup to refresh the cache
    // from upstream APIs and emits a "quotes-refreshed" event when done.
    fetchHoldingQuotes([]);

    // Listen for the backend "quotes-refreshed" event so the UI picks up
    // freshly-updated prices without a manual refresh.
    let unlistenFn: (() => void) | null = null;
    let cancelled = false;

    listen("quotes-refreshed", () => {
      useQuoteStore.getState().fetchHoldingQuotes([]);
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlistenFn = fn;
      }
    });

    // No periodic auto-refresh – quotes are only refreshed when the user
    // explicitly clicks the refresh button.
    return () => {
      cancelled = true;
      if (unlistenFn) unlistenFn();
    };
  },
}));
