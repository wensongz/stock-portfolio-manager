import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type {
  StockOperationReviewFilters,
  StockOperationReviewReport,
} from "../types";

interface StockOperationReviewState {
  report: StockOperationReviewReport | null;
  loading: boolean;
  error: string | null;
  loadReport(filters: StockOperationReviewFilters): Promise<void>;
  clearError(): void;
}

let latestRequest = 0;

export const useStockOperationReviewStore = create<StockOperationReviewState>((set) => ({
  report: null,
  loading: false,
  error: null,
  loadReport: async (filters) => {
    const requestId = ++latestRequest;
    set({ loading: true, error: null });
    try {
      const report = await invoke<StockOperationReviewReport>(
        "get_stock_operation_review",
        {
          startDate: filters.startDate,
          endDate: filters.endDate,
          accountId: filters.accountId,
          market: filters.market,
          baseCurrency: filters.baseCurrency,
        },
      );
      if (requestId === latestRequest) set({ report, loading: false, error: null });
    } catch (error) {
      if (requestId === latestRequest) set({ loading: false, error: String(error) });
    }
  },
  clearError: () => set({ error: null }),
}));
