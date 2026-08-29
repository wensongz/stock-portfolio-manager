import type {
  Currency,
  Market,
  MetricAvailability,
  MetricStatus,
  StockReviewAnnotation,
  StockReviewFilters,
  StockReviewPeriodPreset,
  StockReviewReport,
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
const CURRENCIES: Currency[] = ["USD", "CNY", "HKD"];

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

export interface StockReviewAnnotationActionContext {
  actionId: string;
  accountId: string;
  symbol: string;
}

export interface StockReviewAnnotationCampaignContext {
  campaignId: string;
  accountIds: string[];
  actionIds: string[];
  symbol: string;
  startedAt: string;
  endedAt: string | null;
}

export interface StockReviewAnnotationDisplayContext {
  endDate: string;
  accountId: string | null;
  actions: StockReviewAnnotationActionContext[];
  campaigns: StockReviewAnnotationCampaignContext[];
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

function daysInCalendarMonth(year: number, month: number): number {
  return new Date(Date.UTC(year, month, 0)).getUTCDate();
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

interface AnnotationEconomicDates {
  effectiveDate: string | null;
  effectiveStart: string | null;
  effectiveEnd: string | null;
  snapshotDate: string | null;
}

function annotationEconomicDates(valueJson: string): AnnotationEconomicDates | null {
  let value: unknown;
  try {
    value = JSON.parse(valueJson);
  } catch {
    return null;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const object = value as Record<string, unknown>;
  const date = (key: string): string | null | undefined => {
    if (!(key in object)) return null;
    return isValidDateOnly(object[key]) ? object[key] : undefined;
  };
  const effectiveDate = date("effective_date");
  const effectiveStart = date("effective_start");
  const effectiveEnd = date("effective_end");
  const snapshotDate = date("snapshot_date");
  if (
    effectiveDate === undefined ||
    effectiveStart === undefined ||
    effectiveEnd === undefined ||
    snapshotDate === undefined ||
    (effectiveStart != null && effectiveEnd != null && effectiveStart > effectiveEnd)
  ) {
    return null;
  }
  return { effectiveDate, effectiveStart, effectiveEnd, snapshotDate };
}

function normalizedStockIdentity(value: string | null): string | null {
  const normalized = value?.trim().toUpperCase() ?? "";
  return normalized || null;
}

function annotationVisibleAsOf(
  annotation: StockReviewAnnotation,
  context: StockReviewAnnotationDisplayContext,
): AnnotationEconomicDates | null {
  const dates = annotationEconomicDates(annotation.value_json);
  if (!dates) return null;
  const explicitDate = dates.effectiveDate ?? dates.snapshotDate;
  if (explicitDate != null && explicitDate > context.endDate) return null;
  if (dates.effectiveStart != null && dates.effectiveStart > context.endDate) return null;
  if (
    annotation.account_id != null &&
    context.accountId != null &&
    annotation.account_id !== context.accountId
  ) {
    return null;
  }
  return dates;
}

export function createStockReviewAnnotationDisplayContext(
  source:
    | StockReviewReport
    | StockReviewAnnotationDisplayContext,
): StockReviewAnnotationDisplayContext {
  if ("methodology" in source) {
    return {
      endDate: source.methodology.query.end_date,
      accountId: source.methodology.query.account_id,
      actions: source.actions.map((action) => ({
        actionId: action.action_id,
        accountId: action.account_id,
        symbol: action.symbol,
      })),
      campaigns: source.campaigns.map((campaign) => ({
        campaignId: campaign.campaign_id,
        accountIds: [...campaign.account_ids],
        actionIds: [...campaign.action_ids],
        symbol: campaign.symbol,
        startedAt: campaign.started_at,
        endedAt: campaign.ended_at,
      })),
    };
  }
  return {
    endDate: source.endDate,
    accountId: source.accountId,
    actions: source.actions.map((action) => ({ ...action })),
    campaigns: source.campaigns.map((campaign) => ({
      ...campaign,
      accountIds: [...campaign.accountIds],
      actionIds: [...campaign.actionIds],
    })),
  };
}

export function isStockReviewAnnotationVisibleInReport(
  annotation: StockReviewAnnotation,
  context: StockReviewAnnotationDisplayContext,
): boolean {
  if (!annotationVisibleAsOf(annotation, context)) return false;
  const annotationSymbol = normalizedStockIdentity(annotation.symbol);
  if (annotation.scope_type === "period") return annotationSymbol == null;
  if (annotation.scope_type === "action") {
    const action = context.actions.find(
      (candidate) => candidate.actionId === annotation.scope_key,
    );
    return Boolean(
      action &&
        (annotation.account_id == null || annotation.account_id === action.accountId) &&
        (annotationSymbol == null ||
          annotationSymbol === normalizedStockIdentity(action.symbol)),
    );
  }
  if (annotation.scope_type === "campaign") {
    const campaign = context.campaigns.find(
      (candidate) => candidate.campaignId === annotation.scope_key,
    );
    return Boolean(
      campaign &&
        (annotation.account_id == null ||
          campaign.accountIds.includes(annotation.account_id)) &&
        (annotationSymbol == null ||
          annotationSymbol === normalizedStockIdentity(campaign.symbol)),
    );
  }
  if (annotation.scope_type !== "stock") return false;
  const scopeSymbol = normalizedStockIdentity(annotation.scope_key);
  if (scopeSymbol == null || (annotationSymbol != null && annotationSymbol !== scopeSymbol)) {
    return false;
  }
  return (
    context.actions.some(
      (action) =>
        normalizedStockIdentity(action.symbol) === scopeSymbol &&
        (annotation.account_id == null || annotation.account_id === action.accountId),
    ) ||
    context.campaigns.some(
      (campaign) =>
        normalizedStockIdentity(campaign.symbol) === scopeSymbol &&
        (annotation.account_id == null || campaign.accountIds.includes(annotation.account_id)),
    )
  );
}

export function isStockReviewAnnotationVisibleInCampaign(
  annotation: StockReviewAnnotation,
  context: StockReviewAnnotationDisplayContext,
  campaignId: string,
): boolean {
  if (!isStockReviewAnnotationVisibleInReport(annotation, context)) return false;
  const campaign = context.campaigns.find(
    (candidate) => candidate.campaignId === campaignId,
  );
  if (!campaign) return false;
  if (annotation.scope_type === "period") {
    return annotation.account_id == null && annotation.symbol == null;
  }
  if (annotation.scope_type === "campaign") return annotation.scope_key === campaignId;
  if (annotation.scope_type === "action") {
    return campaign.actionIds.includes(annotation.scope_key);
  }
  if (annotation.scope_type !== "stock") return false;
  if (
    normalizedStockIdentity(annotation.scope_key) !==
      normalizedStockIdentity(campaign.symbol) ||
    (annotation.symbol != null &&
      normalizedStockIdentity(annotation.symbol) !==
        normalizedStockIdentity(campaign.symbol)) ||
    (annotation.account_id != null &&
      !campaign.accountIds.includes(annotation.account_id))
  ) {
    return false;
  }

  const dates = annotationVisibleAsOf(annotation, context);
  if (!dates || !isValidDateOnly(campaign.startedAt.slice(0, 10))) return false;
  const campaignStart = campaign.startedAt.slice(0, 10);
  const campaignEnd =
    campaign.endedAt != null && isValidDateOnly(campaign.endedAt.slice(0, 10))
      ? campaign.endedAt.slice(0, 10) < context.endDate
        ? campaign.endedAt.slice(0, 10)
        : context.endDate
      : context.endDate;
  const explicitDate = dates.effectiveDate ?? dates.snapshotDate;
  if (explicitDate != null) {
    return explicitDate >= campaignStart && explicitDate <= campaignEnd;
  }
  if (dates.effectiveStart != null || dates.effectiveEnd != null) {
    const annotationStart = dates.effectiveStart ?? "0000-01-01";
    const annotationEnd = dates.effectiveEnd ?? "9999-12-31";
    return annotationStart <= campaignEnd && annotationEnd >= campaignStart;
  }
  return (
    context.campaigns.filter(
      (candidate) =>
        normalizedStockIdentity(candidate.symbol) ===
          normalizedStockIdentity(campaign.symbol) &&
        (annotation.account_id == null ||
          candidate.accountIds.includes(annotation.account_id)),
    ).length === 1
  );
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
    const priorAnniversary = {
      year: end.year - 1,
      month: end.month,
      day: Math.min(end.day, daysInCalendarMonth(end.year - 1, end.month)),
    };
    return {
      startDate: formatCalendarDate(shiftCalendarDays(priorAnniversary, 1)),
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
  if (
    Object.keys(record).length !== allowedKeys.size ||
    Object.keys(record).some((key) => !allowedKeys.has(key))
  ) {
    return null;
  }
  if (!PERIOD_PRESETS.includes(record.periodPreset as StockReviewPeriodPreset)) return null;
  if (!CURRENCIES.includes(record.baseCurrency as Currency)) return null;

  const accountId = normalizedNullableString(record.accountId);
  const benchmarkSymbol = normalizedNullableString(record.benchmarkSymbol);
  if (accountId === undefined || benchmarkSymbol === undefined) return null;
  if (record.market !== null && !MARKETS.includes(record.market as Market)) return null;

  let storedRange: StockReviewDateRange;
  try {
    storedRange = validateDateRange({
      startDate: record.startDate as string,
      endDate: record.endDate as string,
    });
  } catch {
    return null;
  }

  const periodPreset = record.periodPreset as StockReviewPeriodPreset;
  let range: StockReviewDateRange;
  try {
    range = getStockReviewDateRange(
      periodPreset,
      now,
      periodPreset === "CUSTOM" ? storedRange : undefined,
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
