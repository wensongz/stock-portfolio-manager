import type {
  Currency,
  Market,
  MetricAvailability,
  MetricStatus,
  StockReviewFilters,
  StockReviewPeriodPreset,
} from "../../types";

export const STOCK_REVIEW_FILTERS_STORAGE_KEY = "review_stock_filters_v1";

const PERIOD_PRESETS: StockReviewPeriodPreset[] = [
  "QTD",
  "PREV_QUARTER",
  "YTD",
  "1Y",
  "CUSTOM",
];
const MARKETS: Market[] = ["US", "CN", "HK"];

const PORTFOLIO_PROMPT =
  "请基于本期确定性股票复盘报告，分析整体调仓是否创造价值、收益是否依赖少数操作、风险结构是否改善，以及最值得进一步复盘的三项操作。请严格区分确定性事实、事后结果和缺失的决策背景。";
const CAMPAIGN_PROMPT =
  "请复盘当前股票Campaign，区分确定性事实、事后推断和缺失背景，重点分析加减仓节奏、仓位变化及其对组合的贡献。";

interface ReviewFilterStorage {
  getItem: (key: string) => string | null;
  setItem: (key: string, value: string) => void;
}

export interface StockReviewDateRange {
  startDate: string;
  endDate: string;
}

export interface StockReviewToolArguments {
  start_date: string;
  end_date: string;
  base_currency: Currency;
  account_id?: string;
  market?: Market;
  benchmark_symbol?: string;
  symbol?: string;
  campaign_id?: string;
}

export interface StockReviewAiPrefill {
  activeSkill: "stock-review";
  prompt: string;
  toolName: "get_stock_review";
  toolArguments: StockReviewToolArguments;
  autoSend: false;
}

export interface StockReviewMetricDisplay {
  value: number | null;
  status: MetricStatus;
  note: string | null;
  displayValue: string;
}

interface CalendarDate {
  year: number;
  month: number;
  day: number;
}

function shanghaiCalendarDate(now: Date): CalendarDate {
  if (Number.isNaN(now.getTime())) throw new RangeError("当前日期无效");
  const parts = new Intl.DateTimeFormat("en-US", {
    timeZone: "Asia/Shanghai",
    year: "numeric",
    month: "2-digit",
    day: "2-digit",
  }).formatToParts(now);
  const value = (type: Intl.DateTimeFormatPartTypes) =>
    Number(parts.find((part) => part.type === type)?.value);
  return { year: value("year"), month: value("month"), day: value("day") };
}

function formatCalendarDate({ year, month, day }: CalendarDate): string {
  return `${String(year).padStart(4, "0")}-${String(month).padStart(2, "0")}-${String(day).padStart(2, "0")}`;
}

function shiftCalendarDays(date: CalendarDate, days: number): CalendarDate {
  const shifted = new Date(Date.UTC(date.year, date.month - 1, date.day + days));
  return {
    year: shifted.getUTCFullYear(),
    month: shifted.getUTCMonth() + 1,
    day: shifted.getUTCDate(),
  };
}

function isValidDateOnly(value: unknown): value is string {
  if (typeof value !== "string" || !/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const [year, month, day] = value.split("-").map(Number);
  return formatCalendarDate(
    shiftCalendarDays({ year, month, day }, 0),
  ) === value;
}

function validateDateRange(range: StockReviewDateRange): StockReviewDateRange {
  if (!isValidDateOnly(range.startDate) || !isValidDateOnly(range.endDate)) {
    throw new RangeError("请选择有效日期");
  }
  if (range.startDate > range.endDate) {
    throw new RangeError("开始日期不能晚于结束日期");
  }
  return range;
}

/**
 * Converts a backend metric into a render-safe scalar without changing its
 * status or deriving a replacement value. Formatting units stays with the UI.
 */
export function mapStockReviewMetricForDisplay(
  value: number | null,
  availability: MetricAvailability,
): StockReviewMetricDisplay {
  const safeValue = value != null && Number.isFinite(value) ? value : null;
  return {
    value: safeValue,
    status: availability.status,
    note: availability.note,
    displayValue: safeValue == null ? "—" : String(safeValue),
  };
}

export function getStockReviewDateRange(
  preset: StockReviewPeriodPreset,
  now: Date = new Date(),
  customRange?: StockReviewDateRange,
): StockReviewDateRange {
  if (preset === "CUSTOM") {
    if (!customRange) throw new RangeError("自定义周期需要有效日期");
    return validateDateRange(customRange);
  }

  const end = shanghaiCalendarDate(now);
  if (preset === "YTD") {
    return { startDate: `${end.year}-01-01`, endDate: formatCalendarDate(end) };
  }
  if (preset === "1Y") {
    return {
      startDate: formatCalendarDate(shiftCalendarDays(end, -364)),
      endDate: formatCalendarDate(end),
    };
  }

  const quarterStartMonth = Math.floor((end.month - 1) / 3) * 3 + 1;
  if (preset === "QTD") {
    return {
      startDate: formatCalendarDate({ year: end.year, month: quarterStartMonth, day: 1 }),
      endDate: formatCalendarDate(end),
    };
  }

  const currentQuarterStart = new Date(Date.UTC(end.year, quarterStartMonth - 1, 1));
  const previousQuarterEnd = new Date(currentQuarterStart.getTime() - 86_400_000);
  const previousQuarterStart = new Date(
    Date.UTC(previousQuarterEnd.getUTCFullYear(), previousQuarterEnd.getUTCMonth() - 2, 1),
  );
  return {
    startDate: formatCalendarDate({
      year: previousQuarterStart.getUTCFullYear(),
      month: previousQuarterStart.getUTCMonth() + 1,
      day: 1,
    }),
    endDate: formatCalendarDate({
      year: previousQuarterEnd.getUTCFullYear(),
      month: previousQuarterEnd.getUTCMonth() + 1,
      day: previousQuarterEnd.getUTCDate(),
    }),
  };
}

export function createDefaultStockReviewFilters(
  now: Date = new Date(),
  baseCurrency: Currency = "USD",
): StockReviewFilters {
  return {
    accountId: null,
    periodPreset: "YTD",
    ...getStockReviewDateRange("YTD", now),
    market: null,
    benchmarkSymbol: null,
    baseCurrency,
  };
}

function normalizedNullableString(value: unknown): string | null | undefined {
  if (value === null) return null;
  if (typeof value !== "string") return undefined;
  const normalized = value.trim();
  return normalized ? normalized : undefined;
}

function parseStoredFilters(
  value: unknown,
  now: Date,
  baseCurrency: Currency,
): StockReviewFilters | null {
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  const allowedKeys = new Set([
    "accountId",
    "periodPreset",
    "startDate",
    "endDate",
    "market",
    "benchmarkSymbol",
    "baseCurrency",
  ]);
  if (Object.keys(record).some((key) => !allowedKeys.has(key))) return null;
  if (!PERIOD_PRESETS.includes(record.periodPreset as StockReviewPeriodPreset)) return null;

  const accountId = normalizedNullableString(record.accountId);
  const benchmarkSymbol = normalizedNullableString(record.benchmarkSymbol);
  if (accountId === undefined || benchmarkSymbol === undefined) return null;
  if (record.market !== null && !MARKETS.includes(record.market as Market)) return null;

  const periodPreset = record.periodPreset as StockReviewPeriodPreset;
  let range: StockReviewDateRange;
  try {
    range = getStockReviewDateRange(
      periodPreset,
      now,
      periodPreset === "CUSTOM"
        ? { startDate: record.startDate as string, endDate: record.endDate as string }
        : undefined,
    );
  } catch {
    return null;
  }

  return {
    accountId,
    periodPreset,
    ...range,
    market: record.market as Market | null,
    benchmarkSymbol,
    baseCurrency,
  };
}

export function loadStockReviewFilters(
  storage: Pick<ReviewFilterStorage, "getItem">,
  now: Date = new Date(),
  baseCurrency: Currency = "USD",
): StockReviewFilters {
  const fallback = createDefaultStockReviewFilters(now, baseCurrency);
  const stored = storage.getItem(STOCK_REVIEW_FILTERS_STORAGE_KEY);
  if (stored == null) return fallback;
  try {
    return parseStoredFilters(JSON.parse(stored), now, baseCurrency) ?? fallback;
  } catch {
    return fallback;
  }
}

export function saveStockReviewFilters(
  storage: Pick<ReviewFilterStorage, "setItem">,
  filters: StockReviewFilters,
): void {
  validateDateRange({ startDate: filters.startDate, endDate: filters.endDate });
  storage.setItem(
    STOCK_REVIEW_FILTERS_STORAGE_KEY,
    JSON.stringify({
      accountId: filters.accountId,
      periodPreset: filters.periodPreset,
      startDate: filters.startDate,
      endDate: filters.endDate,
      market: filters.market,
      benchmarkSymbol: filters.benchmarkSymbol,
      baseCurrency: filters.baseCurrency,
    }),
  );
}

function buildToolArguments(filters: StockReviewFilters): StockReviewToolArguments {
  validateDateRange({ startDate: filters.startDate, endDate: filters.endDate });
  return {
    start_date: filters.startDate,
    end_date: filters.endDate,
    base_currency: filters.baseCurrency,
    ...(filters.accountId ? { account_id: filters.accountId } : {}),
    ...(filters.market ? { market: filters.market } : {}),
    ...(filters.benchmarkSymbol ? { benchmark_symbol: filters.benchmarkSymbol } : {}),
  };
}

export function buildStockReviewAiPrefill(
  filters: StockReviewFilters,
): StockReviewAiPrefill {
  return {
    activeSkill: "stock-review",
    prompt: PORTFOLIO_PROMPT,
    toolName: "get_stock_review",
    toolArguments: buildToolArguments(filters),
    autoSend: false,
  };
}

export function buildStockCampaignAiPrefill(
  filters: StockReviewFilters,
  symbol: string,
  campaignId: string,
): StockReviewAiPrefill {
  const normalizedSymbol = symbol.trim().toUpperCase();
  const normalizedCampaignId = campaignId.trim();
  if (!normalizedSymbol || !normalizedCampaignId) {
    throw new RangeError("股票代码和 Campaign ID 不能为空");
  }
  return {
    activeSkill: "stock-review",
    prompt: CAMPAIGN_PROMPT,
    toolName: "get_stock_review",
    toolArguments: {
      ...buildToolArguments(filters),
      symbol: normalizedSymbol,
      campaign_id: normalizedCampaignId,
    },
    autoSend: false,
  };
}
