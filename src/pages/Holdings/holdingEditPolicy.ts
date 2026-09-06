import type { Holding, Transaction } from "../../types";

export function canEditOpening(holding: Holding, transactions: Transaction[]): boolean {
  const cash = holding.symbol.startsWith("$CASH-");
  const relevant = transactions.filter((transaction) =>
    transaction.holding_id === holding.id ||
    (transaction.account_id === holding.account_id && (
      transaction.symbol.toUpperCase() === holding.symbol.toUpperCase() ||
      (cash && transaction.currency === holding.currency &&
        ["BUY", "SELL", "PAY"].includes(transaction.transaction_type))
    ))
  );
  return relevant.length === 0 ||
    (relevant.length === 1 && relevant[0].transaction_type === "OPEN");
}
