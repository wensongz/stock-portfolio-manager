import type { QuarterlyHoldingHistory, QuarterlyHoldingHistoryRow, QuarterlyHoldingSnapshot } from "../../types";
import { formatMoney } from "../../lib/formatMoney.ts";

type EditorMode = "edit" | "history";
interface EditorTarget {
  snapshotId: string;
  holdingSnapshotId: string;
  notes: string;
  showHistory: boolean;
}
export interface HoldingHistoryEditorState {
  mode: EditorMode;
  notes: string;
  history: QuarterlyHoldingHistory | null;
  historyLoading: boolean;
  historyError: string | null;
  saving: boolean;
  saveError: string | null;
}

export function initialHoldingHistoryState(showHistory = false): HoldingHistoryEditorState {
  return { mode: showHistory ? "history" : "edit", notes: "", history: null, historyLoading: false, historyError: null, saving: false, saveError: null };
}

/** Request and save lifetimes belong to one account holding in one snapshot. */
export function createHoldingHistoryEditor(
  invokeFn: <T>(command: string, args: Record<string, unknown>) => Promise<T>,
  saveNotes: (snapshotId: string, holdingSnapshotId: string, notes: string) => Promise<void>,
  publish: (state: HoldingHistoryEditorState) => void,
) {
  let target: EditorTarget | null = null;
  let generation = 0;
  let historyRequest = 0;
  let state = initialHoldingHistoryState();
  const patch = (next: Partial<HoldingHistoryEditorState>) => { state = { ...state, ...next }; publish(state); };
  const invalidate = () => { target = null; generation += 1; historyRequest += 1; };
  const loadHistory = async () => {
    if (!target || state.mode !== "history") return;
    const selected = target;
    const scope = generation;
    const request = ++historyRequest;
    patch({ history: null, historyLoading: true, historyError: null });
    const current = () => target?.snapshotId === selected.snapshotId && target.holdingSnapshotId === selected.holdingSnapshotId && scope === generation && request === historyRequest;
    try {
      const data = await invokeFn<QuarterlyHoldingHistory>("get_quarterly_holding_history", {
        snapshotId: selected.snapshotId, holdingSnapshotId: selected.holdingSnapshotId,
      });
      if (!current()) return;
      if (data.snapshot_id !== selected.snapshotId || data.holding_snapshot_id !== selected.holdingSnapshotId) throw new Error("操作记录与当前持仓快照不匹配，请重试。");
      patch({ history: data, historyLoading: false });
    } catch (error) {
      if (current()) patch({ history: null, historyLoading: false, historyError: String(error) });
    }
  };
  return {
    async open(next: EditorTarget) {
      invalidate();
      target = next;
      state = { ...initialHoldingHistoryState(next.showHistory), notes: next.notes };
      publish(state);
      if (next.showHistory) await loadHistory();
    },
    close() { invalidate(); state = initialHoldingHistoryState(); publish(state); },
    dispose: invalidate,
    setNotes(notes: string) { if (target && !state.saving) patch({ notes }); },
    async setMode(mode: EditorMode) {
      if (!target) return;
      historyRequest += 1;
      patch({ mode, history: null, historyLoading: false, historyError: null });
      if (mode === "history") await loadHistory();
    },
    retry: loadHistory,
    async save(): Promise<boolean> {
      if (!target || state.saving) return false;
      const selected = target;
      const scope = generation;
      const notes = state.notes;
      patch({ saving: true, saveError: null });
      try {
        await saveNotes(selected.snapshotId, selected.holdingSnapshotId, notes);
        if (scope !== generation) return false;
        patch({ saving: false });
        return true;
      } catch (error) {
        if (scope === generation) patch({ saving: false, saveError: String(error) });
        return false;
      }
    },
  };
}

function formatUtcDate(value: string): string {
  const normalized = value.trim().replace(" ", "T");
  const timestamp = /^\d{4}-\d{2}-\d{2}$/.test(normalized) ? `${normalized}T00:00:00Z`
    : /(?:Z|[+-]\d{2}:?\d{2})$/i.test(normalized) ? normalized : `${normalized}Z`;
  const date = new Date(timestamp);
  return Number.isFinite(date.getTime()) ? date.toISOString().slice(0, 19).replace("T", " ") : "—";
}

export function formatQuarterlyOperation(row: QuarterlyHoldingHistoryRow) {
  const cash = row.symbol.toUpperCase().startsWith("$CASH-");
  const labels = { OPEN: "期初", BUY: cash ? "存入" : "买入", SELL: cash ? "提取" : "卖出", PAY: "分红", STOCK_IN: "存入股票", STOCK_OUT: "提取股票" };
  return {
    id: row.id,
    date: formatUtcDate(row.traded_at),
    type: labels[row.transaction_type],
    security: `${row.symbol} · ${row.name}`,
    shares: row.shares.toLocaleString("en-US", { maximumFractionDigits: 6 }),
    price: formatMoney(row.price, row.currency, 4),
    amount: formatMoney(row.total_amount, row.currency),
    commission: formatMoney(row.commission, row.currency),
    cashDelta: row.cash_delta == null ? "—" : `${row.cash_delta >= 0 ? "+" : "-"}${formatMoney(Math.abs(row.cash_delta), row.currency)}`,
    runningBalance: row.running_balance == null ? "—" : formatMoney(row.running_balance, row.currency),
    notes: row.notes || "—",
  };
}

export function isHoldingEditorTargetCurrent(
  target: { snapshotId: string; holding: Pick<QuarterlyHoldingSnapshot, "id"> } | null,
  snapshotId: string,
  holdings: Pick<QuarterlyHoldingSnapshot, "id" | "quarterly_snapshot_id">[],
): boolean {
  return target !== null && target.snapshotId === snapshotId && holdings.some((holding) => holding.id === target.holding.id && holding.quarterly_snapshot_id === snapshotId);
}
