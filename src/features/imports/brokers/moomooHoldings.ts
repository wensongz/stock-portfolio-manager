import type { Currency, Market } from "../../../types";
import { parseCsvNumber, splitCsvLine, stripBom } from "../csv.ts";
import type { HoldingImportRow, ParseResult } from "../types.ts";

function formatSymbol(symbol: string, market: Market): string {
  if (market === "HK") {
    const digits = symbol.replace(/\D/g, "");
    if (digits) return `${Number.parseInt(digits, 10)}.HK`;
  }
  return symbol.toUpperCase();
}

export function parseMoomooHoldings(text: string, accountMarket: Market): ParseResult<HoldingImportRow> {
  const lines = stripBom(text).split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const headers = splitCsvLine(lines[i]).map((field) => field.trim());
    if (!headers.includes("代码") || !headers.includes("持有数量") || !headers.includes("摊薄成本价")) continue;
    const codeIndex = headers.indexOf("代码");
    const nameIndex = headers.indexOf("名称");
    const quantityIndex = headers.indexOf("持有数量");
    const costIndex = headers.indexOf("摊薄成本价");
    const currencyIndex = headers.indexOf("币种");
    const rows: HoldingImportRow[] = [];
    for (let j = i + 1; j < lines.length; j++) {
      if (!lines[j].trim()) continue;
      const fields = splitCsvLine(lines[j]);
      const raw = (fields[codeIndex] ?? "").trim();
      const shares = parseCsvNumber(fields[quantityIndex]);
      const avgCost = parseCsvNumber(fields[costIndex]);
      if (!raw || Number.isNaN(shares) || shares <= 0 || Number.isNaN(avgCost)) continue;
      const currencyText = currencyIndex === -1 ? "" : (fields[currencyIndex] ?? "").trim().toUpperCase();
      const currency: Currency = currencyText === "HKD" ? "HKD"
        : currencyText === "USD" ? "USD"
        : currencyText === "CNY" || currencyText === "CNH" ? "CNY"
        : accountMarket === "HK" ? "HKD" : "USD";
      const market: Market = currency === "HKD" ? "HK"
        : currency === "CNY" ? "CN"
        : accountMarket === "HK" ? "US" : accountMarket;
      rows.push({
        key: String(rows.length), selected: true, symbol: formatSymbol(raw, market),
        name: (nameIndex === -1 ? "" : fields[nameIndex] ?? "").trim() || raw,
        shares, avgCost, currency, market,
      });
    }
    if (rows.length) return { rows, warnings: [] };
  }
  return { rows: [], warnings: ["未找到持仓数据。请确认上传的 CSV 是 Moomoo 客户端导出的持仓文件，且包含「代码、持有数量、摊薄成本价」列"] };
}
