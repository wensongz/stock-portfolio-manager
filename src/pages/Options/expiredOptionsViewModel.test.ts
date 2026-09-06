// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  buildExpiredOptionStats,
  buildExpiredUnderlyingSummaries,
  getOptionContractRowKey,
  isAllHistoryOptionReview,
  isAllHistoryOptionReviewRequest,
  resolveCurrentOptionAccount,
  resolveExpiredUnderlyingSelection,
  selectAccountOptionContracts,
} from "./expiredOptionsViewModel.ts";

const contract = (overrides: Record<string, unknown>) => ({
  id: "contract-id",
  option_symbol: "AAPL 18SEP26 200 P",
  underlying: "AAPL",
  expiry_date: "2026-09-18",
  strike_price: 200,
  option_type: "P",
  contracts: 1,
  remaining_contracts: 0,
  open_price: 2,
  open_amount: 200,
  commission: 1,
  traded_at: "2026-08-01",
  close_price: 0,
  close_code: "C;Ep",
  status: "expired",
  account_id: "account-id",
  ...overrides,
});

const review = (overrides: Record<string, unknown>) => ({
  underlying: "AAPL",
  completed_campaigns: 3,
  active_campaigns: 0,
  gross_premium: 900,
  net_premium_pnl: 600,
  completed_gross_premium: 900,
  completed_net_premium_pnl: 600,
  retention_rate: 0.67,
  annualized_yield_on_notional: 0.1,
  worst_campaign_pnl: -50,
  flags: [],
  campaigns: [
    { id: "older", status: "completed", ended_at: "2026-08-22", contracts: 2 },
    { id: "latest", status: "completed", ended_at: "2026-09-19", contracts: 4 },
    { id: "active", status: "active", ended_at: null, contracts: 1 },
  ],
  ...overrides,
});

test("expired summaries count records and preserve signed Put/Call quantities", () => {
  const rows = buildExpiredUnderlyingSummaries(
    [
      contract({ id: "assigned", contracts: -2, status: "assigned", option_type: "P" }),
      contract({ id: "expired", contracts: 3, status: "expired", option_type: "C" }),
      contract({ id: "closed", contracts: -1, status: "closed", option_type: "P" }),
    ],
    [review({})],
  );

  assert.deepEqual(rows[0], {
    underlying: "AAPL",
    netPremium: 600,
    totalRecords: 3,
    assignedRecords: 1,
    expiredRecords: 1,
    putQuantity: -3,
    callQuantity: 3,
    assignmentRatio: 1 / 3,
    averageNetPremiumPerRecord: 200,
    latestCompletedAt: "2026-09-19",
  });
});

test("expired summaries keep an underlying when review data is unavailable", () => {
  const rows = buildExpiredUnderlyingSummaries(
    [contract({ underlying: "MSFT", contracts: 2 })],
    [],
  );

  assert.equal(rows[0].underlying, "MSFT");
  assert.equal(rows[0].netPremium, null);
  assert.equal(rows[0].averageNetPremiumPerRecord, null);
  assert.equal(rows[0].latestCompletedAt, null);
});

test("expired summaries suppress financial metrics when review coverage is partial", () => {
  const rows = buildExpiredUnderlyingSummaries(
    [contract({ contracts: 2 })],
    [
      review({
        completed_net_premium_pnl: 100,
        campaigns: [
          { id: "reviewed", status: "completed", ended_at: "2026-09-19", contracts: 1 },
        ],
      }),
    ],
  );

  assert.equal(rows[0].netPremium, null);
  assert.equal(rows[0].averageNetPremiumPerRecord, null);
  assert.equal(rows[0].latestCompletedAt, null);
});

test("expired summaries sort by absolute net premium and then symbol", () => {
  const rows = buildExpiredUnderlyingSummaries(
    [
      contract({ id: "msft", underlying: "MSFT" }),
      contract({ id: "aapl", underlying: "AAPL" }),
      contract({ id: "nvda", underlying: "NVDA" }),
    ],
    [
      review({
        underlying: "MSFT",
        completed_net_premium_pnl: 500,
        campaigns: [{ status: "completed", ended_at: "2026-09-19", contracts: 1 }],
      }),
      review({
        underlying: "AAPL",
        completed_net_premium_pnl: -900,
        campaigns: [{ status: "completed", ended_at: "2026-09-19", contracts: 1 }],
      }),
      review({
        underlying: "NVDA",
        completed_net_premium_pnl: 500,
        campaigns: [{ status: "completed", ended_at: "2026-09-19", contracts: 1 }],
      }),
    ],
  );

  assert.deepEqual(rows.map((row) => row.underlying), ["AAPL", "MSFT", "NVDA"]);
});

test("expired selection preserves a valid symbol and otherwise uses the first row", () => {
  const rows = [
    { underlying: "AAPL" },
    { underlying: "MSFT" },
  ];

  assert.equal(resolveExpiredUnderlyingSelection(rows, "MSFT"), "MSFT");
  assert.equal(resolveExpiredUnderlyingSelection(rows, "MSFT", false), "AAPL");
  assert.equal(resolveExpiredUnderlyingSelection(rows, "NVDA"), "AAPL");
  assert.equal(resolveExpiredUnderlyingSelection([], "AAPL"), null);
});

test("account contract selection excludes stale contracts during account switches", () => {
  const contracts = [
    contract({ id: "old", account_id: "old-account", underlying: "AAPL" }),
    contract({ id: "current", account_id: "current-account", underlying: "MSFT" }),
  ];

  const selected = selectAccountOptionContracts(contracts, "current-account");

  assert.deepEqual(selected.map((item) => item.id), ["current"]);
});

test("all-history review matching rejects finite periods and other accounts", () => {
  assert.equal(
    isAllHistoryOptionReview(
      { account_id: "account-id", period_days: null } as never,
      "account-id",
    ),
    true,
  );
  assert.equal(
    isAllHistoryOptionReview(
      { account_id: "account-id", period_days: 365 } as never,
      "account-id",
    ),
    false,
  );
  assert.equal(
    isAllHistoryOptionReview(
      { account_id: "other-account", period_days: null } as never,
      "account-id",
    ),
    false,
  );
  assert.equal(isAllHistoryOptionReview(null, "account-id"), false);
});

test("all-history request matching rejects stale error provenance", () => {
  assert.equal(
    isAllHistoryOptionReviewRequest("account-id", null, "account-id"),
    true,
  );
  assert.equal(
    isAllHistoryOptionReviewRequest("account-id", 365, "account-id"),
    false,
  );
  assert.equal(
    isAllHistoryOptionReviewRequest("other-account", null, "account-id"),
    false,
  );
  assert.equal(
    isAllHistoryOptionReviewRequest(null, null, "account-id"),
    false,
  );
});

test("option contract row keys cover grouped parents and expanded children", () => {
  assert.equal(getOptionContractRowKey({ key: "group-key", id: "parent-id" }), "group-key");
  assert.equal(getOptionContractRowKey({ id: "child-id" }), "child-id");
});

test("post-import refresh is skipped after the user changes accounts", () => {
  assert.equal(
    resolveCurrentOptionAccount("account-a", "account-a"),
    "account-a",
  );
  assert.equal(
    resolveCurrentOptionAccount("account-a", "account-b"),
    null,
  );
});

test("expired statistics count option records rather than contract quantities", () => {
  assert.deepEqual(
    buildExpiredOptionStats([
      contract({ id: "assigned", contracts: 2, status: "assigned" }),
      contract({ id: "expired", contracts: 3, status: "expired" }),
      contract({ id: "closed", contracts: 4, status: "closed" }),
    ]),
    {
      total_contracts: 3,
      assigned_contracts: 1,
      expired_contracts: 1,
      assignment_ratio: 1 / 3,
    },
  );
});
