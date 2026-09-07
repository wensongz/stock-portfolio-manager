import type { CashBalanceReconciliation, CorrectCashBalancePayload, Holding, HoldingWithQuote, UpdateHoldingPayload } from "../../types";
import { formatMoney } from "../../lib/formatMoney.ts";

export interface HoldingRequestState<T> {
  holdingId: string | null;
  status: "idle" | "loading" | "ready" | "error";
  data: T | null;
  error: string | null;
}

export function emptyHoldingRequest<T>(): HoldingRequestState<T> {
  return { holdingId: null, status: "idle", data: null, error: null };
}

/** Each modal has an independent request lane; closing also invalidates pending work. */
export function createHoldingRequest<T>(publish: (state: HoldingRequestState<T>) => void) {
  let generation = 0;
  let activeHoldingId: string | null = null;
  return {
    invalidate() { generation += 1; activeHoldingId = null; },
    clear() { this.invalidate(); publish(emptyHoldingRequest()); },
    async load(holdingId: string, read: () => Promise<T>) {
      const request = ++generation;
      activeHoldingId = holdingId;
      publish({ holdingId, status: "loading", data: null, error: null });
      const current = () => request === generation && holdingId === activeHoldingId;
      try {
        const data = await read();
        if (current()) publish({ holdingId, status: "ready", data, error: null });
      } catch (error) {
        if (current()) publish({ holdingId, status: "error", data: null, error: String(error) });
      }
    },
  };
}

export function cashBalanceEditDecision(
  holding: Holding,
  balance: number | null | undefined,
  state: HoldingRequestState<CashBalanceReconciliation>,
) {
  const data = state.status === "ready" && state.holdingId === holding.id &&
    state.data?.holding_id === holding.id ? state.data : null;
  const canAdopt = data?.recommended_balance != null && Number.isFinite(data.recommended_balance);
  const balanceChanged = balance !== holding.shares || (data !== null && balance !== data.current_balance);
  let reason: string | null = null;
  if (typeof balance !== "number" || !Number.isFinite(balance)) reason = "请输入有效现金余额，可为零或负数。";
  else if (balanceChanged && !data) reason = "余额核对尚未完成，暂不能保存余额变动。可重试，或恢复原余额后仅保存名称和类别。";
  else if (balanceChanged && data && data.opening_count > 1 &&
    (data.recommended_balance === null || !matchesRecommendedAmount(balance, data.recommended_balance))) {
    reason = "存在多条现金期初，自定义余额前请先整理期初记录；仍可采用推荐值。";
  }
  return { balanceChanged, canSubmit: reason === null, canAdopt, reason, data };
}

function matchesRecommendedAmount(balance: number, recommended: number): boolean {
  // Rust f64::round rounds halfway values away from zero, including debit cash.
  const cents = (value: number) => Math.sign(value) * Math.round(Math.abs(value) * 100);
  return Math.abs(balance - recommended) <= 1e-8 || Math.abs(cents(balance) - cents(recommended)) <= 1e-8;
}

export function cashBalanceSaveCommand(
  holding: Holding,
  values: Pick<UpdateHoldingPayload, "shares" | "name" | "categoryId">,
  state: HoldingRequestState<CashBalanceReconciliation>,
): { kind: "metadata"; payload: UpdateHoldingPayload } | { kind: "correction"; payload: CorrectCashBalancePayload } {
  const decision = cashBalanceEditDecision(holding, values.shares, state);
  if (!decision.canSubmit) throw new Error(decision.reason!);
  if (decision.balanceChanged && decision.data) {
    return { kind: "correction", payload: {
      id: holding.id, balance: values.shares, expectedRevision: decision.data.revision,
      name: values.name, categoryId: values.categoryId,
    } };
  }
  return { kind: "metadata", payload: {
    id: holding.id, accountId: holding.account_id, symbol: holding.symbol,
    market: holding.market, currency: holding.currency, shares: holding.shares,
    avgCost: holding.avg_cost, name: values.name, categoryId: values.categoryId,
  } };
}

/** A synchronous guard also catches a second submit before React renders loading. */
export function createEditSession() {
  let generation = 0;
  let saving = false;
  let active = false;
  return {
    open() { generation += 1; saving = false; active = true; },
    close() { generation += 1; saving = false; active = false; },
    beginSave() {
      if (!active || saving) return null;
      saving = true;
      return generation;
    },
    isCurrent(token: number) { return token === generation; },
    finishSave(token: number) {
      if (token !== generation) return false;
      saving = false;
      return true;
    },
  };
}

export function mergeHoldingQuote(holding: Holding, cached?: HoldingWithQuote): HoldingWithQuote {
  if (holding.symbol.startsWith("$CASH-")) {
    return {
      ...holding, avg_cost: 1, quote: cached?.quote ?? null,
      market_value: holding.shares, total_cost: holding.shares,
      unrealized_pnl: 0, unrealized_pnl_percent: 0,
    };
  }
  return cached ?? {
    ...holding, quote: null, market_value: null, total_cost: null,
    unrealized_pnl: null, unrealized_pnl_percent: null,
  };
}

export function formatCashDelta(value: number, currency: string): string {
  return `${value >= 0 ? "+" : "-"}${formatMoney(Math.abs(value), currency)}`;
}
