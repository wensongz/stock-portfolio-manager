import { parseCsvNumber, splitCsvLine, stripBom } from "../csv.ts";
import type { HoldingImportRow, ParseResult } from "../types.ts";

function deriveSymbol(code: string, exchange: string): string {
  if (exchange.includes("上海") || exchange.toUpperCase().startsWith("SH")) return `sh${code}`;
  if (exchange.includes("深圳") || exchange.toUpperCase().startsWith("SZ")) return `sz${code}`;
  return code.startsWith("6") || code.startsWith("5") ? `sh${code}` : `sz${code}`;
}

export function parseCnHoldings(text: string): ParseResult<HoldingImportRow> {
  const lines = stripBom(text).split(/\r?\n/);
  let cashRaw = "";
  let cashAmount: number | undefined;
  let headerIndex = -1;

  for (let i = 0; i < Math.min(lines.length, 20); i++) {
    if (!lines[i].trim()) continue;
    const fields = splitCsvLine(lines[i]).map((field) => field.trim());
    if (fields.includes("市种") && fields.includes("可用")) {
      const availableIndex = fields.indexOf("可用");
      for (let j = i + 1; j < Math.min(lines.length, i + 5); j++) {
        const data = splitCsvLine(lines[j]).map((field) => field.trim());
        const rmbIndex = data.indexOf("人民币");
        if (rmbIndex !== -1) {
          const value = parseCsvNumber(data[availableIndex + rmbIndex]);
          if (!Number.isNaN(value) && value > 0) { cashAmount = value; cashRaw = lines[j]; }
          break;
        }
      }
    }
    if (fields.includes("证券代码") && fields.includes("证券名称")) {
      headerIndex = i;
      break;
    }
  }
  if (headerIndex === -1) {
    headerIndex = lines.findIndex((line) => {
      const fields = splitCsvLine(line).map((field) => field.trim());
      return fields.includes("证券代码") && fields.includes("证券名称");
    });
  }
  if (headerIndex === -1) {
    return { rows: [], warnings: ["未找到持仓数据，请确认CSV格式是否正确（需含“证券代码”和“证券名称”列）"] };
  }

  const headers = splitCsvLine(lines[headerIndex]).map((field) => field.trim());
  const codeIndex = headers.indexOf("证券代码");
  const nameIndex = headers.indexOf("证券名称");
  const format1Shares = headers.indexOf("参考持股");
  const format1Cost = headers.indexOf("成本价");
  const format2Shares = headers.indexOf("持仓数量");
  const format2Cost = headers.indexOf("参考成本价");
  const fallbackShares = headers.indexOf("股票余额");
  let sharesIndex = -1;
  let costIndex = -1;
  let marketIndex = -1;
  if (format1Shares !== -1 && format1Cost !== -1) {
    sharesIndex = format1Shares;
    costIndex = format1Cost;
  } else if (format2Shares !== -1 && format2Cost !== -1) {
    sharesIndex = format2Shares;
    costIndex = format2Cost;
    marketIndex = headers.indexOf("交易市场");
  } else if (fallbackShares !== -1 && (format1Cost !== -1 || format2Cost !== -1)) {
    sharesIndex = fallbackShares;
    costIndex = format1Cost !== -1 ? format1Cost : format2Cost;
    marketIndex = headers.indexOf("交易市场");
  }
  if ([codeIndex, nameIndex, sharesIndex, costIndex].includes(-1)) {
    return { rows: [], warnings: ["无法识别CSV格式，请确认文件包含支持的证券代码、证券名称、持仓数量和成本价列"] };
  }

  const rows: HoldingImportRow[] = [];
  for (let i = headerIndex + 1; i < lines.length; i++) {
    if (!lines[i].trim()) continue;
    const fields = splitCsvLine(lines[i]);
    const rawCode = (fields[codeIndex] ?? "").trim();
    const code = /^\d+$/.test(rawCode) ? rawCode.padStart(6, "0") : rawCode;
    if (!/^\d{6}$/.test(code)) continue;
    const shares = parseCsvNumber(fields[sharesIndex]);
    const avgCost = parseCsvNumber(fields[costIndex]);
    if (Number.isNaN(shares) || shares <= 0 || Number.isNaN(avgCost)) continue;
    const exchange = marketIndex === -1 ? "" : (fields[marketIndex] ?? "").trim();
    const symbol = deriveSymbol(code, exchange);
    rows.push({
      key: String(rows.length), raw: lines[i], selected: true, isCash: false, symbol,
      name: (fields[nameIndex] ?? "").trim() || symbol, shares, avgCost,
    });
  }
  if (cashAmount !== undefined && cashAmount > 0) {
    rows.unshift({
      key: `cash-${rows.length}`, raw: cashRaw, selected: true, isCash: true, symbol: "$CASH-CNY",
      name: "现金 (CNY)", shares: cashAmount, avgCost: 1,
    });
  }
  return { rows, warnings: [] };
}
