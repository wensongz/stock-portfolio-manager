import type { Market } from "../../../types";

/** Normalize broker codes, including numeric symbols formatted by spreadsheets. */
export function formatBrokerSymbol(symbol: string, market: Market): string {
  const value = symbol.trim().toUpperCase();
  if (market === "US") {
    // Broker share-class aliases share one storage key; the raw CSV is kept separately.
    return value.replace(/^([A-Z][A-Z0-9]*)(?:[._-]|\s+)([A-Z])$/, "$1-$2");
  }
  if (market === "HK") {
    // Strip an all-zero decimal part before removing formatting characters;
    // otherwise Excel's "133.00" would become the different stock "13300.HK".
    const integerSymbol = value.replace(/^(\d+)\.0+(?=\.HK$|$)/, "$1");
    const digits = integerSymbol.replace(/\D/g, "");
    if (digits) return `${Number.parseInt(digits, 10)}.HK`;
  }
  return value;
}
