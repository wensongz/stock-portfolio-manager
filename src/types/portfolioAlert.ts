export type PortfolioAlertMarket = "CN" | "US" | "HK";
export type PortfolioAlertCurrency = "USD" | "CNY" | "HKD";

export type PortfolioAlertScope =
  | { kind: "OVERALL"; market: null; accountId: null }
  | { kind: "MARKET"; market: PortfolioAlertMarket; accountId: null }
  | { kind: "ACCOUNT"; market: null; accountId: string };

export type PortfolioAlertDataStatus =
  | "READY"
  | "EMPTY"
  | "INCOMPLETE"
  | "INVALID_CONFIG";

export type PortfolioAlertBreachKind =
  | "CATEGORY_DEVIATION"
  | "CONCENTRATION";

export type AllocationDirection = "OVERWEIGHT" | "UNDERWEIGHT";

export type PortfolioAlertBreachDirection =
  | "OVERWEIGHT"
  | "UNDERWEIGHT"
  | "ABOVE_LIMIT";

export type PortfolioAlertMissingDataReason =
  | "cached quote is unavailable"
  | "cached quote has a negative value"
  | `exchange rate from ${PortfolioAlertCurrency} to ${PortfolioAlertCurrency} is unavailable`;

export interface PortfolioAlertTarget {
  categoryId: string;
  targetPercent: number;
}

export interface PortfolioAlertConfig {
  id: string;
  scope: PortfolioAlertScope;
  baseCurrency: PortfolioAlertCurrency;
  deviationThreshold: number;
  concentrationThreshold: number;
  isActive: boolean;
  targets: PortfolioAlertTarget[];
  lastSnapshot: PortfolioAlertSnapshot | null;
  lastEvaluatedAt: string | null;
}

export interface SavePortfolioAlertConfigInput {
  id: string | null;
  scope: PortfolioAlertScope;
  baseCurrency: PortfolioAlertCurrency;
  deviationThreshold: number;
  concentrationThreshold: number;
  isActive: boolean;
  targets: PortfolioAlertTarget[];
}

export interface CategoryAllocation {
  categoryId: string | null;
  categoryName: string;
  categoryColor: string;
  categoryIcon: string;
  targetPercent: number;
  currentPercent: number;
  relativeDeviationPercent: number | null;
  currentMarketValue: number;
  targetMarketValue: number;
  rebalanceAmount: number;
  direction: AllocationDirection | null;
}

export interface ConcentrationAlert {
  market: PortfolioAlertMarket;
  symbol: string;
  normalizedSymbol: string;
  name: string;
  categoryId: string | null;
  marketValue: number;
  positionPercent: number;
  thresholdPercent: number;
}

export interface PortfolioAlertSnapshot {
  configId: string;
  scope: PortfolioAlertScope;
  baseCurrency: PortfolioAlertCurrency;
  evaluatedAt: string;
  totalMarketValue: number;
  categories: CategoryAllocation[];
  concentrations: ConcentrationAlert[];
}

export interface MissingPortfolioAlertData {
  market: PortfolioAlertMarket | null;
  symbol: string | null;
  currency: PortfolioAlertCurrency | null;
  reason: PortfolioAlertMissingDataReason;
}

export interface PortfolioAlertBreach {
  configId: string;
  breachKey: string;
  breachKind: PortfolioAlertBreachKind;
  direction: PortfolioAlertBreachDirection;
  firstTriggeredAt: string;
  lastSeenAt: string;
}

export interface PortfolioAlertNotification {
  configId: string;
  scope: PortfolioAlertScope;
  breach: PortfolioAlertBreach;
  message: string;
  triggeredAt: string;
}

export interface PortfolioAlertEvaluation {
  status: PortfolioAlertDataStatus;
  snapshot: PortfolioAlertSnapshot | null;
  stale: boolean;
  missingData: MissingPortfolioAlertData[];
  activeBreaches: PortfolioAlertBreach[];
  newlyTriggered: PortfolioAlertBreach[];
}

export interface PortfolioAlertView {
  config: PortfolioAlertConfig | null;
  evaluation: PortfolioAlertEvaluation | null;
}
