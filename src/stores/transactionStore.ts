import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { Transaction, CreateTransactionPayload } from "../types";

interface TransactionState {
  transactions: Transaction[];
  loading: boolean;
  error: string | null;
  fetchTransactions: (filters?: { accountId?: string; symbol?: string }) => Promise<void>;
  createTransaction: (payload: CreateTransactionPayload) => Promise<Transaction>;
  deleteTransaction: (id: string) => Promise<void>;
}

export const useTransactionStore = create<TransactionState>((set) => ({
  transactions: [],
  loading: false,
  error: null,

  fetchTransactions: async (filters?) => {
    set({ loading: true, error: null });
    try {
      const transactions = await invoke<Transaction[]>("get_transactions", {
        accountId: filters?.accountId ?? null,
        symbol: filters?.symbol ?? null,
      });
      set({ transactions, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  createTransaction: async (payload) => {
    const transaction = await invoke<Transaction>("create_transaction", { ...payload });
    set((state) => ({ transactions: [transaction, ...state.transactions] }));
    return transaction;
  },

  deleteTransaction: async (id) => {
    await invoke("delete_transaction", { id });
    set((state) => ({
      transactions: state.transactions.filter((t) => t.id !== id),
    }));
  },
}));
