import { splitCsvLine, stripBom } from "../csv.ts";
import { isImportSummary, parseImportNumber, recordImportIssue } from "../parseDiagnostics.ts";
import type { HoldingImportRow, ImportParseIssue, ParseResult } from "../types.ts";
import { formatBrokerSymbol } from "./symbol.ts";

export function parseFirstradeHoldings(text: string): ParseResult<HoldingImportRow> {
  const lines = stripBom(text).split(/\r?\n/);
  const rows: HoldingImportRow[] = [];
  const issues: ImportParseIssue[] = [];
  for (let i = 0; i < lines.length; i++) {
    const headers = splitCsvLine(lines[i]).map((field) => field.trim());
    if (!headers.includes("代号") || !headers.includes("股数") || !headers.includes("单位成本")) continue;
    const symbolIndex = headers.indexOf("代号");
    const quantityIndex = headers.indexOf("股数");
    const costIndex = headers.indexOf("单位成本");
    const nameIndex = headers.indexOf("名称");
    for (let j = i + 1; j < lines.length; j++) {
      if (!lines[j].trim()) continue;
      const fields = splitCsvLine(lines[j]);
      const raw = (fields[symbolIndex] ?? "").trim();
      if (raw === "代号") break;
      const name = (nameIndex === -1 ? "" : fields[nameIndex] ?? "").trim();
      if (isImportSummary(raw || name) || /^(美元|股票|现金)$/.test(raw)) continue;
      if (/^(Stocks|Bonds|Options|Cash|USD)$/i.test(raw) && !fields[quantityIndex]?.trim() && !fields[costIndex]?.trim()) continue;
      if (!raw && !name && !fields[quantityIndex]?.trim() && !fields[costIndex]?.trim()) continue;
      const symbol = formatBrokerSymbol(raw, "US");
      if (!/^[A-Z][A-Z0-9._/-]*$/.test(symbol) && fields.filter((field) => field.trim()).length === 1) continue;
      const shares = parseImportNumber(fields[quantityIndex]);
      if (Number.isFinite(shares) && shares <= 0) continue;
      const avgCost = parseImportNumber(fields[costIndex]);
      const messages: string[] = [];
      if (!raw) messages.push("缺少证券代码");
      else if (!/^[A-Z][A-Z0-9._/-]*$/.test(symbol)) messages.push("证券代码格式无效");
      if (!Number.isFinite(shares)) messages.push("持仓数量缺失或不是有效数字");
      if (!Number.isFinite(avgCost)) messages.push("成本价缺失或不是有效数字");
      if (messages.length) {
        recordImportIssue(issues, j + 1, lines[j], messages);
        continue;
      }
      rows.push({
        key: String(rows.length), raw: lines[j], selected: true, symbol,
        name: name || symbol,
        shares, avgCost,
      });
    }
  }
  return { rows, warnings: rows.length ? [] : ["未找到持仓数据。请确认 CSV 来自 Firstrade 持仓页面，且包含「代号、股数、单位成本」列"],
    ...(issues.length ? { issues } : {}) };
}
