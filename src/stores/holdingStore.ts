import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { Holding, CreateHoldingPayload, UpdateHoldingPayload, CorrectCashBalancePayload } from "../types";

interface HoldingState {
  holdings: Holding[];
  loading: boolean;
  error: string | null;
  fetchHoldings: (accountId?: string) => Promise<void>;
  createHolding: (payload: CreateHoldingPayload) => Promise<Holding>;
  updateHolding: (payload: UpdateHoldingPayload) => Promise<Holding>;
  correctCashBalance: (payload: CorrectCashBalancePayload) => Promise<Holding>;
  deleteHolding: (id: string) => Promise<void>;
}

export const createHoldingStore = (invokeFn: typeof invoke = invoke) => {
  let mutationVersion = 0;
  let latestRead = 0;
  return create<HoldingState>((set) => ({
    holdings: [],
    loading: false,
    error: null,

    fetchHoldings: async (accountId?) => {
      const read = ++latestRead;
      const version = mutationVersion;
      set({ loading: true, error: null });
      try {
        const holdings = await invokeFn<Holding[]>("get_holdings", { accountId: accountId ?? null });
        if (read !== latestRead) return;
        set(version === mutationVersion ? { holdings, loading: false } : { loading: false });
      } catch (err) {
        if (read === latestRead) set({ error: String(err), loading: false });
      }
    },

    createHolding: async (payload) => {
      const holding = await invokeFn<Holding>("create_holding", { ...payload });
      mutationVersion += 1;
      set((state) => ({ holdings: [...state.holdings, holding] }));
      return holding;
    },

    updateHolding: async (payload) => {
      const holding = await invokeFn<Holding>("update_holding", { ...payload });
      mutationVersion += 1;
      set((state) => ({
        holdings: state.holdings.map((h) => (h.id === holding.id ? holding : h)),
      }));
      return holding;
    },

    correctCashBalance: async (payload) => {
      const holding = await invokeFn<Holding>("correct_cash_balance", { ...payload });
      mutationVersion += 1;
      set((state) => ({
        holdings: state.holdings.map((h) => h.id === holding.id ? holding : h),
      }));
      return holding;
    },

    deleteHolding: async (id) => {
      await invokeFn("delete_holding", { id });
      mutationVersion += 1;
      set((state) => ({
        holdings: state.holdings.filter((h) => h.id !== id),
      }));
    },
  }));
};

export const useHoldingStore = createHoldingStore();
