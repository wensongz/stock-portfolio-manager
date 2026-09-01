import { parseCsvNumber, splitCsvLine, stripBom } from "../csv.ts";
import type { HoldingImportRow, ParseResult } from "../types.ts";

export function parseFirstradeHoldings(text: string): ParseResult<HoldingImportRow> {
  const lines = stripBom(text).split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const headers = splitCsvLine(lines[i]).map((field) => field.trim());
    if (!headers.includes("代号") || !headers.includes("股数") || !headers.includes("单位成本")) continue;
    const symbolIndex = headers.indexOf("代号");
    const quantityIndex = headers.indexOf("股数");
    const costIndex = headers.indexOf("单位成本");
    const nameIndex = headers.indexOf("名称");
    const rows: HoldingImportRow[] = [];
    for (let j = i + 1; j < lines.length; j++) {
      if (!lines[j].trim()) continue;
      const fields = splitCsvLine(lines[j]);
      const raw = (fields[symbolIndex] ?? "").trim();
      const shares = parseCsvNumber(fields[quantityIndex]);
      const avgCost = parseCsvNumber(fields[costIndex]);
      if (!raw || /^(total|summary)$/i.test(raw) || !/[A-Za-z]/.test(raw)
        || Number.isNaN(shares) || shares <= 0 || Number.isNaN(avgCost)) continue;
      const symbol = raw.toUpperCase();
      rows.push({
        key: String(rows.length), selected: true, symbol,
        name: (nameIndex === -1 ? "" : fields[nameIndex] ?? "").trim() || symbol,
        shares, avgCost,
      });
    }
    if (rows.length) return { rows, warnings: [] };
  }
  return { rows: [], warnings: ["未找到持仓数据。请确认 CSV 来自 Firstrade 持仓页面，且包含「代号、股数、单位成本」列"] };
}
