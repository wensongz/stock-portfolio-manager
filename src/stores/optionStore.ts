import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  OptionContract,
  ExpiredOptionStats,
  SellPutSimulation,
  SellCallSimulation,
  ImportOptionsResult,
  StockPriceInput,
} from "../types";

interface OptionState {
  contracts: OptionContract[];
  expiredStats: ExpiredOptionStats | null;
  putSimulations: SellPutSimulation[];
  callSimulations: SellCallSimulation[];
  loading: boolean;
  error: string | null;

  fetchContracts: (accountId: string) => Promise<void>;
  fetchExpiredStats: (accountId: string) => Promise<void>;
  importOptionsCsv: (accountId: string, csvContent: string) => Promise<ImportOptionsResult>;
  simulateSellPut: (accountId: string, stockPrices: StockPriceInput[]) => Promise<void>;
  simulateSellCall: (accountId: string, stockPrices: StockPriceInput[]) => Promise<void>;
  deleteOptionRecords: (accountId: string) => Promise<void>;
  clearSimulations: () => void;
}

let latestContractsRequest = 0;

export const useOptionStore = create<OptionState>((set) => ({
  contracts: [],
  expiredStats: null,
  putSimulations: [],
  callSimulations: [],
  loading: false,
  error: null,

  fetchContracts: async (accountId: string) => {
    const requestId = ++latestContractsRequest;
    set({ loading: true, error: null });
    try {
      const contracts = await invoke<OptionContract[]>("get_option_contracts", {
        accountId,
      });
      if (requestId === latestContractsRequest) {
        set({ contracts, loading: false });
      }
    } catch (err) {
      if (requestId === latestContractsRequest) {
        set({ error: String(err), loading: false });
      }
    }
  },

  fetchExpiredStats: async (accountId: string) => {
    try {
      const stats = await invoke<ExpiredOptionStats>("get_expired_option_stats", {
        accountId,
      });
      set({ expiredStats: stats });
    } catch (err) {
      set({ error: String(err) });
    }
  },

  importOptionsCsv: async (accountId: string, csvContent: string) => {
    const result = await invoke<ImportOptionsResult>("import_options_csv", {
      accountId,
      csvContent,
    });
    return result;
  },

  simulateSellPut: async (accountId: string, stockPrices: StockPriceInput[]) => {
    try {
      const simulations = await invoke<SellPutSimulation[]>("simulate_sell_put", {
        accountId,
        stockPrices,
      });
      set({ putSimulations: simulations });
    } catch (err) {
      set({ error: String(err) });
    }
  },

  simulateSellCall: async (accountId: string, stockPrices: StockPriceInput[]) => {
    try {
      const simulations = await invoke<SellCallSimulation[]>("simulate_sell_call", {
        accountId,
        stockPrices,
      });
      set({ callSimulations: simulations });
    } catch (err) {
      set({ error: String(err) });
    }
  },

  deleteOptionRecords: async (accountId: string) => {
    const requestId = ++latestContractsRequest;
    await invoke("delete_option_records", { accountId });
    if (requestId === latestContractsRequest) {
      set({ contracts: [], expiredStats: null, putSimulations: [], callSimulations: [] });
    }
  },

  clearSimulations: () => {
    set({ putSimulations: [], callSimulations: [] });
  },
}));
