import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { Currency, ExchangeRates } from "../types";

interface ExchangeRateState {
  rates: ExchangeRates | null;
  baseCurrency: Currency;
  loading: boolean;
  error: string | null;
  fetchRates: () => Promise<void>;
  convertAmount: (amount: number, from: Currency, to: Currency) => Promise<number>;
  setBaseCurrency: (currency: Currency) => void;
  convertWithCachedRates: (amount: number, from: Currency, to: Currency) => number;
}

export const useExchangeRateStore = create<ExchangeRateState>((set, get) => ({
  rates: null,
  baseCurrency: "USD",
  loading: false,
  error: null,

  fetchRates: async () => {
    set({ loading: true, error: null });
    try {
      const rates = await invoke<ExchangeRates>("get_exchange_rates");
      set({ rates, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  convertAmount: async (amount: number, from: Currency, to: Currency) => {
    try {
      return await invoke<number>("convert_amount", {
        amount,
        fromCurrency: from,
        toCurrency: to,
      });
    } catch {
      return amount;
    }
  },

  setBaseCurrency: (currency: Currency) => {
    set({ baseCurrency: currency });
  },

  convertWithCachedRates: (amount: number, from: Currency, to: Currency): number => {
    const { rates } = get();
    if (!rates || from === to) return amount;

    // Convert to USD first, then to target currency
    let usdAmount = amount;
    if (from === "CNY") usdAmount = amount / rates.usd_cny;
    else if (from === "HKD") usdAmount = amount / rates.usd_hkd;

    if (to === "USD") return usdAmount;
    if (to === "CNY") return usdAmount * rates.usd_cny;
    if (to === "HKD") return usdAmount * rates.usd_hkd;
    return usdAmount;
  },
}));
