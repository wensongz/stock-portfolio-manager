import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  QuarterComparison,
  QuarterlyNotesSummary,
  QuarterlySnapshot,
  QuarterlySnapshotDetail,
  QuarterlyTrends,
} from "../types";

interface QuarterlyState {
  snapshots: QuarterlySnapshot[];
  detail: QuarterlySnapshotDetail | null;
  comparison: QuarterComparison | null;
  trends: QuarterlyTrends | null;
  notesSummaries: QuarterlyNotesSummary[];
  missingQuarters: string[];
  loading: boolean;
  error: string | null;

  fetchSnapshots: () => Promise<void>;
  fetchDetail: (snapshotId: string) => Promise<void>;
  refreshSnapshot: (snapshotId: string) => Promise<void>;
  createSnapshot: (quarter?: string) => Promise<QuarterlySnapshot | null>;
  deleteSnapshot: (snapshotId: string) => Promise<void>;
  fetchMissingQuarters: () => Promise<void>;
  compareQuarters: (quarter1: string, quarter2: string) => Promise<void>;
  fetchTrends: () => Promise<void>;
  fetchNotesSummaries: () => Promise<void>;
  updateHoldingNotes: (snapshotId: string, symbol: string, notes: string) => Promise<void>;
  updateQuarterlyNotes: (snapshotId: string, notes: string) => Promise<void>;
  clearDetail: () => void;
  clearComparison: () => void;
}

export const useQuarterlyStore = create<QuarterlyState>((set, get) => ({
  snapshots: [],
  detail: null,
  comparison: null,
  trends: null,
  notesSummaries: [],
  missingQuarters: [],
  loading: false,
  error: null,

  fetchSnapshots: async () => {
    set({ loading: true, error: null });
    try {
      const snapshots = await invoke<QuarterlySnapshot[]>("get_quarterly_snapshots");
      set({ snapshots, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  fetchDetail: async (snapshotId: string) => {
    set({ loading: true, error: null });
    try {
      const detail = await invoke<QuarterlySnapshotDetail>("get_quarterly_snapshot_detail", {
        snapshotId,
      });
      set({ detail, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  refreshSnapshot: async (snapshotId: string) => {
    set({ loading: true, error: null });
    try {
      const detail = await invoke<QuarterlySnapshotDetail>("refresh_quarterly_snapshot", {
        snapshotId,
      });
      set({ detail, loading: false });
      // Also refresh the snapshots list since totals may have changed
      await get().fetchSnapshots();
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  createSnapshot: async (quarter?: string) => {
    set({ loading: true, error: null });
    try {
      const snapshot = await invoke<QuarterlySnapshot>("create_quarterly_snapshot", {
        quarter: quarter ?? null,
      });
      // Refresh snapshots list
      await get().fetchSnapshots();
      set({ loading: false });
      return snapshot;
    } catch (err) {
      set({ error: String(err), loading: false });
      return null;
    }
  },

  deleteSnapshot: async (snapshotId: string) => {
    set({ loading: true, error: null });
    try {
      await invoke<boolean>("delete_quarterly_snapshot", { snapshotId });
      await get().fetchSnapshots();
      set({ loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  fetchMissingQuarters: async () => {
    try {
      const missingQuarters = await invoke<string[]>("check_missing_snapshots");
      set({ missingQuarters });
    } catch (err) {
      console.error("fetchMissingQuarters error:", err);
    }
  },

  compareQuarters: async (quarter1: string, quarter2: string) => {
    set({ loading: true, error: null });
    try {
      const comparison = await invoke<QuarterComparison>("compare_quarters", {
        quarter1,
        quarter2,
      });
      set({ comparison, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  fetchTrends: async () => {
    set({ loading: true, error: null });
    try {
      const trends = await invoke<QuarterlyTrends>("get_quarterly_trends");
      set({ trends, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  fetchNotesSummaries: async () => {
    set({ loading: true, error: null });
    try {
      const notesSummaries = await invoke<QuarterlyNotesSummary[]>("get_quarterly_notes_history");
      set({ notesSummaries, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  updateHoldingNotes: async (snapshotId: string, symbol: string, notes: string) => {
    try {
      await invoke<boolean>("update_holding_notes", { snapshotId, symbol, notes });
      // Refresh detail if currently viewing the same snapshot
      const { detail } = get();
      if (detail?.snapshot.id === snapshotId) {
        await get().fetchDetail(snapshotId);
      }
    } catch (err) {
      set({ error: String(err) });
    }
  },

  updateQuarterlyNotes: async (snapshotId: string, notes: string) => {
    try {
      await invoke<boolean>("update_quarterly_notes", { snapshotId, notes });
      // Refresh detail if currently viewing the same snapshot
      const { detail } = get();
      if (detail?.snapshot.id === snapshotId) {
        await get().fetchDetail(snapshotId);
      }
      // Also refresh snapshots list for updated notes
      await get().fetchSnapshots();
    } catch (err) {
      set({ error: String(err) });
      throw err;
    }
  },

  clearDetail: () => set({ detail: null }),
  clearComparison: () => set({ comparison: null }),
}));
