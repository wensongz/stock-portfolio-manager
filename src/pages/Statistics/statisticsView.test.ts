// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { resolveStatisticsView } from "./statisticsView.ts";

const selection = {
  activeTab: "overview",
  baseCurrency: "USD",
  selectedMarket: "US",
  selectedAccountId: "acct-us",
  selectedCategoryId: "growth",
};

test("overview and market always resolve one current view", () => {
  assert.deepEqual(resolveStatisticsView(selection), {
    kind: "overview",
    baseCurrency: "USD",
  });
  assert.deepEqual(
    resolveStatisticsView({
      ...selection,
      activeTab: "market",
      baseCurrency: "CNY",
    }),
    { kind: "market", market: "US" },
  );
});

test("account and category require a selected id", () => {
  assert.equal(
    resolveStatisticsView({
      ...selection,
      activeTab: "account",
      selectedAccountId: "",
    }),
    null,
  );
  assert.deepEqual(
    resolveStatisticsView({ ...selection, activeTab: "account" }),
    { kind: "account", accountId: "acct-us" },
  );
  assert.equal(
    resolveStatisticsView({
      ...selection,
      activeTab: "category",
      selectedCategoryId: "",
    }),
    null,
  );
  assert.deepEqual(
    resolveStatisticsView({ ...selection, activeTab: "category" }),
    { kind: "category", categoryId: "growth", baseCurrency: "USD" },
  );
});

test("currency affects only overview and category views", () => {
  assert.deepEqual(
    resolveStatisticsView({ ...selection, baseCurrency: "HKD" }),
    { kind: "overview", baseCurrency: "HKD" },
  );
  assert.deepEqual(
    resolveStatisticsView({
      ...selection,
      activeTab: "category",
      baseCurrency: "HKD",
    }),
    { kind: "category", categoryId: "growth", baseCurrency: "HKD" },
  );
  assert.deepEqual(
    resolveStatisticsView({
      ...selection,
      activeTab: "market",
      baseCurrency: "HKD",
    }),
    { kind: "market", market: "US" },
  );
  assert.deepEqual(
    resolveStatisticsView({
      ...selection,
      activeTab: "account",
      baseCurrency: "HKD",
    }),
    { kind: "account", accountId: "acct-us" },
  );
});
