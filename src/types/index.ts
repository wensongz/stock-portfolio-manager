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
