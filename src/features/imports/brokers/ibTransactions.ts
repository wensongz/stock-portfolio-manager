import dayjs from "dayjs";
import type { Market } from "../../../types";
import { splitCsvLine, stripBom } from "../csv.ts";
import type { ImportParseIssue, TransactionImportRow } from "../types.ts";
import { getCurrencySymbol } from "../../../lib/formatMoney.ts";
import { formatBrokerSymbol } from "./symbol.ts";
import { hasInvalidOptionalImportNumber, isImportSummary, isValidImportDate, parseImportNumber as parseCsvNumber, recordImportIssue } from "../parseDiagnostics.ts";

function validAccountId(value: string): boolean {
  return /^[A-Z]{1,3}\d+$/.test(value.trim());
}

function parseDate(raw: string): string {
  const cleaned = raw.trim();
  if (!isValidImportDate(cleaned)) return "";
  const match = cleaned.match(/^(\d{4}-\d{2}-\d{2}),?\s*(\d{2}:\d{2}:\d{2})/);
  if (match) return `${match[1]}T${match[2]}`;
  const strict = dayjs(cleaned, ["YYYY/M/DD", "YYYY-M-D", "YYYY-MM-DD"], true);
  if (strict.isValid()) return strict.format("YYYY-MM-DDTHH:mm:ss");
  const fallback = dayjs(cleaned);
  return fallback.isValid() ? fallback.format("YYYY-MM-DDTHH:mm:ss") : "";
}

function parseTradeTable(lines: string[], headerIndex: number, market: Market, structured: boolean, issues?: ImportParseIssue[]): TransactionImportRow[] {
  const headers = splitCsvLine(lines[headerIndex]).map((field) => field.trim());
  const column = (name: string) => headers.indexOf(name);
  const externalIndex = headers.findIndex(name => ["Trade ID", "TradeID", "Transaction ID", "Execution ID"].includes(name));
  const symbolIndex = column("Symbol");
  const dateIndex = column("Trade Date/Time") !== -1 ? column("Trade Date/Time") : column("Date/Time");
  const quantityIndex = column("Quantity");
  const priceIndex = column("Price") !== -1 ? column("Price") : column("T. Price");
  const proceedsIndex = column("Proceeds");
  const typeIndex = column("Type");
  const categoryIndex = column("Asset Category");
  const discriminatorIndex = column("DataDiscriminator");
  const accountIndex = column("Acct ID");
  const commissionIndex = column("Comm");
  const feeIndex = column("Fee");
  const combinedFeeIndex = column("Comm/Fee") !== -1 ? column("Comm/Fee") : column("Comm in USD");
  if ([symbolIndex, dateIndex, quantityIndex, priceIndex].includes(-1)) return [];

  const rows: TransactionImportRow[] = [];
  for (let i = headerIndex + 1; i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]);
    if (structured && fields[0]?.trim() === "Trades" && fields[1]?.trim() === "Header") break;
    if (structured && (fields[0]?.trim() !== "Trades" || fields[1]?.trim() !== "Data")) continue;
    if (categoryIndex !== -1 && !/^(Stocks?|ETFs?|股票)$/i.test((fields[categoryIndex] ?? "").trim())) continue;
    if (discriminatorIndex !== -1 && isImportSummary(fields[discriminatorIndex] ?? "")) continue;
    const recognizedAccount = accountIndex !== -1 && validAccountId(fields[accountIndex] ?? "");
    if (!structured && fields.length < 3 && !recognizedAccount) continue;
    const rawSymbol = (fields[symbolIndex] ?? "").trim();
    if (rawSymbol === "Symbol" || isImportSummary(rawSymbol) || isImportSummary(fields[0] ?? "")) continue;
    // IB emits currency heading rows with no security/trade fields.
    if (!fields.slice(symbolIndex).some(field => field.trim()) && !recognizedAccount) continue;
    const quantity = parseCsvNumber(fields[quantityIndex]);
    const price = parseCsvNumber(fields[priceIndex]);
    const tradedAt = parseDate(fields[dateIndex] ?? "");
    const symbol = formatBrokerSymbol(rawSymbol, market);
    const errors: string[] = [];
    const validSymbol = market === "US"
      ? /^[A-Z0-9][A-Z0-9.\-/]*$/.test(symbol)
      : /^[A-Z0-9][A-Z0-9.\-/ ]*$/i.test(rawSymbol);
    if (!validSymbol) errors.push("证券代码缺失或无效");
    if (accountIndex !== -1 && !validAccountId(fields[accountIndex] ?? "")) errors.push("账户编号缺失或无效");
    if (!Number.isFinite(quantity) || quantity === 0) errors.push("成交数量缺失或无效（不能为 0）");
    if (!Number.isFinite(price) || price <= 0) errors.push("成交价格缺失或无效（需大于 0）");
    if (!tradedAt) errors.push("成交日期或时间缺失或无效");
    if (typeIndex !== -1 && !/^(BUY|SELL)$/i.test((fields[typeIndex] ?? "").trim())) errors.push("买卖方向缺失或无效");
    for (const index of [proceedsIndex, commissionIndex, feeIndex, combinedFeeIndex]) {
      if (index !== -1 && hasInvalidOptionalImportNumber(fields[index])) errors.push(`${headers[index]} 金额或费用无效`);
    }
    if (errors.length) { recordImportIssue(issues, i + 1, lines[i], errors); continue; }
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
    const externalId = (fields[externalIndex] ?? "").trim();
    rows.push({
      key: String(i), raw: lines[i], external_id: /^0*$/.test(externalId) ? null : externalId, selected: true, transaction_type: action, stock_name: rawSymbol,
      symbol, traded_at: tradedAt,
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
  const displayCurrency = ["RMB", "人民币"].includes(currency) ? "CNY"
    : ["港元", "港币"].includes(currency) ? "HKD" : currency;
  let notes = `每股分红 ${getCurrencySymbol(displayCurrency)}${amount.toFixed(8).replace(/\.?0+$/, "")}`;
  if (/(bonus\s+dividend|奖励分红|奖励股息|红股|送股)/i.test(description)) notes += "（奖励分红）";
  return notes;
}

function parseDividends(lines: string[], headerIndex: number, market: Market, issues?: ImportParseIssue[]): TransactionImportRow[] {
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
    if (!description || isImportSummary(description)) continue;
    if (!/(dividend|股息|股利|分红|interest|利息)/i.test(description)) continue;
    const match = description.match(market === "US"
      ? /^([A-Z][A-Z0-9]*(?:[._-]|\s+)[A-Z]|[0-9A-Z.\-]+)\s*\(/i
      : /^([0-9A-Z.\-]+)\s*\(/);
    if (!match && /(interest|利息)/i.test(description)) continue;
    const symbol = match ? formatBrokerSymbol(match[1], market) : "";
    const amount = parseCsvNumber(fields.slice(amountIndex).join(","));
    const tradedAt = parseDate(date);
    const errors: string[] = [];
    if (!symbol) errors.push("分红证券代码缺失或无效");
    if (!Number.isFinite(amount)) errors.push("分红金额缺失或无效");
    if (!tradedAt) errors.push("分红日期或时间缺失或无效");
    if (errors.length) { recordImportIssue(issues, i + 1, lines[i], errors); continue; }
    rows.push({
      key: String(i), raw: lines[i], selected: true, transaction_type: "PAY", stock_name: symbol, symbol,
      traded_at: tradedAt, price: 0, shares: 0, total_amount: amount, commission: 0,
      notes: dividendNotes(description),
    });
  }
  return rows;
}

export function parseIbTransactions(text: string, market: Market, issues?: ImportParseIssue[]): TransactionImportRow[] {
  const lines = stripBom(text).split(/\r?\n/);
  const rows: TransactionImportRow[] = [];
  let structured = false;
  for (let i = 0; i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]);
    if (fields[0]?.trim() === "Trades" && fields[1]?.trim() === "Header") {
      structured = true;
      rows.push(...parseTradeTable(lines, i, market, true, issues));
    }
  }
  if (structured) return rows;
  for (let i = 0; i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]).map((field) => field.trim());
    if (fields.includes("Symbol") && fields.includes("Quantity") && (fields.includes("Price") || fields.includes("T. Price"))) {
      return parseTradeTable(lines, i, market, false, issues);
    }
  }
  for (let i = 0; i < lines.length; i++) {
    const fields = splitCsvLine(lines[i]).map((field) => field.trim().toLowerCase());
    if (fields.includes("description") && fields.includes("date") && fields.includes("amount")) {
      return parseDividends(lines, i, market, issues);
    }
  }
  return [];
}
