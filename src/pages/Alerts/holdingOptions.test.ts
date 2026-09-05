// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";

const holding = (overrides: Record<string, unknown>) => ({
  id: "holding-a",
  account_id: "account-a",
  symbol: "AAPL",
  name: "Apple",
  market: "US",
  category_id: null,
  shares: 10,
  avg_cost: 150,
  currency: "USD",
  created_at: "2026-09-01T00:00:00Z",
  updated_at: "2026-09-01T00:00:00Z",
  ...overrides,
});

test("alert holding options deduplicate market-symbol pairs across accounts", async () => {
  const { buildAlertHoldingOptions } = await import("./holdingOptions.ts");

  const options = buildAlertHoldingOptions([
    holding({}),
    holding({ id: "holding-b", account_id: "account-b" }),
    holding({
      id: "holding-hk",
      account_id: "account-hk",
      market: "HK",
      currency: "HKD",
    }),
  ]);

  assert.deepEqual(options, [
    { value: "holding-a", label: "AAPL Apple (US)" },
    { value: "holding-hk", label: "AAPL Apple (HK)" },
  ]);
});

test("alert holding options normalize symbol casing and whitespace for deduplication", async () => {
  const { buildAlertHoldingOptions } = await import("./holdingOptions.ts");

  const options = buildAlertHoldingOptions([
    holding({}),
    holding({
      id: "holding-b",
      account_id: "account-b",
      symbol: " aapl ",
    }),
  ]);

  assert.deepEqual(options, [
    { value: "holding-a", label: "AAPL Apple (US)" },
  ]);
});
