import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { PriceAlert, TriggeredAlert } from "../types";

interface AlertState {
  alerts: PriceAlert[];
  triggeredAlerts: TriggeredAlert[];
  loading: boolean;
  error: string | null;

  fetchAlerts: () => Promise<void>;
  createAlert: (
    holdingId: string | null,
    symbol: string,
    name: string,
    market: string,
    alertType: string,
    threshold: number
  ) => Promise<PriceAlert | null>;
  updateAlert: (id: string, isActive: boolean) => Promise<void>;
  deleteAlert: (id: string) => Promise<void>;
  checkAlerts: (quotesJson: Record<string, [number, number, number]>) => Promise<TriggeredAlert[]>;
  clearTriggered: () => void;
}

export const useAlertStore = create<AlertState>((set, get) => ({
  alerts: [],
  triggeredAlerts: [],
  loading: false,
  error: null,

  fetchAlerts: async () => {
    set({ loading: true, error: null });
    try {
      const alerts = await invoke<PriceAlert[]>("get_alerts");
      set({ alerts, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  createAlert: async (holdingId, symbol, name, market, alertType, threshold) => {
    set({ loading: true, error: null });
    try {
      const alert = await invoke<PriceAlert>("create_alert", {
        holdingId,
        symbol,
        name,
        market,
        alertType,
        threshold,
      });
      await get().fetchAlerts();
      set({ loading: false });
      return alert;
    } catch (err) {
      set({ error: String(err), loading: false });
      return null;
    }
  },

  updateAlert: async (id, isActive) => {
    try {
      await invoke<PriceAlert>("update_alert", { id, isActive });
      await get().fetchAlerts();
    } catch (err) {
      set({ error: String(err) });
    }
  },

  deleteAlert: async (id) => {
    try {
      await invoke<boolean>("delete_alert", { id });
      await get().fetchAlerts();
    } catch (err) {
      set({ error: String(err) });
    }
  },

  checkAlerts: async (quotesJson) => {
    try {
      const triggered = await invoke<TriggeredAlert[]>("check_alerts", { quotesJson });
      set({ triggeredAlerts: triggered });
      return triggered;
    } catch (err) {
      console.error("checkAlerts error:", err);
      return [];
    }
  },

  clearTriggered: () => set({ triggeredAlerts: [] }),
}));
