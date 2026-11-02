import type { Currency, Transaction } from "../types";

export interface TransactionPnl {
  currency: Currency;
  shares: number;
  remainingCost: number;
  realizedPnl: number;
  unrealizedPnl: number | null;
}

/** Replay one account/symbol using average acquisition cost, independent of display cost policies. */
export function calculateTransactionPnl(
  transactions: readonly Transaction[],
  currentPrice: number | null | undefined,
): TransactionPnl | null {
  if (transactions.length === 0) return null;
  // Match the backend position replay's traded_at, created_at, id order without reordering the table.
  const chronological = [...transactions].sort((a, b) => {
    for (const key of ["traded_at", "created_at", "id"] as const) {
      if (a[key] < b[key]) return -1;
      if (a[key] > b[key]) return 1;
    }
    return 0;
  });
  const { currency } = chronological[0];
  let shares = 0;
  let avgCost = 0;
  let realizedPnl = 0;

  for (const row of chronological) {
    if (row.currency !== currency || ![row.shares, row.price, row.total_amount, row.commission].every(Number.isFinite)
      || row.shares < 0) return null;

    switch (row.transaction_type) {
      case "OPEN":
        shares = row.shares;
        avgCost = row.price;
        break;
      case "BUY":
      case "STOCK_IN": {
        const nextShares = shares + row.shares;
        if (nextShares <= 0) return null;
        avgCost = (shares * avgCost + row.shares * row.price + row.commission) / nextShares;
        shares = nextShares;
        break;
      }
      case "SELL":
      case "STOCK_OUT": {
        let remaining = shares - row.shares;
        // Use the backend's relative tolerance: absorb arithmetic noise but retain real tiny positions.
        const tolerance = Math.min(4 * Number.EPSILON * Math.max(Math.abs(shares), Math.abs(row.shares)), 1e-9);
        if (remaining < -tolerance) return null;
        if (Math.abs(remaining) <= tolerance) remaining = 0;
        if (row.transaction_type === "SELL") {
          realizedPnl += row.total_amount - row.commission - row.shares * avgCost;
        }
        shares = remaining;
        if (shares === 0) avgCost = 0;
        break;
      }
      case "PAY":
        realizedPnl += row.total_amount - row.commission;
        break;
      default:
        return null;
    }
  }

  const remainingCost = shares * avgCost;
  const unrealizedPnl = shares === 0 ? 0
    : currentPrice != null && Number.isFinite(currentPrice) && currentPrice >= 0
      ? shares * currentPrice - remainingCost : null;
  return { currency, shares, remainingCost, realizedPnl, unrealizedPnl };
}
