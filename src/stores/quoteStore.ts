import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { message } from "antd";
import type { HoldingWithQuote, StockQuote } from "../types";

const DEFAULT_REFRESH_INTERVAL_MS = 5 * 60 * 1000; // 5 minutes
const STORAGE_KEY = "quote_refresh_interval_ms";

const MAX_REFRESH_INTERVAL_MS = 30 * 60 * 1000; // 30 minutes
const XUEQIU_COOKIE_WARNING = "雪球 Cookie 可能已经过期，请到设置页面更新雪球 Cookie。";

function isXueqiuCookieExpiredError(err: string): boolean {
  return (
    err.includes("Xueqiu API error") &&
    (err.includes("400016") || err.includes("重新登录帐号后再试") || err.includes("刷新页面或者重新登录帐号后再试"))
  );
}

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
  warning: null,
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
        warning: null,
        lastUpdatedAt: new Date().toISOString(),
      });
      const warning = await invoke<string | null>("take_quote_warning");
      if (warning) {
        message.warning(warning);
        set({ warning });
      }
    } catch (err) {
      const error = String(err);
      if (isXueqiuCookieExpiredError(error)) {
        message.warning(XUEQIU_COOKIE_WARNING);
        set({ error, warning: XUEQIU_COOKIE_WARNING, loading: false });
        return;
      }
      set({ error, warning: null, loading: false });
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
        warning: null,
        lastUpdatedAt: new Date().toISOString(),
      });
      const warning = await invoke<string | null>("take_quote_warning");
      if (warning) {
        message.warning(warning);
        set({ warning });
      }
    } catch (err) {
      const error = String(err);
      if (isXueqiuCookieExpiredError(error)) {
        message.warning(XUEQIU_COOKIE_WARNING);
        set({ error, warning: XUEQIU_COOKIE_WARNING, loading: false });
        return;
      }
      set({ error, warning: null, loading: false });
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
