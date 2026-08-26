// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  REVIEW_TAB_STORAGE_KEY,
  loadReviewTab,
  saveReviewTab,
} from "./reviewTabPreference.ts";

function memoryStorage(initial: Record<string, string> = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem(key: string) {
      return values.get(key) ?? null;
    },
    setItem(key: string, value: string) {
      values.set(key, value);
    },
  };
}

test("review page remembers the latest selected tab", () => {
  const storage = memoryStorage();

  saveReviewTab(storage, "options");

  assert.equal(storage.getItem(REVIEW_TAB_STORAGE_KEY), "options");
  assert.equal(loadReviewTab(storage), "options");
});

test("review page falls back to the stock tab for missing or invalid data", () => {
  for (const value of [undefined, "", "other"]) {
    const storage = memoryStorage(
      value === undefined ? {} : { review_active_tab: value },
    );
    assert.equal(loadReviewTab(storage), "stock");
  }
});
