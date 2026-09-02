import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  DashboardSummary,
  DashboardReport,
  HoldingDetail,
} from "../types";

interface DashboardState {
  summary: DashboardSummary | null;
  holdingDetails: HoldingDetail[];
  loading: boolean;
  error: string | null;
  fetchReport: (baseCurrency?: string) => Promise<void>;
}

export type DashboardInvoke = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export const createDashboardStore = (invokeFn: DashboardInvoke = invoke) => {
  let latestRequestKey: string | null = null;
  const inFlight = new Map<string, Promise<void>>();

  return create<DashboardState>((set) => ({
    summary: null,
    holdingDetails: [],
    loading: false,
    error: null,

    fetchReport: (baseCurrency) => {
      const requestKey = baseCurrency ?? "USD";
      latestRequestKey = requestKey;
      const existing = inFlight.get(requestKey);
      if (existing) {
        set({ loading: true, error: null });
        return existing;
      }

      set({ loading: true, error: null });
      const request = (async () => {
        try {
          const report = await invokeFn<DashboardReport>(
            "get_dashboard_report",
            {
              baseCurrency: baseCurrency ?? null,
            },
          );
          if (latestRequestKey === requestKey) {
            set({
              summary: report.summary,
              holdingDetails: report.holdings,
              loading: false,
            });
          }
        } catch (err) {
          if (latestRequestKey === requestKey) {
            set({ loading: false, error: String(err) });
          }
        }
      })();
      inFlight.set(requestKey, request);
      void request.finally(() => {
        if (inFlight.get(requestKey) === request) {
          inFlight.delete(requestKey);
        }
      });
      return request;
    },
  }));
};

export const useDashboardStore = createDashboardStore();
