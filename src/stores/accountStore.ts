import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { Account, CreateAccountPayload, UpdateAccountPayload } from "../types";

interface AccountState {
  accounts: Account[];
  loading: boolean;
  error: string | null;
  fetchAccounts: () => Promise<void>;
  createAccount: (payload: CreateAccountPayload) => Promise<Account>;
  updateAccount: (payload: UpdateAccountPayload) => Promise<Account>;
  deleteAccount: (id: string) => Promise<void>;
}

export const useAccountStore = create<AccountState>((set) => ({
  accounts: [],
  loading: false,
  error: null,

  fetchAccounts: async () => {
    set({ loading: true, error: null });
    try {
      const accounts = await invoke<Account[]>("get_accounts");
      set({ accounts, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  createAccount: async (payload) => {
    const account = await invoke<Account>("create_account", { ...payload });
    set((state) => ({ accounts: [...state.accounts, account] }));
    return account;
  },

  updateAccount: async (payload) => {
    const account = await invoke<Account>("update_account", { ...payload });
    set((state) => ({
      accounts: state.accounts.map((a) => (a.id === account.id ? account : a)),
    }));
    return account;
  },

  deleteAccount: async (id) => {
    await invoke("delete_account", { id });
    set((state) => ({
      accounts: state.accounts.filter((a) => a.id !== id),
    }));
  },
}));
