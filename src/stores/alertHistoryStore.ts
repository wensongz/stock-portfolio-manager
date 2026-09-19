import { invoke } from "@tauri-apps/api/core";
import { useStore } from "zustand";
import { createStore, type StoreApi } from "zustand/vanilla";
import type {
  AlertHistoryMessage,
  AlertHistoryPage,
  AlertHistoryQuery,
} from "../types/alertHistory";

export type AlertHistoryInvoke = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export interface AlertHistoryStoreState {
  items: AlertHistoryMessage[];
  total: number;
  page: number;
  pageSize: number;
  kind: AlertHistoryQuery["kind"];
  search: string;
  loading: boolean;
  error: string | null;
  deletingId: string | null;
  fetchHistory(): Promise<void>;
  deleteMessage(id: string): Promise<boolean>;
  setKind(kind: AlertHistoryQuery["kind"]): Promise<void>;
  setSearch(search: string): Promise<void>;
  setPagination(page: number, pageSize: number): Promise<void>;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function currentQuery(state: AlertHistoryStoreState): AlertHistoryQuery {
  return {
    ...(state.kind ? { kind: state.kind } : {}),
    ...(state.search ? { search: state.search } : {}),
    page: state.page,
    pageSize: state.pageSize,
  };
}

export function createAlertHistoryStore(
  invokeFn: AlertHistoryInvoke = invoke,
): StoreApi<AlertHistoryStoreState> {
  let requestGeneration = 0;

  return createStore<AlertHistoryStoreState>((set, get) => {
    const fetchHistory = async (): Promise<void> => {
      const generation = ++requestGeneration;
      const query = currentQuery(get());
      set({ loading: true, error: null });

      try {
        const result = await invokeFn<AlertHistoryPage>("get_alert_history", {
          query,
        });
        if (generation !== requestGeneration) return;
        const lastPage = Math.max(1, Math.ceil(result.total / result.pageSize));
        if (result.page > lastPage) {
          set({ page: lastPage });
          await fetchHistory();
          return;
        }
        set({
          items: result.items,
          total: result.total,
          page: result.page,
          pageSize: result.pageSize,
          loading: false,
          error: null,
        });
      } catch (error) {
        if (generation !== requestGeneration) return;
        set({ loading: false, error: errorMessage(error) });
      }
    };

    return {
      items: [],
      total: 0,
      page: 1,
      pageSize: 20,
      kind: undefined,
      search: "",
      loading: false,
      error: null,
      deletingId: null,
      fetchHistory,
      deleteMessage: async (id) => {
        if (get().deletingId !== null) return false;
        set({ deletingId: id });
        try {
          await invokeFn<boolean>("delete_alert_history", { id });
          // A completed deletion must invalidate any reads started before it.
          // Remove the committed row locally even if the subsequent refresh fails.
          requestGeneration += 1;
          set((state) => ({
            items: state.items.filter((item) => item.id !== id),
            total: state.items.some((item) => item.id === id)
              ? Math.max(0, state.total - 1)
              : state.total,
          }));
          await fetchHistory();
          return true;
        } finally {
          set({ deletingId: null });
        }
      },
      setKind: async (kind) => {
        set({ kind, page: 1 });
        await fetchHistory();
      },
      setSearch: async (search) => {
        set({ search: search.trim(), page: 1 });
        await fetchHistory();
      },
      setPagination: async (page, pageSize) => {
        set({ page, pageSize });
        await fetchHistory();
      },
    };
  });
}

export const alertHistoryStore = createAlertHistoryStore();

export function useAlertHistoryStore<T>(
  selector: (state: AlertHistoryStoreState) => T,
): T {
  return useStore(alertHistoryStore, selector);
}
