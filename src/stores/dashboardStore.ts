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
  fetchReport: (
    baseCurrency?: string,
    mode?: DashboardRequestMode,
  ) => Promise<void>;
}

export type DashboardRequestMode =
  | "join-in-flight"
  | "reload-after-in-flight";

export type DashboardInvoke = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export const createDashboardStore = (invokeFn: DashboardInvoke = invoke) => {
  type RequestEntry = { token: symbol; promise: Promise<void> };

  let latestRequestToken: symbol | null = null;
  const inFlight = new Map<string, RequestEntry>();
  const queuedReloads = new Map<string, RequestEntry>();

  return create<DashboardState>((set) => {
    const startRequest = (
      baseCurrency: string | undefined,
      requestKey: string,
      token: symbol,
      makeLatest: boolean,
    ): Promise<void> => {
      if (makeLatest) latestRequestToken = token;
      if (latestRequestToken === token) {
        set({ loading: true, error: null });
      }

      const request = (async () => {
        try {
          const report = await invokeFn<DashboardReport>(
            "get_dashboard_report",
            {
              baseCurrency: baseCurrency ?? null,
            },
          );
          if (latestRequestToken === token) {
            set({
              summary: report.summary,
              holdingDetails: report.holdings,
              loading: false,
            });
          }
        } catch (err) {
          if (latestRequestToken === token) {
            set({ loading: false, error: String(err) });
          }
        }
      })();
      const entry = { token, promise: request };
      inFlight.set(requestKey, entry);
      void request.finally(() => {
        if (inFlight.get(requestKey) === entry) {
          inFlight.delete(requestKey);
        }
      });
      return request;
    };

    return {
      summary: null,
      holdingDetails: [],
      loading: false,
      error: null,

      fetchReport: (
        baseCurrency,
        mode: DashboardRequestMode = "join-in-flight",
      ) => {
        const requestKey = baseCurrency ?? "USD";
        const queued = queuedReloads.get(requestKey);
        if (queued) {
          latestRequestToken = queued.token;
          set({ loading: true, error: null });
          return queued.promise;
        }

        const existing = inFlight.get(requestKey);
        if (existing && mode === "join-in-flight") {
          latestRequestToken = existing.token;
          set({ loading: true, error: null });
          return existing.promise;
        }

        if (existing) {
          const token = Symbol(requestKey);
          latestRequestToken = token;
          set({ loading: true, error: null });
          const promise = existing.promise.then(() => {
            if (queuedReloads.get(requestKey)?.token === token) {
              queuedReloads.delete(requestKey);
            }
            return startRequest(baseCurrency, requestKey, token, false);
          });
          const entry = { token, promise };
          queuedReloads.set(requestKey, entry);
          void promise.finally(() => {
            if (queuedReloads.get(requestKey) === entry) {
              queuedReloads.delete(requestKey);
            }
          });
          return promise;
        }

        return startRequest(
          baseCurrency,
          requestKey,
          Symbol(requestKey),
          true,
        );
      },
    };
  });
};

export const useDashboardStore = createDashboardStore();
