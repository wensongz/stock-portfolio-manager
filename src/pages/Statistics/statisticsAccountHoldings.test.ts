// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import { resolveAccountHoldingsCoverage } from "./statisticsAccountHoldings.ts";

test("account holdings adapter distinguishes unknown, known empty, and known symbols", () => {
  const unknown = resolveAccountHoldingsCoverage(
    { accountStats: {}, overviewByCurrency: {} },
    "acct-b",
    "USD",
  );
  const knownEmpty = resolveAccountHoldingsCoverage(
    {
      accountStats: { "acct-b": { holdings: [] } },
      overviewByCurrency: {},
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
