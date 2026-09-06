import { invoke } from "@tauri-apps/api/core";
import { useStore } from "zustand";
import { createStore, type StoreApi } from "zustand/vanilla";
import type {
  PortfolioAlertBreach,
  PortfolioAlertEvaluation,
  PortfolioAlertNotification,
  PortfolioAlertScope,
  PortfolioAlertView,
  SavePortfolioAlertConfigInput,
} from "../types";

export type PortfolioAlertInvoke = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export interface PortfolioAlertStoreState {
  selectedScopeKey: string;
  viewsByScope: Record<string, PortfolioAlertView | undefined>;
  loadingByScope: Record<string, boolean>;
  errorsByScope: Record<string, string | undefined>;
  pendingNotifications: PortfolioAlertBreach[];
  selectScope(scope: PortfolioAlertScope): void;
  loadScope(scope: PortfolioAlertScope): Promise<void>;
  saveConfig(input: SavePortfolioAlertConfigInput): Promise<void>;
  setActive(
    configId: string,
    scope: PortfolioAlertScope,
    isActive: boolean,
  ): Promise<void>;
  evaluate(configId: string, scope: PortfolioAlertScope): Promise<void>;
  ingestNotification(notification: PortfolioAlertNotification): void;
  takePendingNotifications(): PortfolioAlertBreach[];
}

export function portfolioAlertScopeKey(scope: PortfolioAlertScope): string {
  switch (scope.kind) {
    case "OVERALL":
      return "overall";
    case "MARKET":
      return `market:${scope.market}`;
    case "ACCOUNT":
      return `account:${scope.accountId}`;
  }
}

export function selectCurrentPortfolioAlertView(
  state: PortfolioAlertStoreState,
): PortfolioAlertView | undefined {
  return state.viewsByScope[state.selectedScopeKey];
}

function breachNotificationKey(breach: PortfolioAlertBreach): string {
  return `${breach.configId}:${breach.breachKey}:${breach.firstTriggeredAt}`;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

export function createPortfolioAlertStore(
  invokeFn: PortfolioAlertInvoke = invoke,
): StoreApi<PortfolioAlertStoreState> {
  const generations = new Map<string, number>();
  const queuedBreachKeys = new Set<string>();

  return createStore<PortfolioAlertStoreState>((set) => {
    const beginRequest = (scopeKey: string): number => {
      const generation = (generations.get(scopeKey) ?? 0) + 1;
      generations.set(scopeKey, generation);
      set((state) => ({
        loadingByScope: { ...state.loadingByScope, [scopeKey]: true },
        errorsByScope: { ...state.errorsByScope, [scopeKey]: undefined },
      }));
      return generation;
    };

    const enqueueBreaches = (
      state: PortfolioAlertStoreState,
      breaches: PortfolioAlertBreach[],
    ): PortfolioAlertBreach[] => {
      const uniqueBreaches = breaches.filter((breach) => {
        const key = breachNotificationKey(breach);
        if (queuedBreachKeys.has(key)) return false;
        queuedBreachKeys.add(key);
        return true;
      });
      return uniqueBreaches.length === 0
        ? state.pendingNotifications
        : [...state.pendingNotifications, ...uniqueBreaches];
    };

    const finishView = (
      scopeKey: string,
      generation: number,
      view: PortfolioAlertView,
    ) => {
      if (generations.get(scopeKey) !== generation) return;
      set((state) => ({
        viewsByScope: { ...state.viewsByScope, [scopeKey]: view },
        loadingByScope: { ...state.loadingByScope, [scopeKey]: false },
        errorsByScope: { ...state.errorsByScope, [scopeKey]: undefined },
        pendingNotifications: enqueueBreaches(
          state,
          view.evaluation?.newlyTriggered ?? [],
        ),
      }));
    };

    const finishEvaluation = (
      scopeKey: string,
      generation: number,
      evaluation: PortfolioAlertEvaluation,
    ) => {
      if (generations.get(scopeKey) !== generation) return;
      set((state) => ({
        viewsByScope: {
          ...state.viewsByScope,
          [scopeKey]: {
            config: state.viewsByScope[scopeKey]?.config ?? null,
            evaluation,
          },
        },
        loadingByScope: { ...state.loadingByScope, [scopeKey]: false },
        errorsByScope: { ...state.errorsByScope, [scopeKey]: undefined },
        pendingNotifications: enqueueBreaches(state, evaluation.newlyTriggered),
      }));
    };

    const failRequest = (scopeKey: string, generation: number, error: unknown) => {
      if (generations.get(scopeKey) !== generation) return;
      set((state) => ({
        loadingByScope: { ...state.loadingByScope, [scopeKey]: false },
        errorsByScope: {
          ...state.errorsByScope,
          [scopeKey]: errorMessage(error),
        },
      }));
    };

    return {
      selectedScopeKey: "overall",
      viewsByScope: {},
      loadingByScope: {},
      errorsByScope: {},
      pendingNotifications: [],

      selectScope: (scope) => {
        set({ selectedScopeKey: portfolioAlertScopeKey(scope) });
      },

      loadScope: async (scope) => {
        const scopeKey = portfolioAlertScopeKey(scope);
        set({ selectedScopeKey: scopeKey });
        const generation = beginRequest(scopeKey);
        try {
          const view = await invokeFn<PortfolioAlertView>(
            "get_portfolio_alert_view",
            { scope },
          );
          finishView(scopeKey, generation, view);
        } catch (error) {
          failRequest(scopeKey, generation, error);
        }
      },

      saveConfig: async (input) => {
        const scopeKey = portfolioAlertScopeKey(input.scope);
        set({ selectedScopeKey: scopeKey });
        const generation = beginRequest(scopeKey);
        try {
          const view = await invokeFn<PortfolioAlertView>(
            "save_portfolio_alert_config",
            { input },
          );
          finishView(scopeKey, generation, view);
        } catch (error) {
          failRequest(scopeKey, generation, error);
        }
      },

      setActive: async (configId, scope, isActive) => {
        const scopeKey = portfolioAlertScopeKey(scope);
        const generation = beginRequest(scopeKey);
        try {
          const view = await invokeFn<PortfolioAlertView>(
            "set_portfolio_alert_active",
            { configId, isActive },
          );
          finishView(scopeKey, generation, view);
        } catch (error) {
          failRequest(scopeKey, generation, error);
        }
      },

      evaluate: async (configId, scope) => {
        const scopeKey = portfolioAlertScopeKey(scope);
        const generation = beginRequest(scopeKey);
        try {
          const evaluation = await invokeFn<PortfolioAlertEvaluation>(
            "evaluate_portfolio_alert",
            { configId },
          );
          finishEvaluation(scopeKey, generation, evaluation);
        } catch (error) {
          failRequest(scopeKey, generation, error);
        }
      },

      ingestNotification: (notification) => {
        set((state) => ({
          pendingNotifications: enqueueBreaches(state, [notification.breach]),
        }));
      },

      takePendingNotifications: () => {
        let notifications: PortfolioAlertBreach[] = [];
        set((state) => {
          notifications = state.pendingNotifications;
          return { pendingNotifications: [] };
        });
        return notifications;
      },
    };
  });
}

export const portfolioAlertStore = createPortfolioAlertStore();

export function usePortfolioAlertStore<T>(
  selector: (state: PortfolioAlertStoreState) => T,
): T {
  return useStore(portfolioAlertStore, selector);
}
