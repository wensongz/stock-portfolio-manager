// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { resolveAccountHoldingsCoverage } from "./statisticsAccountHoldings.ts";

test("account holdings adapter distinguishes unknown, known empty, and known symbols", () => {
  const unknown = resolveAccountHoldingsCoverage(
    {
      accountStats: {},
      overviewByCurrency: {},
      resultRevisionByView: {},
    },
    "acct-b",
    "USD",
  );
  const knownEmpty = resolveAccountHoldingsCoverage(
    {
      accountStats: { "acct-b": { holdings: [] } },
      overviewByCurrency: {},
      resultRevisionByView: { "account:acct-b": 1 },
    },
    "acct-b",
    "USD",
  );
  const knownWithSymbols = resolveAccountHoldingsCoverage(
    {
      accountStats: {},
      overviewByCurrency: {
        USD: {
          holdings: [
            { account_id: "acct-a", symbol: "AAPL", market: "US" },
            { account_id: "acct-b", symbol: "0700.HK", market: "HK" },
          ],
        },
      },
      resultRevisionByView: { "overview:USD": 1 },
    },
    "acct-b",
    "USD",
  );

  assert.deepEqual(unknown, { status: "unknown" });
  assert.deepEqual(knownEmpty, { status: "known-empty" });
  assert.deepEqual(knownWithSymbols, {
    status: "known-with-symbols",
    holdings: [
      { account_id: "acct-b", symbol: "0700.HK", market: "HK" },
    ],
  });
});

test("account holdings adapter chooses the newest authoritative identity source", () => {
  const staleAccount = resolveAccountHoldingsCoverage(
    {
      accountStats: {
        "acct-a": {
          holdings: [
            { account_id: "acct-a", symbol: "AAPL", market: "US" },
          ],
        },
      },
      overviewByCurrency: {
        USD: {
          holdings: [
            { account_id: "acct-a", symbol: "AAPL", market: "US" },
            { account_id: "acct-a", symbol: "MSFT", market: "US" },
          ],
        },
      },
      resultRevisionByView: {
        "account:acct-a": 1,
        "overview:USD": 2,
      },
    },
    "acct-a",
    "USD",
  );

  assert.deepEqual(staleAccount, {
    status: "known-with-symbols",
    holdings: [
      { account_id: "acct-a", symbol: "AAPL", market: "US" },
      { account_id: "acct-a", symbol: "MSFT", market: "US" },
    ],
  });

  const newerAccount = resolveAccountHoldingsCoverage(
    {
      accountStats: {
        "acct-a": {
          holdings: [
            { account_id: "acct-a", symbol: "TSLA", market: "US" },
          ],
        },
      },
      overviewByCurrency: {
        USD: {
          holdings: [
            { account_id: "acct-a", symbol: "AAPL", market: "US" },
          ],
        },
      },
      resultRevisionByView: {
        "overview:USD": 2,
        "account:acct-a": 3,
      },
    },
    "acct-a",
    "USD",
  );

  assert.deepEqual(newerAccount, {
    status: "known-with-symbols",
    holdings: [
      { account_id: "acct-a", symbol: "TSLA", market: "US" },
    ],
  });
});

test("account holdings identity can use a newer overview from another base currency", () => {
  const coverage = resolveAccountHoldingsCoverage(
    {
      accountStats: {},
      overviewByCurrency: {
        USD: {
          holdings: [
            { account_id: "acct-a", symbol: "AAPL", market: "US" },
          ],
        },
      },
      resultRevisionByView: { "overview:USD": 4 },
    },
    "acct-a",
    "CNY",
  );

  assert.deepEqual(coverage, {
    status: "known-with-symbols",
    holdings: [
      { account_id: "acct-a", symbol: "AAPL", market: "US" },
    ],
  });
});
