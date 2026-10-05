import type { Currency, Market } from "../../../types";
import { splitCsvLine, stripBom } from "../csv.ts";
import { isImportSummary, parseImportNumber, recordImportIssue } from "../parseDiagnostics.ts";
import type { HoldingImportRow, ImportParseIssue, ParseResult } from "../types.ts";
import { formatBrokerSymbol } from "./symbol.ts";

export function parseMoomooHoldings(text: string, accountMarket: Market): ParseResult<HoldingImportRow> {
  const lines = stripBom(text).split(/\r?\n/);
  const rows: HoldingImportRow[] = [];
  const issues: ImportParseIssue[] = [];
  for (let i = 0; i < lines.length; i++) {
    const headers = splitCsvLine(lines[i]).map((field) => field.trim());
    if (!headers.includes("代码") || !headers.includes("持有数量") || !headers.includes("摊薄成本价")) continue;
    const codeIndex = headers.indexOf("代码");
    const nameIndex = headers.indexOf("名称");
    const quantityIndex = headers.indexOf("持有数量");
    const costIndex = headers.indexOf("摊薄成本价");
    const currencyIndex = headers.indexOf("币种");
    for (let j = i + 1; j < lines.length; j++) {
      if (!lines[j].trim()) continue;
      const fields = splitCsvLine(lines[j]);
      const raw = (fields[codeIndex] ?? "").trim();
      if (raw === "代码") break;
      const name = (nameIndex === -1 ? "" : fields[nameIndex] ?? "").trim();
      if (isImportSummary(raw || name) || /^(美股|港股|沪深股|股票|现金|人民币|美元|港元)$/.test(raw)) continue;
      if (/^(USD|HKD|CNY|CNH)$/i.test(raw) && !fields[quantityIndex]?.trim() && !fields[costIndex]?.trim()) continue;
      if (!raw && !name && !fields[quantityIndex]?.trim() && !fields[costIndex]?.trim()) continue;
      if (!/^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(raw) && fields.filter((field) => field.trim()).length === 1) continue;
      const shares = parseImportNumber(fields[quantityIndex]);
      if (Number.isFinite(shares) && shares <= 0) continue;
      const avgCost = parseImportNumber(fields[costIndex]);
      const currencyText = currencyIndex === -1 ? "" : (fields[currencyIndex] ?? "").trim().toUpperCase();
      const currency: Currency = currencyText === "HKD" ? "HKD"
        : currencyText === "USD" ? "USD"
        : currencyText === "CNY" || currencyText === "CNH" ? "CNY"
        : accountMarket === "HK" ? "HKD" : "USD";
      const market: Market = currency === "HKD" ? "HK"
        : currency === "CNY" ? "CN"
        : accountMarket === "HK" ? "US" : accountMarket;
      const symbol = formatBrokerSymbol(raw, market);
      const messages: string[] = [];
      if (!raw) messages.push("缺少证券代码");
      else if (!/^[A-Za-z0-9][A-Za-z0-9._/-]*$/.test(market === "US" ? symbol : raw) || (market === "HK" && !/\d/.test(raw))) messages.push("证券代码格式无效");
      if (!Number.isFinite(shares)) messages.push("持仓数量缺失或不是有效数字");
      if (!Number.isFinite(avgCost)) messages.push("成本价缺失或不是有效数字");
      if (messages.length) {
        recordImportIssue(issues, j + 1, lines[j], messages);
        continue;
      }
      rows.push({
        key: String(rows.length), raw: lines[j], selected: true, symbol,
        name: name || raw,
        shares, avgCost, currency, market,
      });
    }
  }
  return { rows, warnings: rows.length ? [] : ["未找到持仓数据。请确认上传的 CSV 是 Moomoo 客户端导出的持仓文件，且包含「代码、持有数量、摊薄成本价」列"],
    ...(issues.length ? { issues } : {}) };
}
