import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { Category, CreateCategoryPayload, UpdateCategoryPayload } from "../types";

interface CategoryState {
  categories: Category[];
  loading: boolean;
  error: string | null;
  fetchCategories: () => Promise<void>;
  createCategory: (payload: CreateCategoryPayload) => Promise<Category>;
  updateCategory: (payload: UpdateCategoryPayload) => Promise<Category>;
  deleteCategory: (id: string) => Promise<void>;
}

export const useCategoryStore = create<CategoryState>((set) => ({
  categories: [],
  loading: false,
  error: null,

  fetchCategories: async () => {
    set({ loading: true, error: null });
    try {
      const categories = await invoke<Category[]>("get_categories");
      set({ categories, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  createCategory: async (payload) => {
    const category = await invoke<Category>("create_category", { ...payload });
    set((state) => ({ categories: [...state.categories, category] }));
    return category;
  },

  updateCategory: async (payload) => {
    const category = await invoke<Category>("update_category", { ...payload });
    set((state) => ({
      categories: state.categories.map((c) => (c.id === category.id ? category : c)),
    }));
    return category;
  },

  deleteCategory: async (id) => {
    await invoke("delete_category", { id });
    set((state) => ({
      categories: state.categories.filter((c) => c.id !== id),
    }));
  },
}));
