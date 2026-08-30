import type {
  Currency,
  Market,
  StockOperationDataQuality,
  StockOperationReviewFilters,
  StockOperationReviewSummary,
  StockOperationSecuritySummary,
  StockReviewPeriodPreset,
} from "../../types";
import { getStockReviewDateRange } from "./stockReviewViewModel.ts";

export const STOCK_OPERATION_REVIEW_FILTERS_STORAGE_KEY =
  "review_stock_operation_filters_v1";
const LEGACY_FILTERS_STORAGE_KEY = "review_stock_filters_v1";
const PRESETS: StockReviewPeriodPreset[] = [
  "QTD",
  "PREV_QUARTER",
  "YTD",
  "1Y",
  "CUSTOM",
];
const MARKETS: Market[] = ["US", "CN", "HK"];
const CURRENCIES: Currency[] = ["USD", "CNY", "HKD"];

interface FilterStorage {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

export function getStockOperationReviewDateRange(
  preset: StockReviewPeriodPreset,
  now: Date = new Date(),
  custom?: { startDate: string; endDate: string },
) {
  return getStockReviewDateRange(preset, now, custom);
}

export function createDefaultStockOperationReviewFilters(
  now: Date = new Date(),
  baseCurrency: Currency = "USD",
): StockOperationReviewFilters {
  return {
    accountId: null,
    periodPreset: "YTD",
    ...getStockOperationReviewDateRange("YTD", now),
    market: null,
    baseCurrency,
  };
}

function nullableString(value: unknown): string | null | undefined {
  if (value === null) return null;
  if (typeof value !== "string") return undefined;
  const normalized = value.trim();
  return normalized || undefined;
}

function validDate(value: unknown): value is string {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const parsed = new Date(`${value}T00:00:00Z`);
  return !Number.isNaN(parsed.getTime()) && parsed.toISOString().slice(0, 10) === value;
}

function parseFilters(
  value: unknown,
  now: Date,
): StockOperationReviewFilters | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (!PRESETS.includes(record.periodPreset as StockReviewPeriodPreset)) return null;
  if (!CURRENCIES.includes(record.baseCurrency as Currency)) return null;
  if (record.market !== null && !MARKETS.includes(record.market as Market)) return null;
  const accountId = nullableString(record.accountId);
  if (accountId === undefined) return null;
  if (!validDate(record.startDate) || !validDate(record.endDate)) return null;
  if (record.startDate > record.endDate) return null;
  const periodPreset = record.periodPreset as StockReviewPeriodPreset;
  const range = getStockOperationReviewDateRange(
    periodPreset,
    now,
    periodPreset === "CUSTOM"
      ? { startDate: record.startDate, endDate: record.endDate }
      : undefined,
  );
  return {
    accountId,
    periodPreset,
    ...range,
    market: record.market as Market | null,
    baseCurrency: record.baseCurrency as Currency,
  };
}

export function loadStockOperationReviewFilters(
  storage: Pick<FilterStorage, "getItem">,
  now: Date = new Date(),
  baseCurrency: Currency = "USD",
): StockOperationReviewFilters {
  const fallback = createDefaultStockOperationReviewFilters(now, baseCurrency);
  const raw =
    storage.getItem(STOCK_OPERATION_REVIEW_FILTERS_STORAGE_KEY) ??
    storage.getItem(LEGACY_FILTERS_STORAGE_KEY);
  if (raw == null) return fallback;
  try {
    return parseFilters(JSON.parse(raw), now) ?? fallback;
  } catch {
    return fallback;
  }
}

export function saveStockOperationReviewFilters(
  storage: Pick<FilterStorage, "setItem">,
  filters: StockOperationReviewFilters,
) {
  if (!validDate(filters.startDate) || !validDate(filters.endDate)) {
    throw new Error("股票操作复盘日期格式无效");
  }
  storage.setItem(
    STOCK_OPERATION_REVIEW_FILTERS_STORAGE_KEY,
    JSON.stringify({
      accountId: filters.accountId,
      periodPreset: filters.periodPreset,
      startDate: filters.startDate,
      endDate: filters.endDate,
      market: filters.market,
      baseCurrency: filters.baseCurrency,
    }),
  );
}

export function formatOperationCurrency(
  value: number | null,
  currency: Currency | string,
) {
  if (value == null) return "—";
  return new Intl.NumberFormat("en-US", {
    style: "currency",
    currency,
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value);
}

export function formatOperationPercent(value: number | null) {
  return value == null ? "—" : `${(value * 100).toFixed(2)}%`;
}

export const formatOperationWeight = formatOperationPercent;

export function formatStockOperationIdentity(symbol: string, name: string) {
  const normalizedName = name.trim();
  return normalizedName ? `${symbol} · ${normalizedName}` : symbol;
}

export function buildStockOperationIdentityDisplay(
  reportAccountId: string | null,
  _market: string,
  accountName: string,
) {
  const showAccount = reportAccountId == null;
  return {
    columnTitle: showAccount ? "股票 / 账户" : "股票",
    securitySecondary: showAccount ? accountName : null,
    actionSecondary: showAccount ? accountName : null,
  };
}

export interface StockOperationSummaryCardView {
  title: string;
  primary: string;
  metrics: Array<{ label: string; value: string }>;
  description: string;
}

export function buildStockOperationSummaryCards(
  summary: StockOperationReviewSummary,
  currency: Currency,
): StockOperationSummaryCardView[] {
  const groupCard = (
    title: string,
    group: StockOperationReviewSummary["total"],
    description: string,
  ): StockOperationSummaryCardView => ({
    title,
    primary: formatOperationCurrency(group.price_effect_base, currency),
    metrics: [
      { label: "相对基准", value: formatOperationPercent(group.weighted_excess_return) },
      { label: "正向金额占比", value: formatOperationPercent(group.positive_notional_ratio) },
      {
        label: "正向 / 负向 / 缺数据",
        value: `${group.positive_count} / ${group.negative_count} / ${group.missing_effect_count}`,
      },
    ],
    description,
  });
  return [
    groupCard("操作总效果", summary.total, "截至复盘期末，相对不执行各项操作的价格效果合计。"),
    groupCard("买入与加仓", summary.buys, "买入和加仓截至期末的价格效果。"),
    groupCard("减仓与清仓", summary.sells, "卖出后的避损或机会损失，截至复盘期末评价。"),
    {
      title: "仓位影响",
      primary: formatOperationWeight(summary.position_impact.largest_absolute_weight_change),
      metrics: [
        { label: "投入", value: formatOperationCurrency(summary.position_impact.invested_amount_base, currency) },
        { label: "回收", value: formatOperationCurrency(summary.position_impact.recovered_amount_base, currency) },
        { label: "费用", value: formatOperationCurrency(summary.position_impact.total_fees_base, currency) },
      ],
      description: `最大单次估算权重变化；缺少权重 ${summary.position_impact.missing_weight_count} 项。`,
    },
  ];
}

export function buildStockOperationReviewQualityText(
  quality: StockOperationDataQuality,
) {
  const gaps: string[] = [];
  if (quality.missing_end_price_count) gaps.push(`${quality.missing_end_price_count} 项缺少期末价`);
  if (quality.missing_benchmark_count) gaps.push(`${quality.missing_benchmark_count} 项缺少基准`);
  if (quality.missing_fx_count) gaps.push(`${quality.missing_fx_count} 项缺少汇率`);
  if (quality.missing_weight_count) gaps.push(`${quality.missing_weight_count} 项缺少权重估算`);
  return `共分析 ${quality.action_count} 项操作${gaps.length ? `；${gaps.join("，")}` : ""}。`;
}

export type StockOperationSecuritySortKey =
  | "effect"
  | "notional"
  | "benchmark"
  | "weight";

function nullableDescending(left: number | null, right: number | null) {
  if (left == null && right == null) return 0;
  if (left == null) return 1;
  if (right == null) return -1;
  return right - left;
}

export function sortStockOperationSecurities<
  T extends Pick<
    StockOperationSecuritySummary,
    | "symbol"
    | "price_effect_base"
    | "buy_notional_local"
    | "sell_notional_local"
    | "weighted_excess_return"
    | "largest_absolute_weight_change"
  >,
>(rows: T[], key: StockOperationSecuritySortKey): T[] {
  return [...rows].sort((left, right) => {
    const comparison = key === "effect"
      ? nullableDescending(left.price_effect_base, right.price_effect_base)
      : key === "benchmark"
        ? nullableDescending(left.weighted_excess_return, right.weighted_excess_return)
        : key === "weight"
          ? nullableDescending(left.largest_absolute_weight_change, right.largest_absolute_weight_change)
          : (right.buy_notional_local + right.sell_notional_local) -
            (left.buy_notional_local + left.sell_notional_local);
    return comparison || left.symbol.localeCompare(right.symbol);
  });
}

export function buildStockOperationReviewAiPrefill(
  filters: StockOperationReviewFilters,
) {
  return {
    activeSkill: "stock-review",
    prompt: "请基于确定性股票操作复盘报告，分析截至复盘期末的操作效果、仓位影响与相对市场表现。",
    toolName: "get_stock_review" as const,
    toolArguments: {
      start_date: filters.startDate,
      end_date: filters.endDate,
      base_currency: filters.baseCurrency,
      ...(filters.accountId ? { account_id: filters.accountId } : {}),
      ...(filters.market ? { market: filters.market } : {}),
    },
    autoSend: false as const,
  };
}
