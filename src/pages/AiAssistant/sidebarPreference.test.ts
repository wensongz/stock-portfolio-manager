// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  AI_SIDEBAR_COLLAPSED_STORAGE_KEY,
  loadAiSidebarCollapsed,
  saveAiSidebarCollapsed,
} from "./sidebarPreference.ts";

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

test("AI assistant remembers the latest chat sidebar state", () => {
  const storage = memoryStorage();

  saveAiSidebarCollapsed(storage, false);

  assert.equal(storage.getItem(AI_SIDEBAR_COLLAPSED_STORAGE_KEY), "false");
  assert.equal(loadAiSidebarCollapsed(storage), false);

  saveAiSidebarCollapsed(storage, true);

  assert.equal(storage.getItem(AI_SIDEBAR_COLLAPSED_STORAGE_KEY), "true");
  assert.equal(loadAiSidebarCollapsed(storage), true);
});

test("AI assistant defaults to a collapsed chat sidebar for missing or invalid data", () => {
  for (const value of [undefined, "", "yes", "0"]) {
    const storage = memoryStorage(
      value === undefined
        ? {}
        : { [AI_SIDEBAR_COLLAPSED_STORAGE_KEY]: value },
    );

    assert.equal(loadAiSidebarCollapsed(storage), true);
  }
});
