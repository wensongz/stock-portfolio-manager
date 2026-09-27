import type { Market } from "../../types";

const SH_PREFIXES = ["600", "601", "603", "605", "688", "900", "5"];
const SZ_PREFIXES = ["000", "001", "002", "003", "004", "300", "301", "200", "15", "16"];

/** Complete mainland exchange prefixes and bare Hong Kong stock codes. */
export function normalizeHoldingSymbol(symbol: string, market?: Market): string {
  const trimmed = symbol.trim();
  if (market === "HK" && /^\d{3,4}$/.test(trimmed)) {
    return `${Number.parseInt(trimmed, 10)}.HK`;
  }
  if (market !== "CN" || !/^\d{6}$/.test(trimmed)) return trimmed;
  if (SH_PREFIXES.some((prefix) => trimmed.startsWith(prefix))) return `sh${trimmed}`;
  if (SZ_PREFIXES.some((prefix) => trimmed.startsWith(prefix))) return `sz${trimmed}`;
  return trimmed;
}
