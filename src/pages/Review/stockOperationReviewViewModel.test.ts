// @ts-nocheck -- Runs directly in Node 26 without browser-focused Node typings.
import test from "node:test";
import assert from "node:assert/strict";
import {
  STOCK_OPERATION_REVIEW_FILTERS_STORAGE_KEY,
  buildStockOperationIdentityDisplay,
  buildStockOperationReviewAiPrefill,
  buildStockOperationReviewQualityText,
  buildStockOperationSummaryCards,
  createDefaultStockOperationReviewFilters,
  formatOperationCurrency,
  formatOperationPercent,
  formatStockOperationIdentity,
  formatOperationWeight,
  getStockOperationReviewDateRange,
  loadStockOperationReviewFilters,
  saveStockOperationReviewFilters,
  sortStockOperationSecurities,
} from "./stockOperationReviewViewModel.ts";

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

const now = new Date("2026-08-30T08:00:00+08:00");

test("lightweight date presets use calendar boundaries", () => {
  assert.deepEqual(getStockOperationReviewDateRange("QTD", now), {
    startDate: "2026-07-01",
    endDate: "2026-08-30",
  });
  assert.deepEqual(getStockOperationReviewDateRange("PREV_QUARTER", now), {
    startDate: "2026-04-01",
    endDate: "2026-06-30",
  });
  assert.deepEqual(getStockOperationReviewDateRange("YTD", now), {
    startDate: "2026-01-01",
    endDate: "2026-08-30",
  });
  assert.deepEqual(getStockOperationReviewDateRange("1Y", now), {
    startDate: "2025-08-31",
    endDate: "2026-08-30",
  });
});

test("one-year preset preserves leap-day calendar semantics", () => {
  assert.deepEqual(
    getStockOperationReviewDateRange("1Y", new Date("2024-02-29T23:30:00+08:00")),
    { startDate: "2023-03-01", endDate: "2024-02-29" },
  );
});

test("filters migrate old benchmark data but save only lightweight fields", () => {
  const old = {
    accountId: "account-a",
    periodPreset: "CUSTOM",
    startDate: "2026-07-01",
    endDate: "2026-08-30",
    market: "CN",
    benchmarkSymbol: "000300.SS",
    baseCurrency: "CNY",
  };
  const storage = memoryStorage({ review_stock_filters_v1: JSON.stringify(old) });
  assert.deepEqual(loadStockOperationReviewFilters(storage, now, "USD"), {
    accountId: "account-a",
    periodPreset: "CUSTOM",
    startDate: "2026-07-01",
    endDate: "2026-08-30",
    market: "CN",
    baseCurrency: "CNY",
  });

  saveStockOperationReviewFilters(storage, {
    accountId: null,
    periodPreset: "YTD",
    startDate: "2026-01-01",
    endDate: "2026-08-30",
    market: null,
    baseCurrency: "USD",
  });
  const saved = JSON.parse(storage.getItem(STOCK_OPERATION_REVIEW_FILTERS_STORAGE_KEY)!);
  assert.deepEqual(Object.keys(saved).sort(), [
    "accountId",
    "baseCurrency",
    "endDate",
    "market",
    "periodPreset",
    "startDate",
  ]);
  assert.equal(saved.benchmarkSymbol, undefined);
});

test("default filters are YTD and use the supplied base currency", () => {
  assert.deepEqual(createDefaultStockOperationReviewFilters(now, "HKD"), {
    accountId: null,
    periodPreset: "YTD",
    startDate: "2026-01-01",
    endDate: "2026-08-30",
    market: null,
    baseCurrency: "HKD",
  });
});

test("formatters distinguish missing values from zero", () => {
  assert.equal(formatOperationCurrency(null, "USD"), "—");
  assert.equal(formatOperationCurrency(0, "USD"), "$0.00");
  assert.equal(formatOperationPercent(null), "—");
  assert.equal(formatOperationPercent(-0.125), "-12.50%");
  assert.equal(formatOperationWeight(null), "—");
  assert.equal(formatOperationWeight(0.0345), "3.45%");
});

const group = (overrides = {}) => ({
  action_count: 2,
  positive_count: 1,
  negative_count: 1,
  missing_effect_count: 0,
  price_effect_base: 123.45,
  positive_notional_ratio: 0.6,
  weighted_excess_return: 0.02,
  ...overrides,
});

test("four summary cards project independent lightweight metrics", () => {
  const cards = buildStockOperationSummaryCards({
    total: group(),
    buys: group({ price_effect_base: 100 }),
    sells: group({ price_effect_base: 23.45 }),
    position_impact: {
      invested_amount_base: 1000,
      recovered_amount_base: 500,
      largest_absolute_weight_change: 0.08,
      total_fees_base: 3,
      missing_weight_count: 1,
    },
  }, "USD");
  assert.deepEqual(cards.map((card) => card.title), [
    "操作总效果",
    "买入与加仓",
    "减仓与清仓",
    "仓位影响",
  ]);
  assert.match(cards[0].primary, /123\.45/);
  assert.match(cards[2].description, /避损或机会损失/);
  assert.match(cards[3].description, /缺少权重 1 项/);

  const missing = buildStockOperationSummaryCards({
    total: group({ price_effect_base: null }),
    buys: group({ price_effect_base: null }),
    sells: group({ price_effect_base: null }),
    position_impact: {
      invested_amount_base: null,
      recovered_amount_base: null,
      largest_absolute_weight_change: null,
      total_fees_base: null,
      missing_weight_count: 2,
    },
  }, "USD");
  assert.equal(missing[0].primary, "—");
  assert.doesNotMatch(JSON.stringify(missing), /不可用|降级/);
});

test("quality summary is neutral and field-specific", () => {
  assert.equal(
    buildStockOperationReviewQualityText({
      action_count: 12,
      missing_end_price_count: 1,
      missing_benchmark_count: 2,
      missing_fx_count: 0,
      missing_weight_count: 3,
      notes: [],
    }),
    "共分析 12 项操作；1 项缺少期末价，2 项缺少基准，3 项缺少权重估算。",
  );
});

test("security ranking keeps unavailable effects last for every descending sort", () => {
  const rows = [
    { symbol: "NONE", price_effect_base: null, buy_notional_local: 9999, sell_notional_local: 0, weighted_excess_return: null, largest_absolute_weight_change: null },
    { symbol: "LOW", price_effect_base: -10, buy_notional_local: 100, sell_notional_local: 0, weighted_excess_return: -0.01, largest_absolute_weight_change: 0.02 },
    { symbol: "HIGH", price_effect_base: 20, buy_notional_local: 200, sell_notional_local: 0, weighted_excess_return: 0.03, largest_absolute_weight_change: 0.05 },
  ];
  assert.deepEqual(
    sortStockOperationSecurities(rows, "effect").map((row) => row.symbol),
    ["HIGH", "LOW", "NONE"],
  );
  assert.deepEqual(
    sortStockOperationSecurities(rows, "benchmark").map((row) => row.symbol),
    ["HIGH", "LOW", "NONE"],
  );
});

test("table identities never show market and hide account for a single-account report", () => {
  assert.deepEqual(
    buildStockOperationIdentityDisplay(null, "CN", "平安证券A"),
    {
      columnTitle: "股票 / 账户",
      securitySecondary: "平安证券A",
      actionSecondary: "平安证券A",
    },
  );
  assert.deepEqual(
    buildStockOperationIdentityDisplay("account-a", "CN", "平安证券A"),
    {
      columnTitle: "股票",
      securitySecondary: null,
      actionSecondary: null,
    },
  );
});

test("stock identity displays symbol before name without duplicating a missing name", () => {
  assert.equal(formatStockOperationIdentity("sz001248", "华菱线缆"), "sz001248 · 华菱线缆");
  assert.equal(formatStockOperationIdentity("sh511880", ""), "sh511880");
  assert.equal(formatStockOperationIdentity("AAPL", "   "), "AAPL");
});

test("AI prefill carries only the lightweight deterministic scope", () => {
  const prefill = buildStockOperationReviewAiPrefill({
    accountId: "account-a",
    periodPreset: "CUSTOM",
    startDate: "2026-07-01",
    endDate: "2026-08-30",
    market: "CN",
    baseCurrency: "CNY",
  });
  assert.equal(prefill.toolName, "get_stock_review");
  assert.deepEqual(prefill.toolArguments, {
    start_date: "2026-07-01",
    end_date: "2026-08-30",
    base_currency: "CNY",
    account_id: "account-a",
    market: "CN",
  });
  assert.equal(prefill.autoSend, false);
  assert.doesNotMatch(JSON.stringify(prefill), /benchmark_symbol|campaign_id/);
});
