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
import {
  createStockReviewAnnotationDisplayContext,
  doesStockReviewAnnotationApplyToCampaign,
  isStockReviewAnnotationInDisplayContext,
} from "../pages/Review/stockReviewViewModel.ts";

type StockReviewErrorSource = "report" | "campaign" | "annotation" | "override";

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
let latestAnnotationRequestId = 0;
let latestOverrideRequestId = 0;
let latestFilterGeneration = 0;
let pendingMutations = 0;
const latestAnnotationRequestById = new Map<string, number>();

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

function filtersEqual(
  left: StockReviewFilters | null,
  right: StockReviewFilters,
): boolean {
  return (
    left != null &&
    left.accountId === right.accountId &&
    left.periodPreset === right.periodPreset &&
    left.startDate === right.startDate &&
    left.endDate === right.endDate &&
    left.market === right.market &&
    left.benchmarkSymbol === right.benchmarkSymbol &&
    left.baseCurrency === right.baseCurrency
  );
}

function mergeVisibleAnnotations(
  report: StockReviewReport,
  currentAnnotations: StockReviewAnnotation[],
): StockReviewReport {
  if (currentAnnotations.length === 0) {
    return report;
  }

  const context = createStockReviewAnnotationDisplayContext(report);
  let annotations = [...report.annotations];
  for (const annotation of currentAnnotations) {
    if (isStockReviewAnnotationInDisplayContext(annotation, context)) {
      annotations = replaceAnnotation(annotations, annotation);
    }
  }
  return annotations.length === report.annotations.length &&
    annotations.every((annotation, index) => annotation === report.annotations[index])
    ? report
    : { ...report, annotations };
}

function clearRelevantError(
  state: Pick<StockReviewState, "error" | "errorSource">,
  sources: StockReviewErrorSource[],
) {
  return state.errorSource != null && sources.includes(state.errorSource)
    ? { error: null, errorSource: null }
    : { error: state.error, errorSource: state.errorSource };
}

export const useStockReviewStore = create<StockReviewState>((set, get) => ({
  filters: null,
  reportLoading: false,
  campaignLoading: false,
  mutating: false,
  report: null,
  selectedCampaign: null,
  error: null,
  errorSource: null,

  loadReport: async (filters) => {
    if (!filtersEqual(get().filters, filters)) latestFilterGeneration += 1;
    const filterGeneration = latestFilterGeneration;
    const requestId = ++latestReportRequestId;
    latestCampaignRequestId += 1;
    set((state) => ({
      filters,
      reportLoading: true,
      campaignLoading: false,
      selectedCampaign: null,
      ...clearRelevantError(state, ["report", "campaign"]),
    }));
    try {
      const report = await invoke<StockReviewReport>(
        "get_stock_review_report",
        queryArguments(filters),
      );
      if (
        requestId !== latestReportRequestId ||
        filterGeneration !== latestFilterGeneration
      ) {
        return;
      }
      set((state) => ({
        report,
        reportLoading: false,
        ...clearRelevantError(state, ["report"]),
      }));
    } catch (error) {
      if (
        requestId !== latestReportRequestId ||
        filterGeneration !== latestFilterGeneration
      ) {
        return;
      }
      set({
        reportLoading: false,
        error: String(error),
        errorSource: "report",
      });
    }
  },

  loadCampaignDetail: async (filters, campaignId) => {
    const requestId = ++latestCampaignRequestId;
    const reportRequestId = latestReportRequestId;
    const filterGeneration = latestFilterGeneration;
    set((state) => ({
      campaignLoading: true,
      ...clearRelevantError(state, ["campaign"]),
    }));
    try {
      const detail = await invoke<StockCampaignDetail>(
        "get_stock_campaign_detail",
        {
          ...queryArguments(filters),
          campaignId,
        },
      );
      if (
        requestId !== latestCampaignRequestId ||
        reportRequestId !== latestReportRequestId ||
        filterGeneration !== latestFilterGeneration
      ) {
        return;
      }
      set((state) => ({
        selectedCampaign: detail,
        campaignLoading: false,
        ...clearRelevantError(state, ["campaign"]),
      }));
    } catch (error) {
      if (
        requestId !== latestCampaignRequestId ||
        reportRequestId !== latestReportRequestId ||
        filterGeneration !== latestFilterGeneration
      ) {
        return;
      }
      set({
        campaignLoading: false,
        error: String(error),
        errorSource: "campaign",
      });
    }
  },

  saveAnnotation: async (input) => {
    const requestId = ++latestAnnotationRequestId;
    latestAnnotationRequestById.set(input.id, requestId);
    const filterGeneration = latestFilterGeneration;
    pendingMutations += 1;
    set((state) => ({
      mutating: true,
      ...clearRelevantError(state, ["annotation"]),
    }));
    try {
      const annotation = await invoke<StockReviewAnnotation>(
        "save_stock_review_annotation",
        { input },
      );
      if (
        latestAnnotationRequestById.get(input.id) === requestId &&
        filterGeneration === latestFilterGeneration
      ) {
        set((state) => {
          const context = state.report
            ? createStockReviewAnnotationDisplayContext(state.report)
            : null;
          const reportVisible = Boolean(
            context && isStockReviewAnnotationInDisplayContext(annotation, context),
          );
          const campaignVisible = Boolean(
            context &&
              reportVisible &&
              state.selectedCampaign &&
              doesStockReviewAnnotationApplyToCampaign(
                annotation,
                context,
                state.selectedCampaign.summary.campaign_id,
              ),
          );
          return {
            report:
              state.report && reportVisible
                ? {
                    ...state.report,
                    annotations: replaceAnnotation(
                      state.report.annotations,
                      annotation,
                    ),
                  }
                : state.report,
            selectedCampaign:
              state.selectedCampaign && campaignVisible
                ? {
                    ...state.selectedCampaign,
                    annotations: replaceAnnotation(
                      state.selectedCampaign.annotations,
                      annotation,
                    ),
                  }
                : state.selectedCampaign,
            ...(requestId === latestAnnotationRequestId
              ? clearRelevantError(state, ["annotation"])
              : { error: state.error, errorSource: state.errorSource }),
          };
        });
      }
      return annotation;
    } catch (error) {
      if (
        requestId === latestAnnotationRequestId &&
        filterGeneration === latestFilterGeneration
      ) {
        set({ error: String(error), errorSource: "annotation" });
      }
      return null;
    } finally {
      if (latestAnnotationRequestById.get(input.id) === requestId) {
        latestAnnotationRequestById.delete(input.id);
      }
      pendingMutations = Math.max(0, pendingMutations - 1);
      set({ mutating: pendingMutations > 0 });
    }
  },

  confirmOverride: async (filters, input) => {
    if (!filtersEqual(get().filters, filters)) latestFilterGeneration += 1;
    const filterGeneration = latestFilterGeneration;
    const requestId = ++latestOverrideRequestId;
    const reportRequestId = ++latestReportRequestId;
    latestCampaignRequestId += 1;
    pendingMutations += 1;
    set((state) => ({
      filters,
      mutating: true,
      reportLoading: false,
      campaignLoading: false,
      selectedCampaign: null,
      ...clearRelevantError(state, ["override", "campaign"]),
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
        requestId === latestOverrideRequestId &&
        reportRequestId === latestReportRequestId &&
        filterGeneration === latestFilterGeneration
      ) {
        set((state) => ({
          filters,
          report: mergeVisibleAnnotations(
            report,
            state.report?.annotations ?? [],
          ),
          ...clearRelevantError(state, ["override"]),
        }));
      }
      return report;
    } catch (error) {
      if (
        requestId === latestOverrideRequestId &&
        reportRequestId === latestReportRequestId &&
        filterGeneration === latestFilterGeneration
      ) {
        set({ error: String(error), errorSource: "override" });
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
      ...clearRelevantError(state, ["campaign"]),
    }));
  },

  clearError: () => set({ error: null, errorSource: null }),
}));
