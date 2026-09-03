import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  QuarterComparison,
  QuarterlySnapshot,
  QuarterlySnapshotDetail,
  QuarterlyTrends,
  StockTransactionGroup,
} from "../types";

export type QuarterlyInvoke = <T>(
  command: string,
  args?: Record<string, unknown>,
) => Promise<T>;

export interface QuarterlyState {
  snapshots: QuarterlySnapshot[];
  detail: QuarterlySnapshotDetail | null;
  detailSnapshotId: string | null;
  comparison: QuarterComparison | null;
  trends: QuarterlyTrends | null;
  missingQuarters: string[];
  quarterlyTransactions: StockTransactionGroup[];

  listLoading: boolean;
  listError: string | null;
  detailLoading: boolean;
  detailError: string | null;
  comparisonLoading: boolean;
  comparisonError: string | null;
  trendsLoading: boolean;
  trendsError: string | null;
  mutationLoading: boolean;
  mutationError: string | null;

  fetchSnapshots: () => Promise<void>;
  fetchDetail: (snapshotId: string) => Promise<void>;
  refreshSnapshot: (snapshotId: string) => Promise<void>;
  createSnapshot: (quarter?: string) => Promise<QuarterlySnapshot | null>;
  deleteSnapshot: (snapshotId: string) => Promise<void>;
  fetchMissingQuarters: () => Promise<void>;
  ensureCurrentQuarterSnapshot: () => Promise<QuarterlySnapshot | null>;
  compareQuarters: (quarter1: string, quarter2: string) => Promise<void>;
  fetchTrends: () => Promise<void>;
  updateHoldingNotes: (snapshotId: string, symbol: string, notes: string) => Promise<void>;
  updateQuarterlyNotes: (snapshotId: string, notes: string) => Promise<void>;
  clearDetail: () => void;
  clearComparison: () => void;
}

export const createQuarterlyStore = (invokeFn: QuarterlyInvoke = invoke) => {
  let listGeneration = 0;
  let detailGeneration = 0;
  let comparisonGeneration = 0;
  let trendsGeneration = 0;
  let missingGeneration = 0;
  let mutationGeneration = 0;
  let activeComparisonKey: string | null = null;

  return create<QuarterlyState>((set, get) => {
    const loadDetailBundle = async (
      snapshotId: string,
      detailCommand: "get_quarterly_snapshot_detail" | "refresh_quarterly_snapshot",
    ): Promise<boolean> => {
      const generation = ++detailGeneration;
      const changedSnapshot = get().detailSnapshotId !== snapshotId;
      set({
        detailSnapshotId: snapshotId,
        detailLoading: true,
        detailError: null,
        ...(changedSnapshot ? { detail: null, quarterlyTransactions: [] } : {}),
      });

      try {
        const [detail, quarterlyTransactions] = await Promise.all([
          invokeFn<QuarterlySnapshotDetail>(detailCommand, { snapshotId }),
          invokeFn<StockTransactionGroup[]>("get_quarterly_transactions", { snapshotId }),
        ]);
        if (
          generation === detailGeneration
          && get().detailSnapshotId === snapshotId
        ) {
          set({
            detail,
            quarterlyTransactions,
            detailLoading: false,
          });
          return true;
        }
      } catch (err) {
        if (
          generation === detailGeneration
          && get().detailSnapshotId === snapshotId
        ) {
          set({ detailError: String(err), detailLoading: false });
        }
      }
      return false;
    };

    const startMutation = () => {
      const generation = ++mutationGeneration;
      set({ mutationLoading: true, mutationError: null });
      return generation;
    };

    const finishMutation = (generation: number, error?: unknown) => {
      if (generation === mutationGeneration) {
        set({
          mutationLoading: false,
          ...(error === undefined ? {} : { mutationError: String(error) }),
        });
      }
    };

    return {
      snapshots: [],
      detail: null,
      detailSnapshotId: null,
      comparison: null,
      trends: null,
      missingQuarters: [],
      quarterlyTransactions: [],

      listLoading: false,
      listError: null,
      detailLoading: false,
      detailError: null,
      comparisonLoading: false,
      comparisonError: null,
      trendsLoading: false,
      trendsError: null,
      mutationLoading: false,
      mutationError: null,

      fetchSnapshots: async () => {
        const generation = ++listGeneration;
        set({ listLoading: true, listError: null });
        try {
          const snapshots = await invokeFn<QuarterlySnapshot[]>("get_quarterly_snapshots");
          if (generation === listGeneration) {
            set({ snapshots, listLoading: false });
          }
        } catch (err) {
          if (generation === listGeneration) {
            set({ listError: String(err), listLoading: false });
          }
        }
      },

      fetchDetail: async (snapshotId) => {
        await loadDetailBundle(snapshotId, "get_quarterly_snapshot_detail");
      },

      refreshSnapshot: async (snapshotId) => {
        const mutation = startMutation();
        const refreshed = await loadDetailBundle(snapshotId, "refresh_quarterly_snapshot");
        if (!refreshed) {
          const error = get().detailSnapshotId === snapshotId
            ? get().detailError ?? "季度快照刷新失败"
            : undefined;
          finishMutation(mutation, error);
          return;
        }
        await get().fetchSnapshots();
        finishMutation(mutation);
      },

      createSnapshot: async (quarter) => {
        const generation = startMutation();
        try {
          const snapshot = await invokeFn<QuarterlySnapshot>("create_quarterly_snapshot", {
            quarter: quarter ?? null,
          });
          await get().fetchSnapshots();
          finishMutation(generation);
          return snapshot;
        } catch (err) {
          finishMutation(generation, err);
          return null;
        }
      },

      deleteSnapshot: async (snapshotId) => {
        const generation = startMutation();
        try {
          await invokeFn<boolean>("delete_quarterly_snapshot", { snapshotId });
          await get().fetchSnapshots();
          finishMutation(generation);
        } catch (err) {
          finishMutation(generation, err);
        }
      },

      fetchMissingQuarters: async () => {
        const generation = ++missingGeneration;
        try {
          const missingQuarters = await invokeFn<string[]>("check_missing_snapshots");
          if (generation === missingGeneration) {
            set({ missingQuarters });
          }
        } catch (err) {
          console.error("fetchMissingQuarters error:", err);
        }
      },

      ensureCurrentQuarterSnapshot: async () => {
        try {
          return await invokeFn<QuarterlySnapshot | null>(
            "ensure_current_quarter_snapshot",
          );
        } catch (err) {
          console.error("ensureCurrentQuarterSnapshot error:", err);
          return null;
        }
      },

      compareQuarters: async (quarter1, quarter2) => {
        const pairKey = JSON.stringify([quarter1.trim(), quarter2.trim()]);
        const generation = ++comparisonGeneration;
        activeComparisonKey = pairKey;
        set({
          comparison: null,
          comparisonLoading: true,
          comparisonError: null,
        });
        try {
          const comparison = await invokeFn<QuarterComparison>("compare_quarters", {
            quarter1,
            quarter2,
          });
          if (
            generation === comparisonGeneration
            && activeComparisonKey === pairKey
          ) {
            set({ comparison, comparisonLoading: false });
          }
        } catch (err) {
          if (generation === comparisonGeneration) {
            set({ comparisonError: String(err), comparisonLoading: false });
          }
        }
      },

      fetchTrends: async () => {
        const generation = ++trendsGeneration;
        set({ trendsLoading: true, trendsError: null });
        try {
          const trends = await invokeFn<QuarterlyTrends>("get_quarterly_trends");
          if (generation === trendsGeneration) {
            set({ trends, trendsLoading: false });
          }
        } catch (err) {
          if (generation === trendsGeneration) {
            set({ trendsError: String(err), trendsLoading: false });
          }
        }
      },

      updateHoldingNotes: async (snapshotId, symbol, notes) => {
        const generation = startMutation();
        try {
          await invokeFn<boolean>("update_holding_notes", { snapshotId, symbol, notes });
          if (get().detailSnapshotId === snapshotId) {
            await get().fetchDetail(snapshotId);
          }
          finishMutation(generation);
        } catch (err) {
          finishMutation(generation, err);
        }
      },

      updateQuarterlyNotes: async (snapshotId, notes) => {
        const generation = startMutation();
        try {
          await invokeFn<boolean>("update_quarterly_notes", { snapshotId, notes });
          if (get().detailSnapshotId === snapshotId) {
            await get().fetchDetail(snapshotId);
          }
          await get().fetchSnapshots();
          finishMutation(generation);
        } catch (err) {
          finishMutation(generation, err);
          throw err;
        }
      },

      clearDetail: () => {
        detailGeneration += 1;
        set({
          detail: null,
          detailSnapshotId: null,
          quarterlyTransactions: [],
          detailLoading: false,
          detailError: null,
        });
      },

      clearComparison: () => {
        comparisonGeneration += 1;
        activeComparisonKey = null;
        set({
          comparison: null,
          comparisonLoading: false,
          comparisonError: null,
        });
      },
    };
  });
};

export const useQuarterlyStore = createQuarterlyStore();
