export const REVIEW_TAB_STORAGE_KEY = "review_active_tab";

export type ReviewTab = "stock" | "options";

interface ReviewTabStorage {
  getItem: (key: string) => string | null;
  setItem: (key: string, value: string) => void;
}

export function isReviewTab(value: string): value is ReviewTab {
  return value === "stock" || value === "options";
}

export function loadReviewTab(
  storage: Pick<ReviewTabStorage, "getItem">,
): ReviewTab {
  const value = storage.getItem(REVIEW_TAB_STORAGE_KEY);
  return value != null && isReviewTab(value) ? value : "stock";
}

export function saveReviewTab(
  storage: Pick<ReviewTabStorage, "setItem">,
  tab: ReviewTab,
): void {
  storage.setItem(REVIEW_TAB_STORAGE_KEY, tab);
}
