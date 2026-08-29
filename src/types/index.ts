export type Market = "US" | "CN" | "HK";
export type Currency = "USD" | "CNY" | "HKD";
export type TransactionType = "BUY" | "SELL" | "OPEN" | "PAY";

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
  // Fundamental / valuation snapshot (optional, may be absent).
  pe_ttm?: number;
  pb?: number;
  market_cap?: number;
  dividend_yield?: number;
  eps?: number;
  roe?: number;
  turnover_rate?: number;
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
  pnl_percent: number | null;
  daily_pnl: number;
  currency: Currency;
  /** Market value normalised to USD for cross-currency sorting. */
  market_value_usd: number;
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
  pnl_percent: number | null;
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
  stock_distribution: PieSlice[];
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
  sortOrder?: number;
}

export interface UpdateCategoryPayload {
  id: string;
  name: string;
  color: string;
  icon: string;
  sortOrder?: number;
}

export interface CreateHoldingPayload {
  accountId: string;
  symbol: string;
  name: string;
  market: Market;
  categoryId?: string;
  shares: number;
  avgCost: number;
  currency: Currency;
}

export interface UpdateHoldingPayload {
  id: string;
  accountId: string;
  symbol: string;
  name: string;
  market: Market;
  categoryId?: string;
  shares: number;
  avgCost: number;
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
  sharpe_ratio: number | null;
  /** Daily return series computed from the same DB query as the summary. */
  return_series: ReturnDataPoint[];
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
  sharpe_ratio: number | null;
  risk_free_rate: number;
  max_drawdown: number;
  calmar_ratio: number | null;
}

export interface CreateTransactionPayload {
  accountId: string;
  symbol: string;
  name: string;
  market: Market;
  transactionType: TransactionType;
  shares: number;
  price: number;
  totalAmount: number;
  commission: number;
  currency: Currency;
  tradedAt: string;
  notes?: string;
}

export interface UpdateTransactionPayload {
  id: string;
  accountId: string;
  symbol: string;
  name: string;
  market: Market;
  transactionType: TransactionType;
  shares: number;
  price: number;
  totalAmount: number;
  commission: number;
  currency: Currency;
  tradedAt: string;
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
  pnl_percent: number | null;
  weight: number;
  notes: string | null;
}

export interface QuarterlySnapshotDetail {
  snapshot: QuarterlySnapshot;
  holdings: QuarterlyHoldingSnapshot[];
  holding_changes: HoldingChanges | null;
  previous_quarter: string | null;
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
  pnl_percent: number | null;
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

/** Per-stock summary of transactions within a quarter. */
export interface StockTransactionGroup {
  symbol: string;
  name: string;
  market: Market;
  currency: Currency;
  buy_count: number;
  sell_count: number;
  total_buy_shares: number;
  total_sell_shares: number;
  total_buy_amount: number;
  total_sell_amount: number;
  transactions: Transaction[];
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

export interface ImportSkipped {
  row: number;
  symbol: string;
  reason: string;
}

export interface ImportResult {
  imported_count: number;
  skipped_count: number;
  skipped_rows: ImportSkipped[];
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

export interface OptionReviewReport {
  account_id: string;
  currency: Currency;
  period_days: number | null;
  generated_at: string;
  summary: OptionReviewSummary;
  underlyings: OptionUnderlyingReview[];
  data_quality: OptionReviewDataQuality;
}

export interface OptionReviewSummary {
  completed_campaigns: number;
  active_campaigns: number;
  /** Opening premium across completed and active Campaigns. */
  gross_premium: number;
  /** Cash net across completed and active Campaigns. */
  net_premium_pnl: number;
  /** Completed-only denominator used by retention_rate. */
  completed_gross_premium: number;
  /** Completed-only numerator used by retention/yield/worst metrics. */
  completed_net_premium_pnl: number;
  retention_rate: number | null;
  annualized_yield_on_notional: number | null;
  worst_campaign: OptionWorstCampaign | null;
}

export interface OptionWorstCampaign {
  campaign_id: string;
  underlying: string;
  started_at: string;
  ended_at: string;
  strategy_path: string[];
  net_premium_pnl: number;
}

export interface OptionUnderlyingReview {
  underlying: string;
  completed_campaigns: number;
  active_campaigns: number;
  /** Opening premium across completed and active Campaigns. */
  gross_premium: number;
  /** Cash net across completed and active Campaigns. */
  net_premium_pnl: number;
  /** Completed-only denominator used by retention_rate. */
  completed_gross_premium: number;
  /** Completed-only numerator used by retention/yield/worst metrics. */
  completed_net_premium_pnl: number;
  retention_rate: number | null;
  annualized_yield_on_notional: number | null;
  worst_campaign_pnl: number | null;
  flags: string[];
  campaigns: OptionCampaign[];
}

export interface OptionCampaign {
  id: string;
  underlying: string;
  option_symbol: string;
  expiry_date: string;
  contracts: number;
  started_at: string;
  ended_at: string | null;
  status: "completed" | "active";
  inferred: boolean;
  strategy_path: string[];
  gross_premium: number;
  close_cost: number;
  fees: number;
  net_premium_pnl: number | null;
  secured_notional: number;
  capital_days: number;
  retention_rate: number | null;
  annualized_yield_on_notional: number | null;
}

export interface OptionReviewDataQuality {
  excluded_open_campaigns: number;
  unmatched_records: number;
  missing_trade_dates: number;
  notes: string[];
}

// Phase 6: Review types
export type MetricStatus = "available" | "degraded" | "pending" | "unavailable";
export type StockActionType = "open" | "add" | "reduce" | "close";
export type StockReviewPeriodPreset =
  | "QTD"
  | "PREV_QUARTER"
  | "YTD"
  | "1Y"
  | "CUSTOM";
export type StockCampaignStatus = "active" | "completed";
export type CampaignCashFlowKind = "buy" | "sell" | "dividend" | "fee";
export type StockReviewIssueSeverity = "info" | "warning" | "error";

/** Frontend filter state. Tauri invocation arguments are derived from this. */
export interface StockReviewFilters {
  accountId: string | null;
  periodPreset: StockReviewPeriodPreset;
  startDate: string;
  endDate: string;
  market: Market | null;
  benchmarkSymbol: string | null;
  baseCurrency: Currency;
}

export interface MetricAvailability {
  status: MetricStatus;
  note: string | null;
}

export interface StockReviewQuery {
  start_date: string;
  end_date: string;
  account_id: string | null;
  market: string | null;
  benchmark_symbol: string | null;
  base_currency: string;
}

export interface StockReviewMethodology {
  query: StockReviewQuery;
  actual_return_method: string;
  shadow_return_method: string;
  benchmark_return_method: string;
  fixed_weights: FixedWeight[];
  benchmark_symbol: string | null;
  market_data_coverage: DataCoverage;
  exchange_rate_coverage: DataCoverage;
  algorithm_version: string;
}

export interface FixedWeight {
  key: string;
  weight: number | null;
}

export interface DataCoverage {
  availability: MetricAvailability;
  covered_days: number | null;
  expected_days: number | null;
  coverage_ratio: number | null;
}

export interface StockReviewReport {
  methodology: StockReviewMethodology;
  summary: StockReviewSummary;
  curves: ReviewCurvePoint[];
  attribution: RebalanceAttributionSummary;
  risk_structure: RiskStructureDetail;
  actions: StockActionReview[];
  campaigns: StockCampaignSummary[];
  data_quality: StockReviewDataQuality;
  annotations: StockReviewAnnotation[];
  generated_at: string;
}

export interface StockReviewSummary {
  result_quality: ResultQualityMetric;
  max_drawdown: MaxDrawdownMetric;
  rebalance_value_add: RebalanceValueAddMetric;
  forward_effect: ForwardEffectMetric;
  risk_structure: RiskStructureMetric;
}

export interface ResultQualityMetric {
  availability: MetricAvailability;
  portfolio_return: number | null;
  shadow_return: number | null;
  benchmark_return: number | null;
  excess_return: number | null;
  active_return: number | null;
}

export interface MaxDrawdownMetric {
  availability: MetricAvailability;
  max_drawdown: number | null;
  peak_date: string | null;
  trough_date: string | null;
  duration_days: number | null;
  recovery_date: string | null;
  recovery_duration_days: number | null;
}

export interface RebalanceValueAddMetric {
  availability: MetricAvailability;
  value_add: number | null;
  actual_return: number | null;
  shadow_return: number | null;
  ending_value_difference_base: number | null;
}

export interface ForwardEffectMetric {
  availability: MetricAvailability;
  day_60: ForwardEffectWindow;
  day_120: ForwardEffectWindow;
}

export interface ForwardEffectWindow {
  trading_days: number;
  status: MetricAvailability;
  matured_actions: number;
  pending_actions: number;
  amount_weighted_excess_return: number | null;
  positive_notional_ratio: number | null;
}

export interface RiskStructureMetric {
  availability: MetricAvailability;
  opening_max_stock_weight: number | null;
  ending_max_stock_weight: number | null;
  opening_cr5: number | null;
  ending_cr5: number | null;
  opening_cash_ratio: number | null;
  ending_cash_ratio: number | null;
  one_way_turnover: number | null;
  fee_drag: number | null;
}

export interface ReviewCurvePoint {
  date: string;
  portfolio_return: number | null;
  shadow_return: number | null;
  benchmark_return: number | null;
}

export interface RebalanceAttributionSummary {
  availability: MetricAvailability;
  total_value_add: number | null;
  buy_value_add: number | null;
  sell_value_add: number | null;
  fees: number | null;
  action_contributions: RebalanceAttributionItem[];
  contributors: RebalanceAttributionItem[];
  detractors: RebalanceAttributionItem[];
  dividend_contribution: number | null;
  fee_contribution: number | null;
  currency_contribution: number | null;
  cash_contribution: number | null;
  explained_value_difference: number | null;
  ending_value_difference: number | null;
  residual: number | null;
  residual_to_average_nav: number | null;
  percentage_basis_label: string;
}

export interface RebalanceAttributionItem {
  market: string;
  symbol: string;
  action_type: StockActionType;
  action_id: string;
  amount: number;
  percentage_of_average_nav: number | null;
}

export interface RiskStructureDetail {
  availability: MetricAvailability;
  concentration_availability: MetricAvailability;
  turnover_availability: MetricAvailability;
  fee_availability: MetricAvailability;
  opening: ConcentrationSnapshot;
  ending: ConcentrationSnapshot;
  peak: ConcentrationSnapshot;
  one_way_turnover: number | null;
  fee_drag: number | null;
  data_hints: string[];
  fact_labels: string[];
  market_weights: RiskStructureWeight[];
  category_weights: RiskStructureWeight[];
  top_position_weights: RiskStructureWeight[];
  concentration: number | null;
  diversification_score: number | null;
}

export interface ConcentrationSnapshot {
  date: string | null;
  max_stock_weight: number | null;
  cr5: number | null;
  hhi: number | null;
  cash_ratio: number | null;
}

export interface RiskStructureWeight {
  key: string;
  weight: number | null;
}

export interface StockActionReview {
  action_id: string;
  transaction_ids: string[];
  account_id: string;
  symbol: string;
  market: string;
  action_type: StockActionType;
  traded_at: string;
  weighted_average_price: number | null;
  gross_amount: number | null;
  currency: string | null;
  shares_before: number | null;
  shares_after: number | null;
  portfolio_weight_before: number | null;
  portfolio_weight_after: number | null;
  fees: number | null;
  contribution: number | null;
  observation_windows: ForwardEffectWindow[];
  status: MetricStatus;
  fact_labels: string[];
}

export interface StockCampaignSummary {
  campaign_id: string;
  account_ids: string[];
  action_ids: string[];
  fragments: AccountCampaignFragment[];
  campaign_status: StockCampaignStatus;
  availability: MetricAvailability;
  symbol: string;
  market: string;
  started_at: string;
  ended_at: string | null;
  contribution: number | null;
}

export interface StockCampaignTransferFact {
  transaction_id: string;
  action_id: string | null;
  traded_at: string;
}

export interface AccountCampaignFragment {
  fragment_id: string;
  logical_campaign_id: string;
  account_id: string;
  symbol: string;
  market: string;
  started_at: string;
  ended_at: string | null;
  status: StockCampaignStatus;
  action_ids: string[];
  transfer_in: StockCampaignTransferFact | null;
  transfer_out: StockCampaignTransferFact | null;
}

export interface StockCampaignDetail {
  availability: MetricAvailability;
  pnl_availability: MetricAvailability;
  excursion_availability: MetricAvailability;
  drawdown_availability: MetricAvailability;
  benchmark_availability: MetricAvailability;
  summary: StockCampaignSummary;
  actions: StockActionReview[];
  forward_effect_20d: ForwardEffectWindow;
  forward_effect_60d: ForwardEffectWindow;
  forward_effect_120d: ForwardEffectWindow;
  pnl: CampaignPnl;
  campaign_return: number | null;
  benchmark_return: number | null;
  excess_return: number | null;
  mae_base: number | null;
  mfe_base: number | null;
  mae_percent: number | null;
  mfe_percent: number | null;
  holding_period_drawdown: number | null;
  timeline: CampaignTimelineItem[];
  fact_labels: string[];
  completed_sample_count: number;
  active_sample_count: number;
  annotations: StockReviewAnnotation[];
  issues: StockReviewIssue[];
}

export interface CampaignPnl {
  buy_outlays_base: number | null;
  sell_proceeds_base: number | null;
  dividends_base: number | null;
  trading_fees_base: number | null;
  remaining_shares: number;
  remaining_market_value_base: number | null;
  total_pnl_base: number | null;
  max_invested_capital_base: number | null;
  label: string;
}

export interface CampaignTimelineItem {
  date: string;
  kind: CampaignCashFlowKind;
  amount_base: number | null;
  amount_local: number;
  currency: string;
  shares: number;
  account_id: string;
  action_id: string | null;
}

export interface StockReviewDataQuality {
  availability: MetricAvailability;
  actual_result_availability: MetricAvailability;
  shadow_value_add_availability: MetricAvailability;
  attribution_availability: MetricAvailability;
  forward_effect_availability: MetricAvailability;
  issues: StockReviewIssue[];
  market_data_coverage: number | null;
  exchange_rate_coverage: number | null;
  interval_drawdown_only: boolean;
  market_price_gap_total: number;
  market_price_gap_omitted: number;
}

export interface StockReviewIssue {
  code: string;
  severity: StockReviewIssueSeverity;
  message: string;
  affected_market: string | null;
  affected_symbol: string | null;
  affected_date: string | null;
}

export interface StockReviewAnnotation {
  id: string;
  scope_type: string;
  scope_key: string;
  account_id: string | null;
  symbol: string | null;
  annotation_type: string;
  value_json: string;
  source: string;
  created_at: string;
  updated_at: string;
}

export interface StockReviewAnnotationInput {
  id: string;
  scope_type: string;
  scope_key: string;
  account_id: string | null;
  symbol: string | null;
  annotation_type: string;
  value_json: string;
  source: string;
}

export interface StockReviewOverride {
  id: string;
  override_type: string;
  transaction_ids_json: string;
  value_json: string;
  created_at: string;
  updated_at: string;
}

export interface StockReviewOverrideInput {
  id: string;
  override_type: string;
  transaction_ids_json: string;
  value_json: string;
}

export interface QuarterlyHoldingStatus {
  snapshot_id: string;
  quarter: string;
  shares: number;
  avg_cost: number;
  close_price: number;
  pnl_percent: number | null;
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
  tools_enabled: boolean;
}

export type AiProvider =
  | "openai"
  | "ollama"
  | "openrouter"
  | "kimi"
  | "glm"
  | "mimo"
  | "deepseek"
  | "anthropic";

export interface AiModelInfo {
  id: string;
  name?: string | null;
  owned_by?: string | null;
}

// AI Assistant chat
export interface ChatMessage {
  role: "user" | "assistant" | "system";
  content: string;
}

/** Token-usage accounting for a single chat turn (from the final SSE chunk). */
export interface ChatUsage {
  promptTokens: number;
  completionTokens: number;
  totalTokens: number;
  /** Portion of promptTokens that hit the provider's prompt cache. 0 if N/A. */
  cachedTokens?: number;
}

/**
 * A chat message with client-side metadata for display.
 * - `createdAt`: epoch millis when the message was added to the UI
 * - `usage`: token breakdown (assistant messages only, populated when the
 *   stream's final chunk arrives)
 * - `stopped`: true if the user aborted this assistant turn mid-stream
 * - `error`: present when this assistant turn failed (network error, HTTP 4xx,
 *   etc.). The UI renders the message as an error card with a retry button
 *   instead of a blank bubble. Not persisted — only lives in memory for the
 *   current session view.
 */
export interface ChatMessageWithMeta {
  id: string;
  role: "user" | "assistant" | "system";
  content: string;
  createdAt: number;
  usage?: ChatUsage;
  stopped?: boolean;
  error?: string;
  /**
   * Chain-of-thought text streamed from `reasoning_content` (DeepSeek-R1 /
   * GLM-4.5+ thinking models). In-memory only — not persisted. Rendered as a
   * collapsible "思考过程" block above the answer.
   */
  reasoning?: string;
  /**
   * Names of the skills the backend activated for this turn (explicit via
   * `active_skills`, or auto-matched from triggers). Populated from the
   * `ai-chat-skill` event. Only set on the assistant placeholder that the
   * backend is streaming into; used to render a "⚡ 已用技能" badge.
   */
  activatedSkills?: string[];
  /**
   * Names of the data tools the backend executed for this turn (e.g.
   * `get_market_overview`, `get_stock_quote`). Populated from the
   * `ai-chat-tool` event and accumulated across rounds. Used to render a
   * "🔍 已查询" badge so the user can see the assistant fetched real data.
   * @deprecated Prefer `toolCalls` (richer, per-call detail). Kept for
   * backward compat with older persisted sessions.
   */
  usedTools?: string[];
  /**
   * Detailed per-tool-call progress for this turn (Claude-style expandable
   * cards). Populated from the `ai-chat-tool-call` event and upserted by id
   * across rounds. In-memory only — not persisted. When present, the UI
   * renders `ToolCallCard`s instead of the legacy `usedTools` name badges.
   */
  toolCalls?: ToolCallInfo[];
  /**
   * Skill IDs the user *explicitly* staged for this turn (via `/`, `@`, or a
   * quick chip). Captured onto the assistant placeholder at send time so a
   * retry of a failed turn can re-send the same explicit selection instead
   * of silently dropping it (see chatStore.retryLastTurn).
   */
  explicitSkillIds?: string[];
  /** Exact host-approved read-tool scope captured for retry/regenerate. */
  explicitToolContext?: AiToolContext;
}

export interface AiToolContext {
  name: "get_stock_review";
  arguments: Record<string, string>;
}

/**
 * One tool invocation's lifecycle, mirrored from the backend `ToolCallEvent`.
 * Used to render a Claude-style expandable tool card showing status, arguments,
 * and results as the agentic loop runs.
 */
export interface ToolCallInfo {
  /** Stable renderer id. Model-origin ids are namespaced by the host. */
  id: string;
  /** Set by the Rust host; provider/model payloads cannot choose this value. */
  origin: "host_prefill" | "model";
  /** Function name, e.g. `get_stock_quote`. */
  name: string;
  /** Raw JSON arguments string the model supplied (may be undefined/empty). */
  arguments?: string;
  status: "running" | "success" | "error";
  /** Tool result JSON (success only; truncated for display). */
  result?: string;
  /** Error message (error only). */
  error?: string;
  /** Wall-clock execution time in milliseconds (success/error only). */
  durationMs?: number;
}

/** A persisted AI chat session (one named conversation). */
export interface ChatSession {
  id: string;
  name: string;
  created_at: string;
  updated_at: string;
}

/** A persisted chat message row (with token-usage accounting). */
export interface ChatMessageRecord {
  id: string;
  session_id: string;
  role: "user" | "assistant" | "system";
  content: string;
  prompt_tokens: number;
  completion_tokens: number;
  total_tokens: number;
  cached_tokens: number;
  /** Chain-of-thought text (reasoning_content), assistant turns only. */
  reasoning?: string;
  /** JSON string of `ToolCallInfo[]`, assistant turns only. */
  tool_calls?: string;
  created_at: string;
}

// AI Assistant skills (Markdown skill files with frontmatter)

/** Where a skill came from. Builtin skills ship with the app; User marks any
 *  skill the user created or edited. The value comes straight from the Rust
 *  enum (serde `rename_all = "lowercase"`). */
export type SkillSource = "builtin" | "user";

/**
 * A single AI assistant skill: a Markdown body plus frontmatter metadata.
 * The body is appended to the system prompt when the skill activates. `id`
 * is the file stem and the stable handle used by `/` / `@` invocation and by
 * `active_skills` on the chat request.
 */
export interface Skill {
  id: string;
  name: string;
  description: string;
  trigger: string[];
  enabled: boolean;
  content: string;
  source: SkillSource;
  updatedAt: string;
}

// Quote Provider Config
export type QuoteProvider = "yahoo" | "eastmoney" | "xueqiu";

export interface QuoteProviderConfig {
  us_provider: QuoteProvider;
  hk_provider: QuoteProvider;
  cn_provider: QuoteProvider;
  xueqiu_cookie?: string | null;
  xueqiu_u?: string | null;
  /** A-share: adjust avg_cost on SELL and dividend. Default true. */
  cn_adjust_sell_pay_cost?: boolean;
  /** US stock: adjust avg_cost on SELL and dividend. Default false. */
  us_adjust_sell_pay_cost?: boolean;
  /** HK stock: adjust avg_cost on SELL and dividend. Default false. */
  hk_adjust_sell_pay_cost?: boolean;
}

// Options Management types
export interface OptionRecord {
  id: string;
  account_id: string;
  option_symbol: string;
  underlying: string;
  expiry_date: string;
  strike_price: number;
  option_type: "P" | "C";
  action: "SELL" | "BUY";
  code: string;
  quantity: number;
  price: number;
  amount: number;
  commission: number;
  fee: number;
  traded_at: string | null;
  settled_at: string | null;
  created_at: string;
}

export interface OptionContract {
  id: string;
  option_symbol: string;
  underlying: string;
  expiry_date: string;
  strike_price: number;
  option_type: "P" | "C";
  contracts: number;
  open_price: number;
  open_amount: number;
  commission: number;
  traded_at: string | null;
  close_price: number | null;
  close_code: string | null;
  status: "active" | "expired" | "assigned" | "closed";
  account_id: string;
}

export interface ExpiredOptionStats {
  total_contracts: number;
  assigned_contracts: number;
  expired_contracts: number;
  assignment_ratio: number;
}

export interface SellPutSimulation {
  underlying: string;
  contracts: PutContractSimulation[];
  total_cash_needed: number;
}

export interface PutContractSimulation {
  option_symbol: string;
  strike_price: number;
  contracts: number;
  would_be_assigned: boolean;
  cash_needed: number;
}

export interface SellCallSimulation {
  underlying: string;
  contracts: CallContractSimulation[];
  total_shares_needed: number;
}

export interface CallContractSimulation {
  option_symbol: string;
  strike_price: number;
  contracts: number;
  would_be_assigned: boolean;
  shares_needed: number;
}

export interface ImportOptionsResult {
  imported: number;
  skipped: number;
  errors: string[];
}

export interface StockSplit {
  id: number;
  stock_code: string;
  split_date: string;
  ratio_from: number;
  ratio_to: number;
  created_at: string;
}

export interface OptionShareLot {
  id: number;
  stock_code: string;
  shares_per_contract: number;
  created_at: string;
}

export interface StockPriceInput {
  symbol: string;
  price: number;
}

// --- Dividend analysis ---
// Field names match the backend's camelCase serialization.

export interface AccountDividend {
  accountId: string;
  accountName: string;
  total: number;
}

export interface DividendRow {
  symbol: string;
  name: string;
  /** [accountId, amount][] — amounts in the market's native currency. */
  perAccount: [string, number][];
  total: number;
}

export interface MarketDividend {
  market: Market;
  currency: Currency;
  accounts: AccountDividend[];
  rows: DividendRow[];
  total: number;
}

export interface DividendAnalysis {
  year: number;
  markets: MarketDividend[];
  grandTotal: number;
}
