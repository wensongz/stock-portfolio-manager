import dayjs from "dayjs";
import type { Market } from "../../../types";
import { parseCsvNumber, splitCsvLine } from "../csv.ts";
import type { TransactionImportRow } from "../types.ts";

function validAccountId(value: string): boolean {
  return /^[A-Z]{1,3}\d+$/.test(value.trim());
}

function formatSymbol(symbol: string, market: Market): string {
  if (market === "HK") {
    const digits = symbol.replace(/\D/g, "");
    if (digits) return `${Number.parseInt(digits, 10)}.HK`;
  }
  return symbol.toUpperCase();
}

function parseDate(raw: string): string {
  const cleaned = raw.trim();
  const match = cleaned.match(/^(\d{4}-\d{2}-\d{2}),?\s*(\d{2}:\d{2}:\d{2})/);
  if (match) return `${match[1]}T${match[2]}`;
  const strict = dayjs(cleaned, ["YYYY/M/DD", "YYYY-M-D", "YYYY-MM-DD"], true);
  if (strict.isValid()) return strict.format("YYYY-MM-DDTHH:mm:ss");
  const fallback = dayjs(cleaned);
  return fallback.isValid() ? fallback.format("YYYY-MM-DDTHH:mm:ss") : "";
}

function parseTradeTable(lines: string[], headerIndex: number, market: Market, structured: boolean): TransactionImportRow[] {
  const headers = splitCsvLine(lines[headerIndex]).map((field) => field.trim());
  const column = (name: string) => headers.indexOf(name);
  const symbolIndex = column("Symbol");
  const dateIndex = column("Trade Date/Time") !== -1 ? column("Trade Date/Time") : column("Date/Time");
  const quantityIndex = column("Quantity");
  const priceIndex = column("Price") !== -1 ? column("Price") : column("T. Price");
  const proceedsIndex = column("Proceeds");
  const typeIndex = column("Type");
  const accountIndex = column("Acct ID");
  const commissionIndex = column("Comm");
  const feeIndex = column("Fee");
  const combinedFeeIndex = column("Comm/Fee") !== -1 ? column("Comm/Fee") : column("Comm in USD");
  if ([symbolIndex, dateIndex, quantityIndex, priceIndex].includes(-1)) return [];

  const rows: TransactionImportRow[] = [];
  for (let i = headerIndex + 1; i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]);
    if (structured && (fields[0]?.trim() !== "Trades" || fields[1]?.trim() !== "Data")) continue;
    if (!structured && fields.length < 3) continue;
    const rawSymbol = (fields[symbolIndex] ?? "").trim();
    if (!rawSymbol || rawSymbol.startsWith("Total") || rawSymbol === "Symbol") continue;
    if (accountIndex !== -1 && !validAccountId(fields[accountIndex] ?? "")) continue;
    const quantity = parseCsvNumber(fields[quantityIndex]);
    const price = parseCsvNumber(fields[priceIndex]);
    if (Number.isNaN(quantity) || Number.isNaN(price)) continue;
    const action = typeIndex === -1
      ? (quantity >= 0 ? "BUY" : "SELL")
      : ((fields[typeIndex] ?? "").trim().toUpperCase() === "SELL" ? "SELL" : "BUY");
    const shares = Math.abs(quantity);
    const proceeds = parseCsvNumber(fields[proceedsIndex]);
    let commission = 0;
    if (commissionIndex !== -1 || feeIndex !== -1) {
      const commissionValue = parseCsvNumber(fields[commissionIndex]);
      const feeValue = parseCsvNumber(fields[feeIndex]);
      commission = (Number.isNaN(commissionValue) ? 0 : Math.abs(commissionValue))
        + (Number.isNaN(feeValue) ? 0 : Math.abs(feeValue));
    } else if (combinedFeeIndex !== -1) {
      const combined = parseCsvNumber(fields[combinedFeeIndex]);
      commission = Number.isNaN(combined) ? 0 : Math.abs(combined);
    }
    rows.push({
      key: String(i), selected: true, transaction_type: action, stock_name: rawSymbol,
      symbol: formatSymbol(rawSymbol, market), traded_at: parseDate(fields[dateIndex] ?? ""),
      price: Math.abs(price), shares,
      total_amount: Math.abs(Number.isNaN(proceeds) ? price * shares : proceeds), commission,
    });
  }
  return rows;
}

function dividendNotes(description: string): string {
  const currencyMatch = description.match(/HKD|CNY|USD|RMB|人民币|港元|港币/i);
  const currency = currencyMatch?.[0].toUpperCase() ?? "HKD";
  const escapedCurrency = currencyMatch?.[0].replace(/[.*+?^${}()|[\]\\]/g, "\\$&") ?? "HKD";
  const amountMatch = description.match(new RegExp(`${escapedCurrency}\\s+([0-9]+(?:\\.[0-9]+)?)`));
  if (!amountMatch) return "";
  const amount = Number.parseFloat(amountMatch[1]);
  if (Number.isNaN(amount)) return "";
  let notes = `每股分红 ${currency} ${amount.toFixed(8).replace(/\.?0+$/, "")}`;
  if (/(bonus\s+dividend|奖励分红|奖励股息|红股|送股)/i.test(description)) notes += "（奖励分红）";
  return notes;
}

function parseDividends(lines: string[], headerIndex: number, market: Market): TransactionImportRow[] {
  const headers = splitCsvLine(lines[headerIndex]).map((field) => field.trim().toLowerCase());
  const dateIndex = headers.indexOf("date");
  const descriptionIndex = headers.indexOf("description");
  const amountIndex = headers.indexOf("amount");
  if ([dateIndex, descriptionIndex, amountIndex].includes(-1)) return [];
  const rows: TransactionImportRow[] = [];
  for (let i = headerIndex + 1; i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]);
    const date = (fields[dateIndex] ?? "").trim();
    const description = (fields[descriptionIndex] ?? "").trim();
    if (!date || !description || description.toLowerCase().startsWith("total")) continue;
    if (!/(dividend|股息|股利|分红|interest|利息)/i.test(description)) continue;
    const match = description.match(/^([0-9A-Z.\-]+)\s*\(/);
    if (!match) continue;
    const symbol = formatSymbol(match[1], market);
    const amount = parseCsvNumber(fields.slice(amountIndex).join(","));
    const tradedAt = parseDate(date);
    if (!symbol || Number.isNaN(amount) || !tradedAt) continue;
    rows.push({
      key: String(i), selected: true, transaction_type: "PAY", stock_name: symbol, symbol,
      traded_at: tradedAt, price: 0, shares: 0, total_amount: amount, commission: 0,
      notes: dividendNotes(description),
    });
  }
  return rows;
}

export function parseIbTransactions(text: string, market: Market): TransactionImportRow[] {
  const lines = text.split(/\r?\n/);
  for (let i = 0; i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]);
    if (fields[0]?.trim() === "Trades" && fields[1]?.trim() === "Header") {
      const rows = parseTradeTable(lines, i, market, true);
      if (rows.length) return rows;
    }
  }
  for (let i = 0; i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]).map((field) => field.trim());
    if (fields.includes("Symbol")) {
      const rows = parseTradeTable(lines, i, market, false);
      if (rows.length) return rows;
    }
  }
  for (let i = 0; i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]).map((field) => field.trim().toLowerCase());
    if (fields.includes("description")) {
      const rows = parseDividends(lines, i, market);
      if (rows.length) return rows;
    }
  }
  return [];
}
