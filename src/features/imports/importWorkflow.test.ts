// @ts-nocheck -- Runs directly in Node 26 without browser-focused Node typings.
import test from "node:test";
import assert from "node:assert/strict";
import { importSelectedRows } from "./useImportWizard.ts";

test("imports selected transaction rows chronologically and keeps partial failures", async () => {
  const rows = [
    { key: "late", selected: true, traded_at: "2026-08-02", stock_name: "Late" },
    { key: "skip", selected: false, traded_at: "2026-08-01", stock_name: "Skip" },
    { key: "early", selected: true, traded_at: "2026-08-01", stock_name: "Early" },
  ];
  const imported: string[] = [];
  const statuses: unknown[] = [];

  const result = await importSelectedRows(rows, {
    importRow: async (row) => {
      imported.push(row.key);
      if (row.key === "late") throw new Error("duplicate");
    },
    rowName: (row) => row.stock_name,
    compareRows: (a, b) => a.traded_at.localeCompare(b.traded_at),
    updateRow: (key, patch) => statuses.push({ key, patch }),
  });

  assert.deepEqual(imported, ["early", "late"]);
  assert.equal(result.success, 1);
  assert.equal(result.failed, 1);
  assert.equal(result.errors[0].name, "Late");
  assert.deepEqual(statuses.map((entry) => entry.key), ["early", "late"]);
});
