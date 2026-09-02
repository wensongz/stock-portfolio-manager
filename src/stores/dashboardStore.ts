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

export const createDashboardStore = (invokeFn: DashboardInvoke = invoke) =>
  create<DashboardState>((set) => ({
    summary: null,
    holdingDetails: [],
    loading: false,
    error: null,

    fetchReport: async (baseCurrency) => {
      set({ loading: true, error: null });
      try {
        const report = await invokeFn<DashboardReport>("get_dashboard_report", {
          baseCurrency: baseCurrency ?? null,
        });
        set({
          summary: report.summary,
          holdingDetails: report.holdings,
          loading: false,
        });
      } catch (err) {
        set({ loading: false, error: String(err) });
      }
    },
  }));

export const useDashboardStore = createDashboardStore();
