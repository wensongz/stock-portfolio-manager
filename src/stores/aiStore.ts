import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { AiConfig } from "../types";

interface AiState {
  config: AiConfig | null;
  loading: boolean;
  error: string | null;

  fetchConfig: () => Promise<void>;
  updateConfig: (config: AiConfig) => Promise<boolean>;
}

export const useAiStore = create<AiState>((set) => ({
  config: null,
  loading: false,
  error: null,

  fetchConfig: async () => {
    set({ loading: true, error: null });
    try {
      const config = await invoke<AiConfig>("get_ai_config");
      set({ config, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  updateConfig: async (config: AiConfig) => {
    set({ loading: true, error: null });
    try {
      await invoke<boolean>("update_ai_config", { config });
      set({ config, loading: false });
      return true;
    } catch (err) {
      set({ error: String(err), loading: false });
      return false;
    }
  },
}));
