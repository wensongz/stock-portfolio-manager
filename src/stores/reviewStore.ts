import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { DecisionStatistics, HoldingReview } from "../types";

interface ReviewState {
  reviewedSymbols: [string, string, string][]; // [symbol, name, market]
  currentReview: HoldingReview | null;
  decisionStats: DecisionStatistics | null;
  loading: boolean;
  error: string | null;

  fetchReviewedSymbols: () => Promise<void>;
  fetchHoldingReview: (symbol: string) => Promise<void>;
  fetchDecisionStatistics: () => Promise<void>;
  updateDecisionQuality: (
    snapshotId: string,
    symbol: string,
    quality: string
  ) => Promise<void>;
  clearCurrentReview: () => void;
}

export const useReviewStore = create<ReviewState>((set, get) => ({
  reviewedSymbols: [],
  currentReview: null,
  decisionStats: null,
  loading: false,
  error: null,

  fetchReviewedSymbols: async () => {
    set({ loading: true, error: null });
    try {
      const reviewedSymbols = await invoke<[string, string, string][]>(
        "get_reviewed_symbols"
      );
      set({ reviewedSymbols, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  fetchHoldingReview: async (symbol: string) => {
    set({ loading: true, error: null });
    try {
      const currentReview = await invoke<HoldingReview>("get_holding_review", {
        symbol,
      });
      set({ currentReview, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  fetchDecisionStatistics: async () => {
    try {
      const decisionStats =
        await invoke<DecisionStatistics>("get_decision_statistics");
      set({ decisionStats });
    } catch (err) {
      console.error("fetchDecisionStatistics error:", err);
    }
  },

  updateDecisionQuality: async (
    snapshotId: string,
    symbol: string,
    quality: string
  ) => {
    try {
      await invoke<boolean>("update_decision_quality", {
        snapshotId,
        symbol,
        quality,
      });
      // Refresh current review if it matches
      const { currentReview } = get();
      if (currentReview?.symbol === symbol) {
        await get().fetchHoldingReview(symbol);
      }
      await get().fetchDecisionStatistics();
    } catch (err) {
      set({ error: String(err) });
    }
  },

  clearCurrentReview: () => set({ currentReview: null }),
}));
