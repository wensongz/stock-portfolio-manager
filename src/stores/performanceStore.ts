import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  DrawdownAnalysis,
  HoldingPerformance,
  MonthlyReturn,
  PerformanceReport,
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
  selectedMarket: string | null;
  selectedAccountId: string | null;

  summary: PerformanceSummary | null;
  returnSeries: ReturnDataPoint[];
  benchmarkSeries: Record<string, ReturnDataPoint[]>;
  drawdown: DrawdownAnalysis | null;
  attribution: ReturnAttribution | null;
  monthlyReturns: MonthlyReturn[];
  holdingPerformances: HoldingPerformance[];
  riskMetrics: RiskMetrics | null;

  loading: boolean;
  error: string | null;

  setTimeRange: (range: TimeRange, start?: string, end?: string) => void;
  setBenchmarks: (symbols: string[]) => void;
  setMarket: (market: string | null) => Promise<void>;
  setAccountId: (accountId: string | null) => Promise<void>;
  fetchAll: (forceRefresh?: boolean) => Promise<void>;
  fetchBenchmark: (symbol: string) => Promise<void>;
}

export type PerformanceInvoke = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export function createPerformanceStore(invokeFn: PerformanceInvoke = invoke) {
  let latestRequestId = 0;
  let nextBenchmarkRequestId = 0;
  const latestBenchmarkRequestIds = new Map<string, number>();

  return create<PerformanceState>((set, get) => ({
  timeRange: "1M",
  customStart: null,
  customEnd: null,
  selectedBenchmarks: [],
  selectedMarket: null,
  selectedAccountId: null,

  summary: null,
  returnSeries: [],
  benchmarkSeries: {},
  drawdown: null,
  attribution: null,
  monthlyReturns: [],
  holdingPerformances: [],
  riskMetrics: null,

  loading: false,
  error: null,

  setTimeRange: (range, start, end) => {
    const nextCustomStart = start ?? null;
    const nextCustomEnd = end ?? null;
    const state = get();
    const rangeChanged =
      state.timeRange !== range ||
      state.customStart !== nextCustomStart ||
      state.customEnd !== nextCustomEnd;

    if (rangeChanged) {
      latestBenchmarkRequestIds.clear();
    }
    set({
      timeRange: range,
      customStart: nextCustomStart,
      customEnd: nextCustomEnd,
      ...(rangeChanged ? { benchmarkSeries: {} } : {}),
    });
  },

  setBenchmarks: (symbols) => {
    set({ selectedBenchmarks: symbols });
  },

  setMarket: async (market) => {
    set({ selectedMarket: market, selectedAccountId: null });
    await get().fetchAll();
  },

  setAccountId: async (accountId) => {
    set({ selectedAccountId: accountId, selectedMarket: null });
    await get().fetchAll();
  },

  fetchAll: async (forceRefresh?: boolean) => {
    const requestId = ++latestRequestId;
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

      const filterParams: { market?: string; accountId?: string } = {};
      if (state.selectedMarket) {
        filterParams.market = state.selectedMarket;
      }
      if (state.selectedAccountId) {
        filterParams.accountId = state.selectedAccountId;
      }

      // Automatically backfill missing daily snapshots using historical closing prices.
      // When forceRefresh is true (user clicked "刷新"), re-create all snapshots
      // including transaction-aware adjustments. Otherwise only fill in dates
      // that have never been computed, so the page loads quickly from cache.
      try {
        await invokeFn<number>("backfill_snapshots", {
          startDate,
          endDate,
          force: forceRefresh ?? false,
        });
      } catch (err) {
        console.warn("backfill_snapshots error (non-fatal):", err);
      }

      if (requestId !== latestRequestId) return;

      const report = await invokeFn<PerformanceReport>("get_performance_report", {
        startDate,
        endDate,
        rankingLimit: 10_000,
        ...filterParams,
      });

      if (requestId !== latestRequestId) return;

      set({
        summary: report.summary,
        returnSeries: report.summary.return_series,
        drawdown: report.drawdown,
        attribution: report.attribution,
        monthlyReturns: report.monthly_returns,
        holdingPerformances: report.holding_performances,
        riskMetrics: report.risk_metrics,
        loading: false,
      });

      // Re-fetch benchmarks that are currently selected
      const bs = get().selectedBenchmarks;
      for (const sym of bs) {
        get().fetchBenchmark(sym);
      }
    } catch (err) {
      if (requestId === latestRequestId) {
        set({ error: String(err), loading: false });
      }
    }
  },

  fetchBenchmark: async (symbol) => {
    const requestId = ++nextBenchmarkRequestId;
    latestBenchmarkRequestIds.set(symbol, requestId);
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
      const series = await invokeFn<ReturnDataPoint[]>("get_benchmark_return_series", {
        symbol,
        startDate,
        endDate,
      });
      if (latestBenchmarkRequestIds.get(symbol) !== requestId) return;
      set((s) => ({
        benchmarkSeries: { ...s.benchmarkSeries, [symbol]: series },
      }));
    } catch (err) {
      if (latestBenchmarkRequestIds.get(symbol) === requestId) {
        console.error("fetchBenchmark error:", err);
      }
    }
  },
  }));
}

export const usePerformanceStore = createPerformanceStore();
