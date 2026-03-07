export interface Account {
  id: string;
  name: string;
  market: 'US' | 'CN' | 'HK';
  description: string;
  created_at: string;
  updated_at: string;
}

export interface CreateAccountRequest {
  name: string;
  market: 'US' | 'CN' | 'HK';
  description?: string;
}

export interface UpdateAccountRequest {
  name?: string;
  description?: string;
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

export interface CreateCategoryRequest {
  name: string;
  color: string;
  icon?: string;
  sort_order?: number;
}

export interface UpdateCategoryRequest {
  name?: string;
  color?: string;
  icon?: string;
  sort_order?: number;
}

export interface Holding {
  id: string;
  account_id: string;
  symbol: string;
  name: string;
  market: 'US' | 'CN' | 'HK';
  category_id: string | null;
  shares: number;
  avg_cost: number;
  currency: 'USD' | 'CNY' | 'HKD';
  created_at: string;
  updated_at: string;
}

export interface CreateHoldingRequest {
  account_id: string;
  symbol: string;
  name: string;
  market: 'US' | 'CN' | 'HK';
  category_id?: string;
  shares: number;
  avg_cost: number;
  currency: 'USD' | 'CNY' | 'HKD';
}

export interface UpdateHoldingRequest {
  name?: string;
  category_id?: string;
  shares?: number;
  avg_cost?: number;
}

export interface Transaction {
  id: string;
  holding_id: string | null;
  account_id: string;
  symbol: string;
  name: string;
  market: 'US' | 'CN' | 'HK';
  type: 'BUY' | 'SELL';
  shares: number;
  price: number;
  total_amount: number;
  commission: number;
  currency: 'USD' | 'CNY' | 'HKD';
  traded_at: string;
  notes: string;
  created_at: string;
}

export interface CreateTransactionRequest {
  account_id: string;
  symbol: string;
  name: string;
  market: 'US' | 'CN' | 'HK';
  type: 'BUY' | 'SELL';
  shares: number;
  price: number;
  commission?: number;
  currency: 'USD' | 'CNY' | 'HKD';
  traded_at: string;
  notes?: string;
}

export const MARKET_LABELS: Record<string, string> = {
  US: '🇺🇸 美股',
  CN: '🇨🇳 A股',
  HK: '🇭🇰 港股',
};

export const CURRENCY_MAP: Record<string, string> = {
  US: 'USD',
  CN: 'CNY',
  HK: 'HKD',
};
