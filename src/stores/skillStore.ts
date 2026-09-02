import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { Skill } from "../types";

interface SkillState {
  skills: Skill[];
  loading: boolean;
  error: string | null;

  fetchSkills: () => Promise<void>;
  saveSkill: (skill: Skill) => Promise<boolean>;
  deleteSkill: (id: string) => Promise<boolean>;
  resetSkills: () => Promise<boolean>;
  /** Clone a skill to a fresh id. Returns the created skill, or null on failure. */
  cloneSkill: (sourceId: string, newId: string) => Promise<Skill | null>;
  /** Export a skill's Markdown to a path (path chosen by caller via dialog). */
  exportSkill: (id: string, path: string) => Promise<string | null>;
  /** Import a skill from a Markdown path. Returns the saved skill or null. */
  importSkill: (path: string) => Promise<Skill | null>;
}

export const useSkillStore = create<SkillState>((set, get) => ({
  skills: [],
  loading: false,
  error: null,

  fetchSkills: async () => {
    set({ loading: true, error: null });
    try {
      const skills = await invoke<Skill[]>("list_skills");
      set({ skills, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  saveSkill: async (skill) => {
    set({ loading: true, error: null });
    try {
      await invoke("save_skill", { skill });
      // Refresh the list so chips / autocomplete reflect the new state.
      await get().fetchSkills();
      return true;
    } catch (err) {
      set({ error: String(err), loading: false });
      return false;
    }
  },

  deleteSkill: async (id) => {
    set({ loading: true, error: null });
    try {
      await invoke("delete_skill", { id });
      await get().fetchSkills();
      return true;
    } catch (err) {
      set({ error: String(err), loading: false });
      return false;
    }
  },

  resetSkills: async () => {
    set({ loading: true, error: null });
    try {
      await invoke("reset_skills");
      await get().fetchSkills();
      return true;
    } catch (err) {
      set({ error: String(err), loading: false });
      return false;
    }
  },

  cloneSkill: async (sourceId, newId) => {
    set({ loading: true, error: null });
    try {
      const cloned = await invoke<Skill>("clone_skill", {
        sourceId,
        newId,
      });
      await get().fetchSkills();
      return cloned;
    } catch (err) {
      set({ error: String(err), loading: false });
      return null;
    }
  },

  exportSkill: async (id, path) => {
    try {
      const written = await invoke<string>("export_skill", { id, path });
      return written;
    } catch (err) {
      set({ error: String(err) });
      return null;
    }
  },

  importSkill: async (path) => {
    set({ loading: true, error: null });
    try {
      const imported = await invoke<Skill>("import_skill", { path });
      await get().fetchSkills();
      return imported;
    } catch (err) {
      set({ error: String(err), loading: false });
      return null;
    }
  },
}));

/**
 * Return the enabled skills whose `trigger` keywords appear in `text`
 * (case-insensitive). Mirrors the backend auto-activation logic so the UI
 * can show a hint chip ("⚡ 风险分析 已自动激活") before sending.
 *
 * Matching rule (kept in sync with `skill_service::match_triggers`):
 * - CJK keywords (any non-ASCII char) → substring match (no word boundaries).
 * - Pure-ASCII keywords → whole-word match via regex `\b`, so `risk` does not
 *   match `riskier`.
 */
export function matchTriggers(skills: Skill[], text: string): Skill[] {
  const haystack = text.toLowerCase();
  return skills.filter(
    (s) =>
      s.enabled &&
      s.trigger.some((t) => {
        const kw = t.trim().toLowerCase();
        if (kw.length === 0) return false;
        const isCjk = /[^\x00-\x7f]/.test(kw);
        if (isCjk) {
          return haystack.includes(kw);
        }
        // `\b` treats ASCII alphanumerics as word chars; punctuation and CJK
        // count as boundaries, matching the Rust `is_word_byte` rule.
        const escaped = kw.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
        const re = new RegExp(`(?:^|[^a-z0-9])${escaped}(?:$|[^a-z0-9])`);
        return re.test(haystack);
      })
  );
}

/**
 * Find a skill by id. Convenience wrapper over the in-memory list so UI
 * components don't each have to filter the array themselves.
 */
export function findSkill(skills: Skill[], id: string): Skill | undefined {
  return skills.find((s) => s.id === id);
}
