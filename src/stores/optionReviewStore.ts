import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type { OptionReviewReport } from "../types";

interface OptionReviewState {
  report: OptionReviewReport | null;
  loading: boolean;
  error: string | null;
  requestedAccountId: string | null;
  requestedPeriodDays: number | null;
  fetchOptionReview: (accountId: string, periodDays: number | null) => Promise<void>;
  clearOptionReview: () => void;
}

let latestOptionReviewRequest = 0;

export const useOptionReviewStore = create<OptionReviewState>((set) => ({
  report: null,
  loading: false,
  error: null,
  requestedAccountId: null,
  requestedPeriodDays: null,
  fetchOptionReview: async (accountId, periodDays) => {
    const requestId = ++latestOptionReviewRequest;
    set({
      report: null,
      loading: true,
      error: null,
      requestedAccountId: accountId,
      requestedPeriodDays: periodDays,
    });
    try {
      const report = await invoke<OptionReviewReport>("get_option_review", {
        accountId,
        periodDays: periodDays ?? null,
      });
      if (requestId === latestOptionReviewRequest) set({ report, loading: false });
    } catch (error) {
      if (requestId === latestOptionReviewRequest) {
        set({ report: null, loading: false, error: String(error) });
      }
    }
  },
  clearOptionReview: () => {
    latestOptionReviewRequest += 1;
    set({
      report: null,
      error: null,
      loading: false,
      requestedAccountId: null,
      requestedPeriodDays: null,
    });
  },
}));
