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

// Phase 3: Dashboard types
export interface DashboardSummary {
  total_market_value: number;
  total_cost: number;
  total_pnl: number;
  total_pnl_percent: number;
  daily_pnl: number;
  us_market_value: number;
  cn_market_value: number;
  hk_market_value: number;
  exchange_rates: ExchangeRates;
  base_currency: string;
}

export interface HoldingDetail {
  id: string;
  account_id: string;
  account_name: string;
  symbol: string;
  name: string;
  market: string;
  category_name: string;
  category_color: string;
  shares: number;
  avg_cost: number;
  current_price: number;
  market_value: number;
  cost_value: number;
  pnl: number;
  pnl_percent: number;
  currency: Currency;
}

// Phase 3: Statistics types
export interface PieSlice {
  name: string;
  value: number;
  color?: string | null;
}

export interface PnlItem {
  symbol: string;
  name: string;
  pnl: number;
  pnl_percent: number;
  market_value: number;
}

export interface StatisticsOverview {
  total_market_value: number;
  total_cost: number;
  total_pnl: number;
  total_pnl_percent: number;
  market_distribution: PieSlice[];
  category_distribution: PieSlice[];
  account_distribution: PieSlice[];
  top_gainers: PnlItem[];
  top_losers: PnlItem[];
}

export interface MarketStatistics {
  market: string;
  total_market_value: number;
  total_cost: number;
  total_pnl: number;
  total_pnl_percent: number;
  account_distribution: PieSlice[];
  category_distribution: PieSlice[];
  stock_distribution: PieSlice[];
  holdings: HoldingDetail[];
}

export interface AccountStatistics {
  account_id: string;
  account_name: string;
  market: string;
  total_market_value: number;
  total_cost: number;
  total_pnl: number;
  total_pnl_percent: number;
  category_distribution: PieSlice[];
  stock_distribution: PieSlice[];
  holdings: HoldingDetail[];
}

export interface CategoryStatistics {
  category_id: string;
  category_name: string;
  category_color: string;
  total_market_value: number;
  total_cost: number;
  total_pnl: number;
  total_pnl_percent: number;
  market_distribution: PieSlice[];
  holdings: HoldingDetail[];
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

// Phase 4: Performance types
export interface PerformanceSummary {
  start_date: string;
  end_date: string;
  start_value: number;
  end_value: number;
  total_return: number;
  annualized_return: number;
  total_pnl: number;
  max_drawdown: number;
  volatility: number;
  sharpe_ratio: number;
}

export interface ReturnDataPoint {
  date: string;
  cumulative_return: number;
  daily_return: number;
  portfolio_value: number;
  daily_pnl: number;
}

export interface DrawdownPoint {
  date: string;
  drawdown: number;
}

export interface DrawdownAnalysis {
  max_drawdown: number;
  peak_date: string;
  trough_date: string;
  recovery_date: string | null;
  drawdown_duration: number;
  recovery_duration: number | null;
  drawdown_series: DrawdownPoint[];
}

export interface AttributionItem {
  name: string;
  pnl: number;
  contribution_percent: number;
  weight: number;
}

export interface ReturnAttribution {
  total_pnl: number;
  by_market: AttributionItem[];
  by_category: AttributionItem[];
  by_holding: AttributionItem[];
}

export interface MonthlyReturn {
  year: number;
  month: number;
  return_rate: number;
  pnl: number;
  start_value: number;
  end_value: number;
}

export interface HoldingPerformance {
  symbol: string;
  name: string;
  market: string;
  category_name: string;
  return_rate: number;
  pnl: number;
  start_value: number;
  end_value: number;
}

export interface RiskMetrics {
  daily_volatility: number;
  annualized_volatility: number;
  sharpe_ratio: number;
  risk_free_rate: number;
  max_drawdown: number;
  calmar_ratio: number;
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

// Phase 5: Quarterly Analysis types
export interface QuarterlySnapshot {
  id: string;
  quarter: string;
  snapshot_date: string;
  total_value: number;
  total_cost: number;
  total_pnl: number;
  us_value: number;
  us_cost: number;
  cn_value: number;
  cn_cost: number;
  hk_value: number;
  hk_cost: number;
  exchange_rates: string;
  overall_notes: string | null;
  created_at: string;
  holding_count: number;
}

export interface QuarterlyHoldingSnapshot {
  id: string;
  quarterly_snapshot_id: string;
  account_id: string;
  account_name: string;
  symbol: string;
  name: string;
  market: string;
  category_name: string;
  category_color: string;
  shares: number;
  avg_cost: number;
  close_price: number;
  market_value: number;
  cost_value: number;
  pnl: number;
  pnl_percent: number;
  weight: number;
  notes: string | null;
}

export interface QuarterlySnapshotDetail {
  snapshot: QuarterlySnapshot;
  holdings: QuarterlyHoldingSnapshot[];
}

export interface ComparisonOverview {
  q1_total_value: number;
  q2_total_value: number;
  value_change: number;
  value_change_percent: number;
  q1_total_cost: number;
  q2_total_cost: number;
  q1_pnl: number;
  q2_pnl: number;
  q1_holding_count: number;
  q2_holding_count: number;
}

export interface MarketComparison {
  market: string;
  q1_value: number;
  q2_value: number;
  value_change: number;
  value_change_percent: number;
  q1_cost: number;
  q2_cost: number;
  q1_pnl: number;
  q2_pnl: number;
}

export interface CategoryComparison {
  category_name: string;
  category_color: string;
  q1_value: number;
  q2_value: number;
  value_change: number;
  value_change_percent: number;
  q1_cost: number;
  q2_cost: number;
  q1_pnl: number;
  q2_pnl: number;
}

export interface HoldingChangeItem {
  symbol: string;
  name: string;
  market: string;
  category_name: string;
  q1_shares: number | null;
  q2_shares: number | null;
  q1_value: number | null;
  q2_value: number | null;
  shares_change: number;
  value_change: number;
}

export interface HoldingChanges {
  new_holdings: HoldingChangeItem[];
  closed_holdings: HoldingChangeItem[];
  increased: HoldingChangeItem[];
  decreased: HoldingChangeItem[];
  unchanged: HoldingChangeItem[];
}

export interface QuarterComparison {
  quarter1: string;
  quarter2: string;
  overview: ComparisonOverview;
  by_market: MarketComparison[];
  by_category: CategoryComparison[];
  holding_changes: HoldingChanges;
}

export interface HoldingNoteHistory {
  quarter: string;
  snapshot_date: string;
  shares: number;
  avg_cost: number;
  close_price: number;
  pnl_percent: number;
  notes: string;
}

export interface QuarterlyNotesSummary {
  snapshot_id: string;
  quarter: string;
  snapshot_date: string;
  overall_notes: string;
  total_value: number;
  total_pnl: number;
}

export interface QuarterlyTrends {
  quarters: string[];
  total_values: number[];
  total_costs: number[];
  total_pnls: number[];
  market_values: Record<string, number[]>;
  category_values: Record<string, number[]>;
  holding_counts: number[];
}

// Phase 6: Import/Export types
export interface ExportFilters {
  market?: string;
  account_id?: string;
  category_id?: string;
}

export interface ImportError {
  row: number;
  column: string;
  message: string;
}

export interface ImportPreview {
  total_rows: number;
  valid_rows: number;
  error_rows: ImportError[];
  preview_data: Record<string, unknown>[];
  column_mapping: Record<string, string>;
}

export interface ImportData {
  data_type: string;
  rows: Record<string, unknown>[];
  column_mapping: Record<string, string>;
  account_id: string;
}

export interface ImportResult {
  imported_count: number;
  skipped_count: number;
  errors: ImportError[];
}

// Phase 6: Price Alerts
export type AlertType =
  | "PRICE_ABOVE"
  | "PRICE_BELOW"
  | "CHANGE_ABOVE"
  | "CHANGE_BELOW"
  | "PNL_ABOVE"
  | "PNL_BELOW";

export interface PriceAlert {
  id: string;
  holding_id: string | null;
  symbol: string;
  name: string;
  market: Market;
  alert_type: AlertType;
  threshold: number;
  is_active: boolean;
  is_triggered: boolean;
  triggered_at: string | null;
  created_at: string;
}

export interface TriggeredAlert {
  alert: PriceAlert;
  current_value: number;
  message: string;
}

// Phase 6: Review types
export interface QuarterlyHoldingStatus {
  snapshot_id: string;
  quarter: string;
  shares: number;
  avg_cost: number;
  close_price: number;
  pnl_percent: number;
  notes: string | null;
  decision_quality: "correct" | "wrong" | "pending" | null;
}

export interface HoldingReview {
  symbol: string;
  name: string;
  market: Market;
  is_current_holding: boolean;
  quarterly_timeline: QuarterlyHoldingStatus[];
}

export interface DecisionStatistics {
  total_decisions: number;
  correct_count: number;
  wrong_count: number;
  pending_count: number;
  accuracy_rate: number;
}

// Phase 6: AI Config
export interface AiConfig {
  provider: string;
  api_key: string;
  model: string;
  base_url: string | null;
  system_prompt: string;
}

