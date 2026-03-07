export type Market = "US" | "CN" | "HK";
export type Currency = "USD" | "CNY" | "HKD";
export type TransactionType = "BUY" | "SELL";

export interface Account {
  id: string;
  name: string;
  market: Market;
  description: string | null;
  created_at: string;
  updated_at: string;
}

export interface Category {
  id: string;
  name: string;
  color: string;
  icon: string;
  is_system: boolean;
  sort_order: number;
  created_at: string;
}

export interface Holding {
  id: string;
  account_id: string;
  symbol: string;
  name: string;
  market: Market;
  category_id: string | null;
  shares: number;
  avg_cost: number;
  currency: Currency;
  created_at: string;
  updated_at: string;
}

export interface Transaction {
  id: string;
  holding_id: string | null;
  account_id: string;
  symbol: string;
  name: string;
  market: Market;
  transaction_type: TransactionType;
  shares: number;
  price: number;
  total_amount: number;
  commission: number;
  currency: Currency;
  traded_at: string;
  notes: string | null;
  created_at: string;
}

export interface StockQuote {
  symbol: string;
  name: string;
  market: Market;
  current_price: number;
  previous_close: number;
  change: number;
  change_percent: number;
  high: number;
  low: number;
  volume: number;
  updated_at: string;
}

export interface HoldingWithQuote extends Holding {
  quote: StockQuote | null;
  market_value: number | null;
  total_cost: number | null;
  unrealized_pnl: number | null;
  unrealized_pnl_percent: number | null;
}

export interface ExchangeRates {
  usd_cny: number;
  usd_hkd: number;
  cny_hkd: number;
  updated_at: string;
}

export interface DailyPortfolioValue {
  id: number;
  date: string;
  total_cost: number;
  total_value: number;
  us_cost: number;
  us_value: number;
  cn_cost: number;
  cn_value: number;
  hk_cost: number;
  hk_value: number;
  exchange_rates: string;
  daily_pnl: number;
  cumulative_pnl: number;
}

export interface CreateAccountPayload {
  name: string;
  market: Market;
  description?: string;
}

export interface UpdateAccountPayload {
  id: string;
  name: string;
  market: Market;
  description?: string;
}

export interface CreateCategoryPayload {
  name: string;
  color: string;
  icon: string;
  sort_order?: number;
}

export interface UpdateCategoryPayload {
  id: string;
  name: string;
  color: string;
  icon: string;
  sort_order?: number;
}

export interface CreateHoldingPayload {
  account_id: string;
  symbol: string;
  name: string;
  market: Market;
  category_id?: string;
  shares: number;
  avg_cost: number;
  currency: Currency;
}

export interface UpdateHoldingPayload {
  id: string;
  account_id: string;
  symbol: string;
  name: string;
  market: Market;
  category_id?: string;
  shares: number;
  avg_cost: number;
  currency: Currency;
}

export interface CreateTransactionPayload {
  account_id: string;
  symbol: string;
  name: string;
  market: Market;
  transaction_type: TransactionType;
  shares: number;
  price: number;
  total_amount: number;
  commission: number;
  currency: Currency;
  traded_at: string;
  notes?: string;
}

