import dayjs from "dayjs";
import { parseCsvNumber, splitCsvLine, stripBom } from "../csv.ts";
import type { TransactionImportRow } from "../types.ts";

function parseDate(raw: string): string {
  const cleaned = raw.trim();
  const match = cleaned.match(/^(\d{4})\/(\d{1,2})\/(\d{1,2})$/);
  if (match) {
    const parsed = dayjs(`${match[1]}-${match[2].padStart(2, "0")}-${match[3].padStart(2, "0")}`);
    if (parsed.isValid()) return parsed.hour(10).minute(30).second(0).format("YYYY-MM-DDTHH:mm:ss");
  }
  const parsed = dayjs(cleaned);
  return parsed.isValid() ? parsed.hour(10).minute(30).second(0).format("YYYY-MM-DDTHH:mm:ss") : "";
}

export function parseFirstradeTransactions(text: string): TransactionImportRow[] {
  const lines = stripBom(text).split(/\r?\n/);
  const headerIndex = lines.findIndex((line) => {
    const fields = splitCsvLine(line).map((field) => field.trim());
    return fields.includes("Symbol") && fields.includes("Action");
  });
  if (headerIndex === -1) return [];

  const headers = splitCsvLine(lines[headerIndex]).map((field) => field.trim());
  const column = (name: string) => headers.indexOf(name);
  const externalIndex = headers.findIndex(name => ["Trade ID", "TradeID", "Transaction ID", "Execution ID"].includes(name));
  const symbolIndex = column("Symbol");
  const quantityIndex = column("Quantity");
  const priceIndex = column("Price");
  const actionIndex = column("Action");
  const dateIndex = column("TradeDate");
  const amountIndex = column("Amount");
  const commissionIndex = column("Commission");
  const feeIndex = column("Fee");
  if ([symbolIndex, quantityIndex, priceIndex, actionIndex].includes(-1)) return [];

  const rows: TransactionImportRow[] = [];
  for (let i = headerIndex + 1; i < lines.length; i++) {
    if (!lines[i].trim()) continue;
    const fields = splitCsvLine(lines[i]);
    const action = (fields[actionIndex] ?? "").trim().toUpperCase();
    if (action !== "BUY" && action !== "SELL") continue;
    const symbol = (fields[symbolIndex] ?? "").trim().toUpperCase();
    const quantity = parseCsvNumber(fields[quantityIndex]);
    const price = parseCsvNumber(fields[priceIndex]);
    if (!symbol || Number.isNaN(quantity) || Number.isNaN(price) || price <= 0 || quantity === 0) continue;
    const shares = Math.abs(quantity);
    const amount = parseCsvNumber(fields[amountIndex]);
    const commission = parseCsvNumber(fields[commissionIndex]);
    const fee = parseCsvNumber(fields[feeIndex]);
    const externalId = (fields[externalIndex] ?? "").trim();
    rows.push({
      key: String(i), raw: lines[i], external_id: /^0*$/.test(externalId) ? null : externalId, selected: true, transaction_type: action, stock_name: symbol, symbol,
      traded_at: parseDate(fields[dateIndex] ?? ""), price: Math.abs(price), shares,
      total_amount: Math.abs(Number.isNaN(amount) ? price * shares : amount),
      commission: (Number.isNaN(commission) ? 0 : Math.abs(commission)) + (Number.isNaN(fee) ? 0 : Math.abs(fee)),
    });
  }
  return rows;
}
