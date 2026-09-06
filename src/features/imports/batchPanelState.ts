import type { ImportBatch } from "./batchTypes";

type Row = ImportBatch["rows"][number];
export const selectableBatchRow = (row: Row) => ["ready", "suspected", "failed"].includes(row.status);
export function initialBatchSelection(batch: ImportBatch): string[] {
  return batch.status === "undone" ? [] : batch.rows.filter((row) => row.status === "ready").map((row) => row.key);
}
export function batchApplyArgs(batch: ImportBatch, selected: readonly string[]) {
  const rows = batch.status === "undone" || batch.conflict ? [] : batch.rows.filter((row) => selected.includes(row.key) && selectableBatchRow(row));
  return { batchId: batch.id, rowKeys: rows.map((row) => row.key), allowSuspectedKeys: rows.filter((row) => row.status === "suspected").map((row) => row.key) };
}
export function selectionAfterBatchResponse(batch: ImportBatch, selected: readonly string[]): string[] {
  // A commit can newly classify a previously ready row as suspected.
  // Require a new explicit selection after every server response.
  const safeSelection = selected.filter((key) => !batch.rows.some((row) => row.key === key && row.status === "suspected"));
  return batchApplyArgs(batch, safeSelection).rowKeys;
}
export function batchReconciliationArgs(batch: ImportBatch, balances: Record<string, number | null>) {
  return {
    batchId: batch.id,
    balances: Object.entries(balances)
      .filter(([, expected]) => expected != null)
      .map(([symbol, expected_shares]) => ({symbol, expected_shares})),
  };
}

export function reconciliationMatches(row: ImportBatch["reconciliation"][number]): boolean {
  if (row.expected_shares == null || row.difference == null) return false;
  const tolerance = row.symbol.startsWith("$CASH-") ? 0.005 : 1e-8;
  return Math.abs(row.difference) < tolerance;
}

export function addExpectedBalance(
  batch: ImportBatch,
  balances: Record<string, number | null>,
  inputSymbol: string,
  expected: number | null,
): Record<string, number | null> {
  const symbol = inputSymbol.trim().toUpperCase();
  if (!symbol) throw new Error("请输入证券代码或现金代码");
  const existing = [...batch.reconciliation.map((row) => row.symbol), ...Object.keys(balances)];
  if (existing.some((value) => value.trim().toUpperCase() === symbol)) {
    throw new Error("此证券已存在，请直接编辑表格中的券商余额");
  }
  if (expected == null || !Number.isFinite(expected)) throw new Error("请输入有效数量或现金余额");
  return {...balances, [symbol]: expected};
}

export function reconciliationDisplayRows(batch: ImportBatch, balances: Record<string, number | null>): ImportBatch["reconciliation"] {
  const existing = new Set(batch.reconciliation.map((row) => row.symbol));
  const additions = Object.keys(balances).filter((symbol) => !existing.has(symbol)).map((symbol) => ({
    symbol,
    currency: symbol.startsWith("$CASH-") ? symbol.slice(6) : "",
    before_shares: 0,
    after_shares: 0,
    expected_shares: null,
    difference: null,
  }));
  return [...batch.reconciliation, ...additions];
}
