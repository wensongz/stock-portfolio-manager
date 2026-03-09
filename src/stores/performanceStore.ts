import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  DrawdownAnalysis,
  HoldingPerformance,
  MonthlyReturn,
  PerformanceSummary,
  ReturnAttribution,
  ReturnDataPoint,
  RiskMetrics,
} from "../types";
import dayjs from "dayjs";

export type TimeRange =
  | "1W"
  | "1M"
  | "3M"
  | "6M"
  | "YTD"
  | "1Y"
  | "3Y"
  | "5Y"
  | "ALL"
  | "CUSTOM";

export const BENCHMARK_SYMBOLS = [
  { label: "🇺🇸 S&P 500", value: "^GSPC" },
  { label: "🇺🇸 NASDAQ", value: "^IXIC" },
  { label: "🇨🇳 沪深300", value: "000300.SS" },
  { label: "🇨🇳 上证指数", value: "000001.SS" },
  { label: "🇭🇰 恒生指数", value: "^HSI" },
];

function getDateRange(range: TimeRange): { start: string; end: string } {
  const end = dayjs().format("YYYY-MM-DD");
  let start: string;
  switch (range) {
    case "1W":
      start = dayjs().subtract(7, "day").format("YYYY-MM-DD");
      break;
    case "1M":
      start = dayjs().subtract(1, "month").format("YYYY-MM-DD");
      break;
    case "3M":
      start = dayjs().subtract(3, "month").format("YYYY-MM-DD");
      break;
    case "6M":
      start = dayjs().subtract(6, "month").format("YYYY-MM-DD");
      break;
    case "YTD":
      start = dayjs().startOf("year").format("YYYY-MM-DD");
      break;
    case "1Y":
      start = dayjs().subtract(1, "year").format("YYYY-MM-DD");
      break;
    case "3Y":
      start = dayjs().subtract(3, "year").format("YYYY-MM-DD");
      break;
    case "5Y":
      start = dayjs().subtract(5, "year").format("YYYY-MM-DD");
      break;
    case "ALL":
    default:
      start = "2000-01-01";
      break;
  }
  return { start, end };
}

interface PerformanceState {
  timeRange: TimeRange;
  customStart: string | null;
  customEnd: string | null;
  selectedBenchmarks: string[];

  summary: PerformanceSummary | null;
  returnSeries: ReturnDataPoint[];
  benchmarkSeries: Record<string, ReturnDataPoint[]>;
  drawdown: DrawdownAnalysis | null;
  attribution: ReturnAttribution | null;
  monthlyReturns: MonthlyReturn[];
  topGainers: HoldingPerformance[];
  topLosers: HoldingPerformance[];
  riskMetrics: RiskMetrics | null;

  loading: boolean;
  error: string | null;

  setTimeRange: (range: TimeRange, start?: string, end?: string) => void;
  setBenchmarks: (symbols: string[]) => void;
  fetchAll: () => Promise<void>;
  fetchBenchmark: (symbol: string) => Promise<void>;
}

export const usePerformanceStore = create<PerformanceState>((set, get) => ({
  timeRange: "1Y",
  customStart: null,
  customEnd: null,
  selectedBenchmarks: [],

  summary: null,
  returnSeries: [],
  benchmarkSeries: {},
  drawdown: null,
  attribution: null,
  monthlyReturns: [],
  topGainers: [],
  topLosers: [],
  riskMetrics: null,

  loading: false,
  error: null,

  setTimeRange: (range, start, end) => {
    set({
      timeRange: range,
      customStart: start ?? null,
      customEnd: end ?? null,
    });
  },

  setBenchmarks: (symbols) => {
    set({ selectedBenchmarks: symbols });
  },

  fetchAll: async () => {
    set({ loading: true, error: null });
    try {
      const state = get();
      let startDate: string;
      let endDate: string;

      if (state.timeRange === "CUSTOM" && state.customStart && state.customEnd) {
        startDate = state.customStart;
        endDate = state.customEnd;
      } else {
        const range = getDateRange(state.timeRange);
        startDate = range.start;
        endDate = range.end;
      }

      // Automatically backfill missing daily snapshots using historical closing prices
      try {
        await invoke<number>("backfill_snapshots", { startDate, endDate });
      } catch (err) {
        console.warn("backfill_snapshots error (non-fatal):", err);
      }

      const [summary, returnSeries, drawdown, attribution, monthlyReturns, topGainers, topLosers, riskMetrics] =
        await Promise.allSettled([
          invoke<PerformanceSummary>("get_performance_summary", { startDate, endDate }),
          invoke<ReturnDataPoint[]>("get_return_series", { startDate, endDate }),
          invoke<DrawdownAnalysis>("get_drawdown_analysis", { startDate, endDate }),
          invoke<ReturnAttribution>("get_return_attribution", { startDate, endDate }),
          invoke<MonthlyReturn[]>("get_monthly_returns", { startDate, endDate }),
          invoke<HoldingPerformance[]>("get_holding_performance_ranking", {
            startDate,
            endDate,
            sortBy: "return_rate",
            limit: 10,
          }),
          invoke<HoldingPerformance[]>("get_holding_performance_ranking", {
            startDate,
            endDate,
            sortBy: "pnl",
            limit: 10,
          }),
          invoke<RiskMetrics>("get_risk_metrics", { startDate, endDate }),
        ]);

      set({
        summary: summary.status === "fulfilled" ? summary.value : null,
        returnSeries: returnSeries.status === "fulfilled" ? returnSeries.value : [],
        drawdown: drawdown.status === "fulfilled" ? drawdown.value : null,
        attribution: attribution.status === "fulfilled" ? attribution.value : null,
        monthlyReturns: monthlyReturns.status === "fulfilled" ? monthlyReturns.value : [],
        topGainers: topGainers.status === "fulfilled" ? topGainers.value : [],
        topLosers:
          topLosers.status === "fulfilled"
            ? [...topLosers.value].sort((a, b) => a.pnl - b.pnl).slice(0, 10)
            : [],
        riskMetrics: riskMetrics.status === "fulfilled" ? riskMetrics.value : null,
        loading: false,
      });

      // Re-fetch benchmarks that are currently selected
      const bs = get().selectedBenchmarks;
      for (const sym of bs) {
        get().fetchBenchmark(sym);
      }
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  fetchBenchmark: async (symbol) => {
    const state = get();
    let startDate: string;
    let endDate: string;
    if (state.timeRange === "CUSTOM" && state.customStart && state.customEnd) {
      startDate = state.customStart;
      endDate = state.customEnd;
    } else {
      const range = getDateRange(state.timeRange);
      startDate = range.start;
      endDate = range.end;
    }
    try {
      const series = await invoke<ReturnDataPoint[]>("get_benchmark_return_series", {
        symbol,
        startDate,
        endDate,
      });
      set((s) => ({
        benchmarkSeries: { ...s.benchmarkSeries, [symbol]: series },
      }));
    } catch (err) {
      console.error("fetchBenchmark error:", err);
    }
  },
}));
