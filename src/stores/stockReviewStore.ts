import { invoke } from "@tauri-apps/api/core";
import { create } from "zustand";
import type {
  StockCampaignDetail,
  StockReviewAnnotation,
  StockReviewAnnotationInput,
  StockReviewFilters,
  StockReviewOverrideInput,
  StockReviewReport,
} from "../types";

type StockReviewErrorSource = "report" | "campaign" | "mutation";

interface StockReviewState {
  filters: StockReviewFilters | null;
  reportLoading: boolean;
  campaignLoading: boolean;
  mutating: boolean;
  report: StockReviewReport | null;
  selectedCampaign: StockCampaignDetail | null;
  error: string | null;
  errorSource: StockReviewErrorSource | null;
  loadReport: (filters: StockReviewFilters) => Promise<void>;
  loadCampaignDetail: (
    filters: StockReviewFilters,
    campaignId: string,
  ) => Promise<void>;
  saveAnnotation: (
    input: StockReviewAnnotationInput,
  ) => Promise<StockReviewAnnotation | null>;
  confirmOverride: (
    filters: StockReviewFilters,
    input: StockReviewOverrideInput,
  ) => Promise<StockReviewReport | null>;
  clearSelectedCampaign: () => void;
  clearError: () => void;
}

let latestReportRequestId = 0;
let latestCampaignRequestId = 0;
let latestMutationRequestId = 0;
let pendingMutations = 0;

function queryArguments(filters: StockReviewFilters) {
  return {
    startDate: filters.startDate,
    endDate: filters.endDate,
    accountId: filters.accountId,
    market: filters.market,
    benchmarkSymbol: filters.benchmarkSymbol,
    baseCurrency: filters.baseCurrency,
  };
}

function replaceAnnotation(
  annotations: StockReviewAnnotation[],
  annotation: StockReviewAnnotation,
): StockReviewAnnotation[] {
  const existingIndex = annotations.findIndex((item) => item.id === annotation.id);
  if (existingIndex < 0) return [...annotations, annotation];
  return annotations.map((item, index) =>
    index === existingIndex ? annotation : item,
  );
}

function normalizedIdentity(value: string | null): string | null {
  const normalized = value?.trim().toUpperCase() ?? "";
  return normalized || null;
}

function annotationAppliesToCampaign(
  annotation: StockReviewAnnotation,
  campaign: StockCampaignDetail,
): boolean {
  const { summary } = campaign;
  if (annotation.scope_type === "period") return true;
  if (annotation.scope_type === "campaign") {
    return annotation.scope_key === summary.campaign_id;
  }
  if (annotation.scope_type === "action") {
    return summary.action_ids.includes(annotation.scope_key);
  }
  if (annotation.scope_type !== "stock") return false;

  const annotationSymbol = normalizedIdentity(annotation.symbol ?? annotation.scope_key);
  if (annotationSymbol !== normalizedIdentity(summary.symbol)) return false;
  return (
    annotation.account_id == null || summary.account_ids.includes(annotation.account_id)
  );
}

function clearMatchingError(
  state: Pick<StockReviewState, "error" | "errorSource">,
  source: StockReviewErrorSource,
) {
  return state.errorSource === source
    ? { error: null, errorSource: null }
    : { error: state.error, errorSource: state.errorSource };
}

export const useStockReviewStore = create<StockReviewState>((set) => ({
  filters: null,
  reportLoading: false,
  campaignLoading: false,
  mutating: false,
  report: null,
  selectedCampaign: null,
  error: null,
  errorSource: null,

  loadReport: async (filters) => {
    const requestId = ++latestReportRequestId;
    set((state) => ({
      filters,
      reportLoading: true,
      ...clearMatchingError(state, "report"),
    }));
    try {
      const report = await invoke<StockReviewReport>(
        "get_stock_review_report",
        queryArguments(filters),
      );
      if (requestId !== latestReportRequestId) return;
      set((state) => ({
        report,
        reportLoading: false,
        ...clearMatchingError(state, "report"),
      }));
    } catch (error) {
      if (requestId !== latestReportRequestId) return;
      set({
        reportLoading: false,
        error: String(error),
        errorSource: "report",
      });
    }
  },

  loadCampaignDetail: async (filters, campaignId) => {
    const requestId = ++latestCampaignRequestId;
    set((state) => ({
      campaignLoading: true,
      ...clearMatchingError(state, "campaign"),
    }));
    try {
      const detail = await invoke<StockCampaignDetail>(
        "get_stock_campaign_detail",
        {
          ...queryArguments(filters),
          campaignId,
        },
      );
      if (requestId !== latestCampaignRequestId) return;
      set((state) => ({
        selectedCampaign: detail,
        campaignLoading: false,
        ...clearMatchingError(state, "campaign"),
      }));
    } catch (error) {
      if (requestId !== latestCampaignRequestId) return;
      set({
        campaignLoading: false,
        error: String(error),
        errorSource: "campaign",
      });
    }
  },

  saveAnnotation: async (input) => {
    const requestId = ++latestMutationRequestId;
    const reportContextId = latestReportRequestId;
    const campaignContextId = latestCampaignRequestId;
    pendingMutations += 1;
    set((state) => ({
      mutating: true,
      ...clearMatchingError(state, "mutation"),
    }));
    try {
      const annotation = await invoke<StockReviewAnnotation>(
        "save_stock_review_annotation",
        { input },
      );
      if (
        requestId === latestMutationRequestId &&
        reportContextId === latestReportRequestId
      ) {
        set((state) => ({
          report: state.report
            ? {
                ...state.report,
                annotations: replaceAnnotation(state.report.annotations, annotation),
              }
            : null,
          selectedCampaign:
            campaignContextId === latestCampaignRequestId &&
            state.selectedCampaign &&
            annotationAppliesToCampaign(annotation, state.selectedCampaign)
              ? {
                  ...state.selectedCampaign,
                  annotations: replaceAnnotation(
                    state.selectedCampaign.annotations,
                    annotation,
                  ),
                }
              : state.selectedCampaign,
          ...clearMatchingError(state, "mutation"),
        }));
      }
      return annotation;
    } catch (error) {
      if (
        requestId === latestMutationRequestId &&
        reportContextId === latestReportRequestId
      ) {
        set({ error: String(error), errorSource: "mutation" });
      }
      return null;
    } finally {
      pendingMutations = Math.max(0, pendingMutations - 1);
      set({ mutating: pendingMutations > 0 });
    }
  },

  confirmOverride: async (filters, input) => {
    const requestId = ++latestMutationRequestId;
    const reportRequestId = ++latestReportRequestId;
    pendingMutations += 1;
    set((state) => ({
      mutating: true,
      reportLoading: false,
      ...clearMatchingError(state, "mutation"),
    }));
    try {
      const report = await invoke<StockReviewReport>(
        "confirm_stock_review_override",
        {
          ...queryArguments(filters),
          input,
        },
      );
      if (
        requestId === latestMutationRequestId &&
        reportRequestId === latestReportRequestId
      ) {
        latestCampaignRequestId += 1;
        set((state) => ({
          filters,
          report,
          selectedCampaign: null,
          campaignLoading: false,
          ...clearMatchingError(state, "mutation"),
        }));
      }
      return report;
    } catch (error) {
      if (
        requestId === latestMutationRequestId &&
        reportRequestId === latestReportRequestId
      ) {
        set({ error: String(error), errorSource: "mutation" });
      }
      return null;
    } finally {
      pendingMutations = Math.max(0, pendingMutations - 1);
      set({ mutating: pendingMutations > 0 });
    }
  },

  clearSelectedCampaign: () => {
    latestCampaignRequestId += 1;
    set((state) => ({
      selectedCampaign: null,
      campaignLoading: false,
      ...clearMatchingError(state, "campaign"),
    }));
  },

  clearError: () => set({ error: null, errorSource: null }),
}));
