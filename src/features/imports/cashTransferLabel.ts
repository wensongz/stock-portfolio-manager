export function cashTransferLabel(symbol: unknown, transactionType: unknown): string | undefined {
  if (typeof symbol !== "string" || !symbol.startsWith("$CASH-")) return undefined;
  if (transactionType === "BUY") return "存入现金";
  if (transactionType === "SELL") return "提取现金";
  return undefined;
}
