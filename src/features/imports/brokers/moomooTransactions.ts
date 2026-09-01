import dayjs from "dayjs";
import type { Market } from "../../../types";
import { parseCsvNumber, splitCsvLine, stripBom } from "../csv.ts";
import type { TransactionImportRow } from "../types.ts";

function formatSymbol(code: string, market: Market): string {
  const value = code.trim();
  if (market === "HK") {
    const digits = value.replace(/\D/g, "");
    if (digits) return `${Number.parseInt(digits, 10)}.HK`;
  }
  return value.toUpperCase();
}

function parseDate(raw: string): string {
  const match = raw.trim().match(/^(\d{4})\/(\d{2})\/(\d{2})(?:\s+(\d{2}:\d{2}(?::\d{2})?))?/);
  if (match) {
    const time = match[4] ? (match[4].length === 5 ? `${match[4]}:00` : match[4]) : "09:30:00";
    return `${match[1]}-${match[2]}-${match[3]}T${time}`;
  }
  const parsed = dayjs(raw.trim());
  return parsed.isValid() ? parsed.format("YYYY-MM-DDTHH:mm:ss") : "";
}

function detectMarket(value: string, fallback: Market): Market {
  const normalized = value.trim().toUpperCase();
  if (normalized.includes("港") || normalized.includes("HK")) return "HK";
  if (normalized.includes("美") || normalized.includes("US")) return "US";
  if (normalized.includes("A股") || normalized.includes("沪") || normalized.includes("深") || normalized.includes("CN")) return "CN";
  return fallback;
}

export function parseMoomooTransactions(text: string, defaultMarket: Market): TransactionImportRow[] {
  const lines = stripBom(text).split(/\r?\n/);
  const headerIndex = lines.findIndex((line) => splitCsvLine(line)[0]?.trim() === "方向");
  if (headerIndex === -1) return [];
  const headers = splitCsvLine(lines[headerIndex]).map((field) => field.trim());
  const column = (name: string) => headers.indexOf(name);
  const directionIndex = column("方向");
  const codeIndex = column("代码");
  const nameIndex = column("名称");
  const marketIndex = column("市场");
  const sharesIndex = column("成交数量");
  const priceIndex = column("成交价格");
  const amountIndex = column("成交金额");
  const timeIndex = column("成交时间");
  const commissionIndex = column("合计费用") !== -1 ? column("合计费用") : column("合计手续费");
  if ([codeIndex, sharesIndex, priceIndex].includes(-1)) return [];

  interface Fill { shares: number; price: number; amount: number; time: string; commission: number }
  interface Group { direction: string; code: string; name: string; market: Market; fills: Fill[] }
  const rows: TransactionImportRow[] = [];
  let group: Group | null = null;
  let key = 0;
  const finalize = () => {
    if (!group || group.fills.length === 0) return;
    const shares = group.fills.reduce((sum, fill) => sum + fill.shares, 0);
    const amount = group.fills.reduce((sum, fill) => sum + fill.amount, 0);
    rows.push({
      key: String(key++), selected: true, transaction_type: group.direction, stock_name: group.name,
      symbol: formatSymbol(group.code, group.market), traded_at: group.fills[0].time,
      price: Math.round((shares > 0 ? amount / shares : group.fills[0].price) * 10_000) / 10_000,
      shares, total_amount: Math.round(amount * 100) / 100,
      commission: Math.round(group.fills.reduce((sum, fill) => sum + fill.commission, 0) * 100) / 100,
    });
  };

  for (let i = headerIndex + 1; i < lines.length; i++) {
    if (!lines[i].trim()) continue;
    const fields = splitCsvLine(lines[i]);
    const direction = (fields[directionIndex] ?? "").trim();
    const main = direction === "买入" || direction === "卖出";
    const child = direction === "" && group !== null;
    if (!main && !child) continue;
    const shares = parseCsvNumber(fields[sharesIndex]);
    const price = parseCsvNumber(fields[priceIndex]);
    if (Number.isNaN(shares) || Number.isNaN(price)) continue;
    const amount = parseCsvNumber(fields[amountIndex]);
    const commission = parseCsvNumber(fields[commissionIndex]);
    const fill = {
      shares: Math.abs(shares), price: Math.abs(price),
      amount: Math.abs(Number.isNaN(amount) ? price * shares : amount),
      time: parseDate(fields[timeIndex] ?? ""),
      commission: Number.isNaN(commission) ? 0 : Math.abs(commission),
    };
    if (main) {
      const code = (fields[codeIndex] ?? "").trim();
      if (!code) continue;
      finalize();
      const marketText = marketIndex === -1 ? "" : fields[marketIndex] ?? "";
      group = {
        direction: direction === "卖出" ? "SELL" : "BUY", code,
        name: (nameIndex === -1 ? "" : fields[nameIndex] ?? "").trim() || code,
        market: marketText ? detectMarket(marketText, defaultMarket) : defaultMarket,
        fills: [fill],
      };
    } else {
      group!.fills.push(fill);
    }
  }
  finalize();
  return rows;
}
