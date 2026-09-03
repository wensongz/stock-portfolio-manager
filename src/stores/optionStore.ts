import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  OptionContract,
  SellPutSimulation,
  SellCallSimulation,
  ImportOptionsResult,
  StockPriceInput,
} from "../types";

interface OptionState {
  contracts: OptionContract[];
  putSimulations: SellPutSimulation[];
  callSimulations: SellCallSimulation[];
  loading: boolean;
  error: string | null;
  contractsError: string | null;

  fetchContracts: (accountId: string) => Promise<void>;
  importOptionsCsv: (accountId: string, csvContent: string) => Promise<ImportOptionsResult>;
  simulateSellPut: (accountId: string, stockPrices: StockPriceInput[]) => Promise<void>;
  simulateSellCall: (accountId: string, stockPrices: StockPriceInput[]) => Promise<void>;
  deleteOptionRecords: (accountId: string) => Promise<void>;
  clearSimulations: () => void;
}

let latestContractsRequest = 0;

export const useOptionStore = create<OptionState>((set) => ({
  contracts: [],
  putSimulations: [],
  callSimulations: [],
  loading: false,
  error: null,
  contractsError: null,

  fetchContracts: async (accountId: string) => {
    const requestId = ++latestContractsRequest;
    set({ contracts: [], loading: true, error: null, contractsError: null });
    try {
      const contracts = await invoke<OptionContract[]>("get_option_contracts", {
        accountId,
      });
      if (requestId === latestContractsRequest) {
        set({ contracts, loading: false });
      }
    } catch (err) {
      if (requestId === latestContractsRequest) {
        set({ error: String(err), contractsError: String(err), loading: false });
      }
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
      set({
        contracts: [],
        putSimulations: [],
        callSimulations: [],
        contractsError: null,
      });
    }
  },

  clearSimulations: () => {
    set({ putSimulations: [], callSimulations: [] });
  },
}));
