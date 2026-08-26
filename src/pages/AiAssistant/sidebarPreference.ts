export const AI_SIDEBAR_COLLAPSED_STORAGE_KEY =
  "ai_assistant_sidebar_collapsed";

interface SidebarPreferenceStorage {
  getItem: (key: string) => string | null;
  setItem: (key: string, value: string) => void;
}

export function loadAiSidebarCollapsed(
  storage: Pick<SidebarPreferenceStorage, "getItem">,
): boolean {
  const value = storage.getItem(AI_SIDEBAR_COLLAPSED_STORAGE_KEY);
  return value === "false" ? false : true;
}

export function saveAiSidebarCollapsed(
  storage: Pick<SidebarPreferenceStorage, "setItem">,
  collapsed: boolean,
): void {
  storage.setItem(AI_SIDEBAR_COLLAPSED_STORAGE_KEY, String(collapsed));
}
