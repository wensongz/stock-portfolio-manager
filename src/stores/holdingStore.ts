import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { Holding, CreateHoldingPayload, UpdateHoldingPayload } from "../types";

interface HoldingState {
  holdings: Holding[];
  loading: boolean;
  error: string | null;
  fetchHoldings: (accountId?: string) => Promise<void>;
  createHolding: (payload: CreateHoldingPayload) => Promise<Holding>;
  updateHolding: (payload: UpdateHoldingPayload) => Promise<Holding>;
  deleteHolding: (id: string) => Promise<void>;
}

export const useHoldingStore = create<HoldingState>((set) => ({
  holdings: [],
  loading: false,
  error: null,

  fetchHoldings: async (accountId?) => {
    set({ loading: true, error: null });
    try {
      const holdings = await invoke<Holding[]>("get_holdings", { accountId: accountId ?? null });
      set({ holdings, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  createHolding: async (payload) => {
    const holding = await invoke<Holding>("create_holding", { ...payload });
    set((state) => ({ holdings: [...state.holdings, holding] }));
    return holding;
  },

  updateHolding: async (payload) => {
    const holding = await invoke<Holding>("update_holding", { ...payload });
    set((state) => ({
      holdings: state.holdings.map((h) => (h.id === holding.id ? holding : h)),
    }));
    return holding;
  },

  deleteHolding: async (id) => {
    await invoke("delete_holding", { id });
    set((state) => ({
      holdings: state.holdings.filter((h) => h.id !== id),
    }));
  },
}));
