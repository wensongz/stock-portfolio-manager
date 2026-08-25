// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  DEFAULT_TABLE_PAGE_SIZE,
  TABLE_PAGE_SIZE_STORAGE_KEY,
  loadTablePageSize,
  saveTablePageSize,
} from "./tablePageSize.ts";

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

test("table page size remembers the user's last valid selection", () => {
  const storage = memoryStorage();

  saveTablePageSize(storage, 50);

  assert.equal(storage.getItem(TABLE_PAGE_SIZE_STORAGE_KEY), "50");
  assert.equal(loadTablePageSize(storage), 50);
});

test("table page size falls back when persisted data is invalid", () => {
  for (const value of ["", "lots", "0", "-10", "20.5"]) {
    const storage = memoryStorage({ [TABLE_PAGE_SIZE_STORAGE_KEY]: value });
    assert.equal(loadTablePageSize(storage), DEFAULT_TABLE_PAGE_SIZE);
  }
});
