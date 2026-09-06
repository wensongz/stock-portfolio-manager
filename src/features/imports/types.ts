import type { Currency, Market } from "../../types";

export interface ImportRow {
  key: string;
  raw?: unknown;
  external_id?: string | null;
  selected: boolean;
  lookingUp?: boolean;
  importOk?: boolean;
  importError?: string;
}

export interface TransactionImportRow extends ImportRow {
  transaction_type: string;
  stock_name: string;
  symbol: string;
  traded_at: string;
  price: number;
  shares: number;
  total_amount: number;
  commission: number;
  notes?: string;
}

export interface HoldingImportRow extends ImportRow {
  symbol: string;
  name: string;
  shares: number;
  avgCost: number;
  currency?: Currency;
  market?: Market;
  isCash?: boolean;
}

export interface ParseResult<Row extends ImportRow> {
  rows: Row[];
  warnings: string[];
  sourceContent?: string;
}

export interface ImportResult {
  success: number;
  failed: number;
  errors: { name: string; error: string }[];
}

