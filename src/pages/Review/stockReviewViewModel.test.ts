// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  STOCK_REVIEW_FILTERS_STORAGE_KEY,
  buildStockCampaignAiPrefill,
  buildStockReviewAiPrefill,
  createDefaultStockReviewFilters,
  getStockReviewDateRange,
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
