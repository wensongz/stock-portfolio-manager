// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";

const contract = (overrides: Record<string, unknown>) => ({
  id: "contract-id",
  option_symbol: "AAPL 18SEP26 200 P",
  underlying: "AAPL",
  expiry_date: "18SEP26",
  strike_price: 200,
  option_type: "P",
  contracts: -1,
  open_price: 2,
  open_amount: 200,
  commission: -1,
  traded_at: "2026-08-01",
  close_price: null,
  close_code: null,
  status: "active",
  account_id: "account-id",
  ...overrides,
});

const campaign = (overrides: Record<string, unknown>) => ({
  id: "option-review:account-id:AAPL:contract-id",
  underlying: "AAPL",
  option_symbol: "AAPL 18SEP26 200 P",
  expiry_date: "2026-09-18",
  contracts: -1,
  started_at: "2026-08-01",
  ended_at: null,
  status: "active",
  inferred: true,
  strategy_path: ["CSP"],
  gross_premium: 100,
  close_cost: 0,
  fees: 5,
  net_premium_pnl: 95,
  secured_notional: 20_000,
  capital_days: 660_000,
  retention_rate: null,
  annualized_yield_on_notional: null,
  ...overrides,
});

const review = (overrides: Record<string, unknown>) => ({
  underlying: "AAPL",
  completed_campaigns: 1,
  active_campaigns: 2,
  gross_premium: 1_155,
  net_premium_pnl: 1_145,
  completed_gross_premium: 1_000,
  completed_net_premium_pnl: 1_000,
  retention_rate: 1,
  annualized_yield_on_notional: null,
  worst_campaign_pnl: null,
  flags: ["有进行中仓位"],
  campaigns: [
    campaign({}),
    campaign({
      id: "option-review:account-id:AAPL:contract-call",
      option_symbol: "AAPL 16OCT26 220 C",
      expiry_date: "2026-10-16",
      contracts: -2,
      strategy_path: ["Covered Call"],
      gross_premium: 55,
      fees: 5,
      net_premium_pnl: 50,
    }),
    campaign({
      id: "option-review:account-id:AAPL:completed",
      option_symbol: "AAPL 21AUG26 190 P",
      expiry_date: "2026-08-21",
      ended_at: "2026-08-21",
      status: "completed",
      gross_premium: 1_000,
      fees: 0,
      net_premium_pnl: 1_000,
      retention_rate: 1,
      annualized_yield_on_notional: 0.2,
    }),
  ],
  ...overrides,
});

test("active summaries preserve signed Put and Call contract totals", async () => {
  const { buildActiveUnderlyingSummaries } = await import(
    "./activeOptionsViewModel.ts"
  );

  const [row] = buildActiveUnderlyingSummaries(
    [
      contract({ id: "put-1", contracts: -2 }),
      contract({ id: "put-2", contracts: -3 }),
      contract({ id: "call", option_type: "C", contracts: -4 }),
    ],
    [],
    "2026-09-03",
  );

  assert.equal(row.putContracts, -5);
  assert.equal(row.callContracts, -4);
});

test("active summaries aggregate active-only net premium and expiry risk by underlying", async () => {
  const { buildActiveUnderlyingSummaries } = await import(
    "./activeOptionsViewModel.ts"
  );

  const rows = buildActiveUnderlyingSummaries(
    [
      contract({}),
      contract({
        id: "contract-call",
        option_symbol: "AAPL 16OCT26 220 C",
        expiry_date: "16OCT26",
        option_type: "C",
        contracts: -2,
      }),
    ],
    [review({})],
    "2026-09-03",
  );

  assert.deepEqual(rows, [
    {
      underlying: "AAPL",
      netPremium: 145,
      totalRecords: 2,
      putContracts: -1,
      callContracts: -2,
      averageNetPremiumPerRecord: 72.5,
      nextExpiryDate: "2026-09-18",
      expiringWithin30Days: 1,
    },
  ]);
});

test("active summaries sort by net premium descending with missing values last", async () => {
  const { buildActiveUnderlyingSummaries } = await import(
    "./activeOptionsViewModel.ts"
  );

  const rows = buildActiveUnderlyingSummaries(
    [
      contract({ id: "unknown", underlying: "ZZZ", expiry_date: "unknown" }),
      contract({ id: "loss", underlying: "AAPL", expiry_date: "18SEP26" }),
      contract({ id: "tie-m", underlying: "MSFT", expiry_date: "20NOV26" }),
      contract({ id: "tie-b", underlying: "BABA", expiry_date: "16OCT26" }),
    ],
    [
      review({
        underlying: "AAPL",
        active_campaigns: 1,
        campaigns: [
          campaign({
            id: "option-review:account-id:AAPL:loss",
            underlying: "AAPL",
            net_premium_pnl: -50,
          }),
        ],
      }),
      review({
        underlying: "MSFT",
        active_campaigns: 1,
        campaigns: [
          campaign({
            id: "option-review:account-id:MSFT:tie-m",
            underlying: "MSFT",
            net_premium_pnl: 500,
          }),
        ],
      }),
      review({
        underlying: "BABA",
        active_campaigns: 1,
        campaigns: [
          campaign({
            id: "option-review:account-id:BABA:tie-b",
            underlying: "BABA",
            net_premium_pnl: 500,
          }),
        ],
      }),
    ],
    "2026-09-03",
  );

  assert.deepEqual(
    rows.map((row) => row.underlying),
    ["BABA", "MSFT", "AAPL", "ZZZ"],
  );
});

test("active selection preserves a valid symbol and otherwise uses the first summary row", async () => {
  const { resolveActiveUnderlyingSelection } = await import(
    "./activeOptionsViewModel.ts"
  );
  const rows = [{ underlying: "AAPL" }, { underlying: "MSFT" }];

  assert.equal(resolveActiveUnderlyingSelection(rows, "MSFT"), "MSFT");
  assert.equal(resolveActiveUnderlyingSelection(rows, "MSFT", false), "AAPL");
  assert.equal(resolveActiveUnderlyingSelection(rows, "NVDA"), "AAPL");
  assert.equal(resolveActiveUnderlyingSelection([], "AAPL"), null);
});

test("active summaries suppress financial metrics when review coverage is incomplete", async () => {
  const { buildActiveUnderlyingSummaries } = await import(
    "./activeOptionsViewModel.ts"
  );

  const withoutReview = buildActiveUnderlyingSummaries(
    [contract({})],
    [],
    "2026-09-03",
  );
  const partialReview = buildActiveUnderlyingSummaries(
    [contract({}), contract({ id: "second", expiry_date: "16OCT26" })],
    [review({ active_campaigns: 1, campaigns: [campaign({})] })],
    "2026-09-03",
  );

  assert.equal(withoutReview[0].netPremium, null);
  assert.equal(withoutReview[0].averageNetPremiumPerRecord, null);
  assert.equal(partialReview[0].netPremium, null);
  assert.equal(partialReview[0].averageNetPremiumPerRecord, null);
});

test("active summaries reject same-count review data from different opening records", async () => {
  const { buildActiveUnderlyingSummaries } = await import(
    "./activeOptionsViewModel.ts"
  );

  const rows = buildActiveUnderlyingSummaries(
    [contract({ id: "current-open" })],
    [
      review({
        active_campaigns: 1,
        campaigns: [
          campaign({
            id: "option-review:account-id:AAPL:different-open",
            net_premium_pnl: 998,
          }),
        ],
      }),
    ],
    "2026-09-03",
  );

  assert.equal(rows[0].netPremium, null);
  assert.equal(rows[0].averageNetPremiumPerRecord, null);
});

test("active expiry risk accepts supported formats, rejects impossible dates, and includes day 30", async () => {
  const { buildActiveUnderlyingSummaries } = await import(
    "./activeOptionsViewModel.ts"
  );

  const rows = buildActiveUnderlyingSummaries(
    [
      contract({ id: "today", underlying: "AAPL", expiry_date: "2026/09/03" }),
      contract({ id: "day-30", underlying: "MSFT", expiry_date: "2026/10/03" }),
      contract({ id: "day-31", underlying: "NVDA", expiry_date: "04OCT26" }),
      contract({ id: "mixed", underlying: "MIX", expiry_date: "2026/09-18" }),
      contract({ id: "invalid", underlying: "ZZZ", expiry_date: "31FEB26" }),
    ],
    [],
    "2026-09-03",
  );

  assert.deepEqual(
    rows
      .map(({ underlying, nextExpiryDate, expiringWithin30Days }) => ({
        underlying,
        nextExpiryDate,
        expiringWithin30Days,
      }))
      .sort((left, right) => left.underlying.localeCompare(right.underlying)),
    [
      {
        underlying: "AAPL",
        nextExpiryDate: "2026-09-03",
        expiringWithin30Days: 1,
      },
      {
        underlying: "MIX",
        nextExpiryDate: null,
        expiringWithin30Days: 0,
      },
      {
        underlying: "MSFT",
        nextExpiryDate: "2026-10-03",
        expiringWithin30Days: 1,
      },
      {
        underlying: "NVDA",
        nextExpiryDate: "2026-10-04",
        expiringWithin30Days: 0,
      },
      {
        underlying: "ZZZ",
        nextExpiryDate: null,
        expiringWithin30Days: 0,
      },
    ],
  );
});
