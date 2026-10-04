import type { Market } from "../../../types";
import { splitCsvLine, stripBom } from "../csv.ts";
import { isImportSummary, parseImportNumber, recordImportIssue } from "../parseDiagnostics.ts";
import type { HoldingImportRow, ImportParseIssue, ParseResult } from "../types.ts";
import { formatBrokerSymbol } from "./symbol.ts";

const SUMMARY = /^(Stocks|Bonds|Options|Futures|Forex|Total|USD|HKD|CNY|EUR|GBP|JPY|CAD|AUD|CHF|NZD|SGD)$/i;

function parseTable(lines: string[], headerIndex: number, market: Market, structured: boolean,
  rows: HoldingImportRow[], issues: ImportParseIssue[]): void {
  const headers = splitCsvLine(lines[headerIndex]).map((field) => field.trim());
  const symbolIndex = headers.indexOf("Symbol");
  const quantityIndex = headers.indexOf("Quantity");
  const costIndex = headers.indexOf("Cost Price") !== -1 ? headers.indexOf("Cost Price") : headers.indexOf("Avg Cost");
  const categoryIndex = headers.indexOf("Asset Category");
  if ([symbolIndex, quantityIndex, costIndex].includes(-1)) return;
  for (let i = headerIndex + 1; i < lines.length; i++) {
    if (!lines[i].trim()) continue;
    const fields = splitCsvLine(lines[i]);
    if (structured) {
      if (fields[0]?.trim() !== "Open Positions" || fields[1]?.trim() === "Header") break;
      if (fields[1]?.trim() !== "Data") continue;
      const category = categoryIndex === -1 ? "" : (fields[categoryIndex] ?? "").trim();
      if (category && !/^(Stocks?|Equities|股票)$/i.test(category)) continue;
    } else if (fields[symbolIndex]?.trim() === "Symbol") {
      break;
    }
    const raw = (fields[symbolIndex] ?? "").trim();
    if (SUMMARY.test(raw) || isImportSummary(raw)) continue;
    if (!raw && !fields[quantityIndex]?.trim() && !fields[costIndex]?.trim()) continue;
    const validSymbol = /^[A-Za-z0-9][A-Za-z0-9._/-]*(?: [A-Za-z0-9]{1,2})?$/.test(raw)
      && (market !== "HK" || /\d/.test(raw));
    if (!structured && !validSymbol && fields.filter((field) => field.trim()).length === 1) continue;
    const shares = parseImportNumber(fields[quantityIndex]);
    if (Number.isFinite(shares) && shares <= 0) continue;
    const avgCost = parseImportNumber(fields[costIndex]);
    const messages: string[] = [];
    if (!raw) messages.push("缺少证券代码");
    else if (!validSymbol) messages.push("证券代码格式无效");
    if (!Number.isFinite(shares)) messages.push("持仓数量缺失或不是有效数字");
    if (!Number.isFinite(avgCost)) messages.push("成本价缺失或不是有效数字");
    if (messages.length) {
      recordImportIssue(issues, i + 1, lines[i], messages);
      continue;
    }
    rows.push({ key: String(rows.length), raw: lines[i], selected: true, symbol: formatBrokerSymbol(raw, market), name: raw, shares, avgCost });
  }
}

export function parseIbHoldings(text: string, market: Market): ParseResult<HoldingImportRow> {
  const lines = stripBom(text).split(/\r?\n/);
  const rows: HoldingImportRow[] = [];
  const issues: ImportParseIssue[] = [];
  let structured = false;
  for (let i = 0; i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]);
    if (fields[0]?.trim() === "Open Positions" && fields[1]?.trim() === "Header") {
      structured = true;
      parseTable(lines, i, market, true, rows, issues);
    }
  }
  for (let i = 0; !structured && i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]).map((field) => field.trim());
    if (fields.includes("Symbol") && fields.includes("Quantity") && (fields.includes("Cost Price") || fields.includes("Avg Cost"))) {
      parseTable(lines, i, market, false, rows, issues);
    }
  }
  return { rows, warnings: rows.length ? [] : ["未找到持仓数据。请确认 CSV 格式符合要求：IB 活动报表 CSV（含 Open Positions 段落），或包含 Symbol、Quantity、Cost Price 列的扁平表格"],
    ...(issues.length ? { issues } : {}) };
}
