import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import type { HoldingWithQuote } from "../types";

interface QuoteState {
  holdingQuotes: HoldingWithQuote[];
  loading: boolean;
  // quoteWarning holds a Xueqiu error message to be shown in the UI.
  // It is set immediately after every quote fetch that touches the upstream
  // API (by calling the take_quote_warning Tauri command) and via the
  // quote-warning Tauri event emitted by the backend's background refresh task.
  quoteWarning: string | null;
  lastUpdatedAt: string | null;
  fetchHoldingQuotes: (refreshSymbols?: [string, string][]) => Promise<void>;
  setQuoteWarning: (w: string | null) => void;
  startQuoteSync: () => () => void;
}

export const useQuoteStore = create<QuoteState>((set, get) => ({
  // NOTE: lastUpdatedAt is initially null. Module-level code below fires an
  // async call to load the persisted timestamp from the backend DB so the UI
  // shows the correct value even on first render / after a restart.
  holdingQuotes: [],
  loading: false,
  quoteWarning: null,
  lastUpdatedAt: null,

  fetchHoldingQuotes: async (refreshSymbols?: [string, string][]) => {
    // A "refresh fetch" is one that hits the upstream API: either a full
    // refresh (no refreshSymbols arg) or a targeted refresh (non-empty list).
    // A cache-only call passes an empty array and does NOT touch the API.
    const isRefreshFetch = refreshSymbols === undefined || refreshSymbols.length > 0;
    // Clear any stale warning at the start of a refresh so the UI doesn't
    // keep showing it while the new fetch is in-flight.
    set({ loading: true, ...(isRefreshFetch ? { quoteWarning: null } : {}) });
    try {
      const holdingQuotes = await invoke<HoldingWithQuote[]>("get_holding_quotes", {
        ...(refreshSymbols !== undefined ? { refreshSymbols } : {}),
      });
      // Read any Xueqiu warning produced during this fetch. This is the most
      // reliable delivery path for user-triggered refreshes: the warning is
      // checked immediately after the invoke that may have produced it, with
      // no timing ambiguity and no dependency on startup-only Tauri events.
      const qw = await invoke<string | null>("take_quote_warning").catch(() => null);
      set({
        holdingQuotes,
        loading: false,
        lastUpdatedAt: new Date().toISOString(),
        // For refresh fetches: always update quoteWarning (null = no issue,
        // which clears a previously-shown warning). For cache-only fetches:
        // only write if there IS a new warning so we don't accidentally clear
        // a warning that was already being displayed.
        ...(isRefreshFetch ? { quoteWarning: qw } : qw ? { quoteWarning: qw } : {}),
      });
    } catch {
      set({ loading: false });
    }
  },

  setQuoteWarning: (w) => set({ quoteWarning: w }),

  startQuoteSync: () => {
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

// Load the persisted last quote refresh time from the database at module
// initialization time so any page (dashboard, statistics, holdings) sees the
// correct timestamp immediately after a restart, even if startQuoteSync has
// not been called yet. Runs once when this module is first imported.
invoke<string | null>("get_last_quote_refresh_time")
  .then((ts) => {
    if (ts) useQuoteStore.setState({ lastUpdatedAt: ts });
  })
  .catch(() => {
    // Ignore errors – the UI shows no timestamp until the first refresh.
  });
