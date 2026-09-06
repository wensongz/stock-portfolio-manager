import type { Market } from "../../../types";
import { parseCsvNumber, splitCsvLine, stripBom } from "../csv.ts";
import type { HoldingImportRow, ParseResult } from "../types.ts";

const SUMMARY = /^(Stocks|Bonds|Options|Futures|Forex|Total|USD|HKD|CNY|EUR|GBP|JPY|CAD|AUD|CHF|NZD|SGD)$/i;

function formatSymbol(symbol: string, market: Market): string {
  if (market === "HK") {
    const digits = symbol.replace(/\D/g, "");
    if (digits) return `${Number.parseInt(digits, 10)}.HK`;
  }
  return symbol.toUpperCase();
}

function parseTable(lines: string[], headerIndex: number, market: Market, structured: boolean): HoldingImportRow[] {
  const headers = splitCsvLine(lines[headerIndex]).map((field) => field.trim());
  const symbolIndex = headers.indexOf("Symbol");
  const quantityIndex = headers.indexOf("Quantity");
  const costIndex = headers.indexOf("Cost Price") !== -1 ? headers.indexOf("Cost Price") : headers.indexOf("Avg Cost");
  if ([symbolIndex, quantityIndex, costIndex].includes(-1)) return [];
  const rows: HoldingImportRow[] = [];
  for (let i = headerIndex + 1; i < lines.length; i++) {
    if (!lines[i].trim()) continue;
    const fields = splitCsvLine(lines[i]);
    if (structured && (fields[0]?.trim() !== "Open Positions" || fields[1]?.trim() !== "Data")) continue;
    const raw = (fields[symbolIndex] ?? "").trim();
    if (!raw || SUMMARY.test(raw)) continue;
    const shares = parseCsvNumber(fields[quantityIndex]);
    const avgCost = parseCsvNumber(fields[costIndex]);
    if (Number.isNaN(shares) || shares <= 0 || Number.isNaN(avgCost)) continue;
    rows.push({ key: String(rows.length), raw: lines[i], selected: true, symbol: formatSymbol(raw, market), name: raw, shares, avgCost });
  }
  return rows;
}

export function parseIbHoldings(text: string, market: Market): ParseResult<HoldingImportRow> {
  const lines = stripBom(text).split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]);
    if (fields[0]?.trim() === "Open Positions" && fields[1]?.trim() === "Header") {
      const rows = parseTable(lines, i, market, true);
      if (rows.length) return { rows, warnings: [] };
    }
  }
  for (let i = 0; i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]).map((field) => field.trim());
    if (fields.includes("Symbol") && fields.includes("Quantity") && (fields.includes("Cost Price") || fields.includes("Avg Cost"))) {
      const rows = parseTable(lines, i, market, false);
      if (rows.length) return { rows, warnings: [] };
    }
  }
  return { rows: [], warnings: ["未找到持仓数据。请确认 CSV 格式符合要求：IB 活动报表 CSV（含 Open Positions 段落），或包含 Symbol、Quantity、Cost Price 列的扁平表格"] };
}
