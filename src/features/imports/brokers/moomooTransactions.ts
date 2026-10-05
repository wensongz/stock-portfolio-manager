import dayjs from "dayjs";
import type { Market } from "../../../types";
import { splitCsvLine, stripBom } from "../csv.ts";
import type { ImportParseIssue, TransactionImportRow } from "../types.ts";
import { formatBrokerSymbol } from "./symbol.ts";
import { hasInvalidOptionalImportNumber, isImportSummary, isValidImportDate, parseImportNumber as parseCsvNumber, recordImportIssue } from "../parseDiagnostics.ts";

function parseDate(raw: string): string {
  if (!isValidImportDate(raw)) return "";
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

export function parseMoomooTransactions(text: string, defaultMarket: Market, issues?: ImportParseIssue[]): TransactionImportRow[] {
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

  const externalIndex = headers.findIndex(name => ["成交编号", "成交序号", "交易编号", "Execution ID"].includes(name));
  interface Fill { raw: string; externalId: string | null; shares: number; price: number; amount: number; time: string; commission: number }
  interface Group { direction: string; code: string; name: string; market: Market; fills: Fill[] }
  const rows: TransactionImportRow[] = [];
  let group: Group | null = null;
  let key = 0;
  const finalize = () => {
    if (!group || group.fills.length === 0) return;
    // Execution identifiers describe individual fills. Never discard them by
    // aggregating: a later export may contain an overlapping subset of fills.
    if (group.fills.some(fill => fill.externalId !== null)) {
      for (const fill of group.fills) {
        rows.push({ key: String(key++), raw: fill.raw, external_id: fill.externalId,
          selected: true, transaction_type: group.direction, stock_name: group.name,
          symbol: formatBrokerSymbol(group.code, group.market), traded_at: fill.time,
          price: fill.price, shares: fill.shares, total_amount: fill.amount, commission: fill.commission });
      }
      return;
    }
    const shares = group.fills.reduce((sum, fill) => sum + fill.shares, 0);
    const amount = group.fills.reduce((sum, fill) => sum + fill.amount, 0);
    rows.push({
      key: String(key++), raw: group.fills.map(fill => fill.raw),
      external_id: group.fills.length === 1 ? group.fills[0].externalId : null, selected: true, transaction_type: group.direction, stock_name: group.name,
      symbol: formatBrokerSymbol(group.code, group.market), traded_at: group.fills[0].time,
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
    const code = (fields[codeIndex] ?? "").trim();
    if (isImportSummary(direction) || isImportSummary(code)) { finalize(); group = null; continue; }
    if (main) {
      finalize();
      const marketText = marketIndex === -1 ? "" : fields[marketIndex] ?? "";
      group = {
        direction: direction === "卖出" ? "SELL" : "BUY", code,
        name: (nameIndex === -1 ? "" : fields[nameIndex] ?? "").trim() || code,
        market: marketText ? detectMarket(marketText, defaultMarket) : defaultMarket,
        fills: [],
      };
    }
    const child = direction === "" && [sharesIndex, priceIndex, amountIndex, timeIndex]
      .some(index => index !== -1 && (fields[index] ?? "").trim());
    if (!main && !child) {
      if (direction) { finalize(); group = null; }
      continue;
    }
    const shares = parseCsvNumber(fields[sharesIndex]);
    const price = parseCsvNumber(fields[priceIndex]);
    const time = parseDate(fields[timeIndex] ?? "");
    const errors: string[] = [];
    const validSymbol = group?.market === "US"
      ? /^[A-Z0-9][A-Z0-9.\-/]*$/.test(formatBrokerSymbol(group.code, group.market))
      : !!group?.code && /^[A-Z0-9][A-Z0-9.\-/ ]*$/i.test(group.code);
    if (!validSymbol) errors.push("证券代码缺失或无效，无法确定成交所属订单");
    if (!Number.isFinite(shares) || shares === 0) errors.push("成交数量缺失或无效（不能为 0）");
    if (!Number.isFinite(price) || price <= 0) errors.push("成交价格缺失或无效（需大于 0）");
    // Legacy parser callers may use exports without dates; import diagnostics must still surface them.
    if (!time && (timeIndex !== -1 || issues)) errors.push("成交日期或时间缺失或无效");
    for (const index of [amountIndex, commissionIndex]) {
      if (index !== -1 && hasInvalidOptionalImportNumber(fields[index])) errors.push(`${headers[index]} 金额或费用无效`);
    }
    if (errors.length) { recordImportIssue(issues, i + 1, lines[i], errors); continue; }
    const amount = parseCsvNumber(fields[amountIndex]);
    const commission = parseCsvNumber(fields[commissionIndex]);
    const externalId = (fields[externalIndex] ?? "").trim();
    group!.fills.push({
      raw: lines[i], externalId: /^0*$/.test(externalId) ? null : externalId,
      shares: Math.abs(shares), price: Math.abs(price),
      amount: Math.abs(Number.isNaN(amount) ? price * shares : amount), time,
      commission: Number.isNaN(commission) ? 0 : Math.abs(commission),
    });
  }
  finalize();
  return rows;
}
