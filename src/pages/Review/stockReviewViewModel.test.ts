// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  STOCK_REVIEW_FILTERS_STORAGE_KEY,
  buildStockCampaignAiPrefill,
  buildStockReviewAiPrefill,
  createStockReviewAnnotationDisplayContext,
  createDefaultStockReviewFilters,
  doesStockReviewAnnotationApplyToCampaign,
  getStockReviewDateRange,
  isStockReviewAnnotationInDisplayContext,
  loadStockReviewFilters,
  mapStockReviewMetricForDisplay,
  saveStockReviewFilters,
} from "./stockReviewViewModel.ts";

const now = new Date("2026-08-28T00:00:00+08:00");

function memoryStorage(initial: Record<string, string> = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem(key: string) {
      return values.get(key) ?? null;
    },
    setItem(key: string, value: string) {
      values.set(key, value);
    },
  };
}

test("date presets use Shanghai calendar dates without a UTC day shift", () => {
  assert.deepEqual(getStockReviewDateRange("YTD", now), {
    startDate: "2026-01-01",
    endDate: "2026-08-28",
  });
  assert.deepEqual(getStockReviewDateRange("QTD", now), {
    startDate: "2026-07-01",
    endDate: "2026-08-28",
  });
  assert.deepEqual(getStockReviewDateRange("PREV_QUARTER", now), {
    startDate: "2026-04-01",
    endDate: "2026-06-30",
  });
  assert.deepEqual(getStockReviewDateRange("1Y", now), {
    startDate: "2025-08-29",
    endDate: "2026-08-28",
  });
});

test("date presets handle Q1 and a previous quarter crossing the year boundary", () => {
  const january = new Date("2026-01-01T00:15:00+08:00");
  assert.deepEqual(getStockReviewDateRange("QTD", january), {
    startDate: "2026-01-01",
    endDate: "2026-01-01",
  });
  assert.deepEqual(getStockReviewDateRange("PREV_QUARTER", january), {
    startDate: "2025-10-01",
    endDate: "2025-12-31",
  });
  assert.deepEqual(getStockReviewDateRange("YTD", january), {
    startDate: "2026-01-01",
    endDate: "2026-01-01",
  });
});

test("1Y uses an anniversary-exclusive inclusive range across leap day", () => {
  const leapDay = new Date("2024-02-29T23:30:00+08:00");
  assert.deepEqual(getStockReviewDateRange("1Y", leapDay), {
    startDate: "2023-03-01",
    endDate: "2024-02-29",
  });
  assert.deepEqual(
    getStockReviewDateRange("1Y", new Date("2024-03-01T00:15:00+08:00")),
    { startDate: "2023-03-02", endDate: "2024-03-01" },
  );
});

test("Shanghai date extraction stays on the local day near the UTC boundary", () => {
  const shanghaiNewYear = new Date("2026-01-01T00:05:00+08:00");
  assert.deepEqual(getStockReviewDateRange("YTD", shanghaiNewYear), {
    startDate: "2026-01-01",
    endDate: "2026-01-01",
  });
});

test("missing, corrupt, or unknown filter data falls back to the complete default", () => {
  const expected = createDefaultStockReviewFilters(now, "CNY");
  const invalidValues = [
    undefined,
    "not json",
    JSON.stringify({ periodPreset: "LAST_30_DAYS" }),
    JSON.stringify({
      accountId: null,
      periodPreset: "YTD",
      startDate: "2026-01-01",
      endDate: "2026-08-28",
      market: "EU",
      benchmarkSymbol: null,
    }),
  ];

  for (const value of invalidValues) {
    const storage = memoryStorage(
      value === undefined ? {} : { [STOCK_REVIEW_FILTERS_STORAGE_KEY]: value },
    );
    assert.deepEqual(loadStockReviewFilters(storage, now, "CNY"), expected);
  }

  assert.deepEqual(expected, {
    accountId: null,
    periodPreset: "YTD",
    startDate: "2026-01-01",
    endDate: "2026-08-28",
    market: null,
    benchmarkSymbol: null,
    baseCurrency: "CNY",
  });
});

test("valid persisted filters are restored while the current app base currency wins", () => {
  const storage = memoryStorage({
    [STOCK_REVIEW_FILTERS_STORAGE_KEY]: JSON.stringify({
      accountId: "account-a",
      periodPreset: "CUSTOM",
      startDate: "2024-02-29",
      endDate: "2024-12-31",
      market: "US",
      benchmarkSymbol: "QQQ",
      baseCurrency: "USD",
    }),
  });

  assert.deepEqual(loadStockReviewFilters(storage, now, "HKD"), {
    accountId: "account-a",
    periodPreset: "CUSTOM",
    startDate: "2024-02-29",
    endDate: "2024-12-31",
    market: "US",
    benchmarkSymbol: "QQQ",
    baseCurrency: "HKD",
  });
});

test("persisted v1 validates every field before ignoring preset-derived dates", () => {
  const valid = {
    accountId: "account-a",
    periodPreset: "YTD",
    startDate: "2026-01-01",
    endDate: "2026-08-28",
    market: "US",
    benchmarkSymbol: "QQQ",
    baseCurrency: "USD",
  };
  const corrupt = [
    { ...valid, baseCurrency: "EUR" },
    { ...valid, startDate: "2026-02-30" },
    { ...valid, startDate: "2026-09-01", endDate: "2026-08-28" },
    { ...valid, accountId: 42 },
    { ...valid, benchmarkSymbol: false },
    { ...valid, market: "EU" },
    { ...valid, unknownFutureField: "value" },
  ];
  const expected = createDefaultStockReviewFilters(now, "HKD");

  for (const value of corrupt) {
    const storage = memoryStorage({
      [STOCK_REVIEW_FILTERS_STORAGE_KEY]: JSON.stringify(value),
    });
    assert.deepEqual(loadStockReviewFilters(storage, now, "HKD"), expected);
  }
});

test("custom ranges reject impossible or reversed dates before persistence", () => {
  assert.throws(
    () =>
      getStockReviewDateRange("CUSTOM", now, {
        startDate: "2026-02-30",
        endDate: "2026-03-01",
      }),
    /有效日期/,
  );
  assert.throws(
    () =>
      getStockReviewDateRange("CUSTOM", now, {
        startDate: "2026-09-01",
        endDate: "2026-08-28",
      }),
    /开始日期/,
  );

  const storage = memoryStorage();
  assert.throws(
    () =>
      saveStockReviewFilters(storage, {
        accountId: null,
        periodPreset: "CUSTOM",
        startDate: "2026-09-01",
        endDate: "2026-08-28",
        market: null,
        benchmarkSymbol: null,
        baseCurrency: "USD",
      }),
    /开始日期/,
  );
  assert.equal(storage.getItem(STOCK_REVIEW_FILTERS_STORAGE_KEY), null);
});

test("portfolio AI prefill activates stock-review with executable filters and never sends", () => {
  const filters = {
    accountId: null,
    periodPreset: "YTD",
    startDate: "2026-01-01",
    endDate: "2026-08-28",
    market: "US",
    benchmarkSymbol: null,
    baseCurrency: "USD",
  };

  assert.deepEqual(buildStockReviewAiPrefill(filters), {
    activeSkill: "stock-review",
    prompt:
      "请基于本期确定性股票复盘报告，分析整体调仓是否创造价值、收益是否依赖少数操作、风险结构是否改善，以及最值得进一步复盘的三项操作。请严格区分确定性事实、事后结果和缺失的决策背景。",
    toolName: "get_stock_review",
    toolArguments: {
      start_date: "2026-01-01",
      end_date: "2026-08-28",
      base_currency: "USD",
      market: "US",
    },
    autoSend: false,
  });
});

test("Campaign AI prefill adds normalized symbol and Campaign identity without sending", () => {
  const filters = {
    accountId: "account-a",
    periodPreset: "CUSTOM",
    startDate: "2026-04-01",
    endDate: "2026-06-30",
    market: "US",
    benchmarkSymbol: "SPY",
    baseCurrency: "CNY",
  };

  assert.deepEqual(buildStockCampaignAiPrefill(filters, "  aapl ", " campaign-7 "), {
    activeSkill: "stock-review",
    prompt:
      "请复盘当前股票Campaign，区分确定性事实、事后推断和缺失背景，重点分析加减仓节奏、仓位变化及其对组合的贡献。",
    toolName: "get_stock_review",
    toolArguments: {
      start_date: "2026-04-01",
      end_date: "2026-06-30",
      base_currency: "CNY",
      account_id: "account-a",
      market: "US",
      benchmark_symbol: "SPY",
      symbol: "AAPL",
      campaign_id: "campaign-7",
    },
    autoSend: false,
  });
});

test("display mapping preserves backend status and never fills a missing value with zero", () => {
  assert.deepEqual(
    mapStockReviewMetricForDisplay(null, {
      status: "unavailable",
      note: "缺少行情",
    }),
    {
      value: null,
      status: "unavailable",
      note: "缺少行情",
      displayValue: "—",
    },
  );
  assert.deepEqual(
    mapStockReviewMetricForDisplay(0, { status: "available", note: null }),
    {
      value: 0,
      status: "available",
      note: null,
      displayValue: "0",
    },
  );
  assert.deepEqual(
    mapStockReviewMetricForDisplay(Number.NaN, {
      status: "degraded",
      note: "无效展示值",
    }),
    {
      value: null,
      status: "degraded",
      note: "无效展示值",
      displayValue: "—",
    },
  );
});

const displayContext = createStockReviewAnnotationDisplayContext({
  endDate: "2026-08-28",
  accountId: "account-a",
  actions: [
    { actionId: "action-a", accountId: "account-a", symbol: "AAPL" },
    { actionId: "action-msft", accountId: "account-a", symbol: "MSFT" },
  ],
  campaigns: [
    {
      campaignId: "campaign-old",
      accountIds: ["account-a"],
      actionIds: ["action-old"],
      symbol: "AAPL",
      startedAt: "2025-01-01T09:30:00Z",
      endedAt: "2025-06-30T16:00:00Z",
    },
    {
      campaignId: "campaign-current",
      accountIds: ["account-a"],
      actionIds: ["action-a"],
      symbol: "AAPL",
      startedAt: "2026-01-01T09:30:00Z",
      endedAt: null,
    },
  ],
});

function annotation(overrides: Record<string, unknown> = {}) {
  return {
    id: "annotation",
    scope_type: "period",
    scope_key: "2026-01-01:2026-08-28",
    account_id: null,
    symbol: null,
    annotation_type: "note",
    value_json: "{}",
    source: "user",
    created_at: "2026-08-29T00:00:00Z",
    updated_at: "2026-08-29T00:00:00Z",
    ...overrides,
  };
}

test("display context uses exact scoped account and as-of rules without period-array membership", () => {
  const invisible = [
    annotation(),
    annotation({ account_id: "account-b" }),
    annotation({ value_json: '{"effective_date":"2026-08-29"}' }),
    annotation({ value_json: '{"effective_start":"2026-09-01"}' }),
    annotation({ value_json: '{"effective_date":"2026-02-30"}' }),
  ];

  for (const item of invisible) {
    assert.equal(isStockReviewAnnotationInDisplayContext(item, displayContext), false);
  }
  const visible = [
    annotation({ account_id: "account-a" }),
    annotation({
      scope_type: "action",
      scope_key: "action-outside-filtered-report",
      account_id: "account-a",
      symbol: "NVDA",
    }),
    annotation({
      scope_type: "campaign",
      scope_key: "campaign-outside-filtered-report",
      account_id: "account-a",
      symbol: "NVDA",
    }),
    annotation({
      scope_type: "stock",
      scope_key: "NVDA",
      symbol: "NVDA",
      account_id: "account-a",
      value_json: '{"effective_date":"2026-02-01"}',
    }),
  ];

  for (const item of visible) {
    assert.equal(isStockReviewAnnotationInDisplayContext(item, displayContext), true);
  }
});

test("all-account display context includes global and every account like the backend query", () => {
  const allAccounts = createStockReviewAnnotationDisplayContext({
    ...displayContext,
    accountId: null,
  });
  assert.equal(isStockReviewAnnotationInDisplayContext(annotation(), allAccounts), true);
  assert.equal(
    isStockReviewAnnotationInDisplayContext(
      annotation({ account_id: "account-b" }),
      allAccounts,
    ),
    true,
  );
});

test("Campaign applicability excludes period and matches exact Campaign and action scopes", () => {
  assert.equal(
    doesStockReviewAnnotationApplyToCampaign(annotation(), displayContext, "campaign-current"),
    false,
  );
  assert.equal(
    doesStockReviewAnnotationApplyToCampaign(
      annotation({ scope_type: "action", scope_key: "action-a" }),
      displayContext,
      "campaign-current",
    ),
    true,
  );
  assert.equal(
    doesStockReviewAnnotationApplyToCampaign(
      annotation({ scope_type: "action", scope_key: "action-old" }),
      displayContext,
      "campaign-current",
    ),
    false,
  );
  assert.equal(
    doesStockReviewAnnotationApplyToCampaign(
      annotation({ scope_type: "campaign", scope_key: "campaign-current" }),
      displayContext,
      "campaign-current",
    ),
    true,
  );
  assert.equal(
    doesStockReviewAnnotationApplyToCampaign(
      annotation({ scope_type: "campaign", scope_key: "campaign-old" }),
      displayContext,
      "campaign-current",
    ),
    false,
  );
  assert.equal(
    doesStockReviewAnnotationApplyToCampaign(
      annotation({
        scope_type: "campaign",
        scope_key: "campaign-current",
        account_id: "account-b",
      }),
      displayContext,
      "campaign-current",
    ),
    false,
  );
});

test("Campaign stock applicability respects lifetime and prevents same-symbol cycle leakage", () => {
  assert.equal(
    doesStockReviewAnnotationApplyToCampaign(
      annotation({
        scope_type: "stock",
        scope_key: "AAPL",
        symbol: "AAPL",
        account_id: "account-a",
      }),
      displayContext,
      "campaign-current",
    ),
    false,
  );
  assert.equal(
    doesStockReviewAnnotationApplyToCampaign(
      annotation({
        scope_type: "stock",
        scope_key: "AAPL",
        symbol: "AAPL",
        account_id: "account-a",
        value_json: '{"effective_start":"2026-02-01","effective_end":"2026-03-01"}',
      }),
      displayContext,
      "campaign-current",
    ),
    true,
  );
  assert.equal(
    doesStockReviewAnnotationApplyToCampaign(
      annotation({
        scope_type: "stock",
        scope_key: "AAPL",
        symbol: "AAPL",
        account_id: "account-a",
        value_json: '{"effective_date":"2025-03-01"}',
      }),
      displayContext,
      "campaign-current",
    ),
    false,
  );
  assert.equal(
    doesStockReviewAnnotationApplyToCampaign(
      annotation({
        scope_type: "stock",
        scope_key: "AAPL",
        symbol: "AAPL",
        account_id: "account-a",
        value_json: '{"effective_date":"2026-09-01"}',
      }),
      displayContext,
      "campaign-current",
    ),
    false,
  );

  const futureCampaign = createStockReviewAnnotationDisplayContext({
    ...displayContext,
    campaigns: [
      {
        ...displayContext.campaigns[1],
        campaignId: "campaign-future",
        startedAt: "2026-09-01T09:30:00Z",
      },
    ],
  });
  assert.equal(
    doesStockReviewAnnotationApplyToCampaign(
      annotation({
        scope_type: "stock",
        scope_key: "AAPL",
        symbol: "AAPL",
        account_id: "account-a",
      }),
      futureCampaign,
      "campaign-future",
    ),
    false,
  );
});
