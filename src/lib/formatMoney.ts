const CURRENCY_SYMBOLS: Record<string, string> = { USD: "$", CNY: "¥", HKD: "HK$" };

export function getCurrencySymbol(currency: string): string {
  return CURRENCY_SYMBOLS[currency] ?? currency;
}

export function formatMoney(value: number, currency: string, precision = 2): string {
  return `${getCurrencySymbol(currency)}${value.toLocaleString("en-US", {
    minimumFractionDigits: precision,
    maximumFractionDigits: precision,
  })}`;
}

const INTEGER_SHARE_SYMBOLS = new Set(["$CASH-USD", "$CASH-CNY", "$CASH-HKD"]);

export function formatHoldingShares(value: number, symbol: string): string {
  if (INTEGER_SHARE_SYMBOLS.has(symbol)) {
    return value.toLocaleString("en-US", { maximumFractionDigits: 0 });
  }
  return value.toLocaleString();
}
