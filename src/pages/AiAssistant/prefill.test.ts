// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  consumeAiPrefillToolContext,
  readAiPrefill,
  readAiPrefillActiveSkill,
  readAiPrefillToolContext,
  resolveAiPrefillSessionId,
} from "./prefill.ts";

test("reads a non-empty prefill prompt", () => {
  assert.equal(readAiPrefill({ prefillPrompt: "  复盘 AAPL  " }), "复盘 AAPL");
});

test("rejects missing, blank, and non-string prompts", () => {
  assert.equal(readAiPrefill(null), null);
  assert.equal(readAiPrefill({ prefillPrompt: "  " }), null);
  assert.equal(readAiPrefill({ prefillPrompt: 42 }), null);
});

test("valid prefill targets a new chat instead of the active session", () => {
  assert.equal(resolveAiPrefillSessionId("复盘 AAPL", "existing-session"), null);
});

test("ordinary navigation preserves the active session", () => {
  assert.equal(resolveAiPrefillSessionId(null, "existing-session"), "existing-session");
});

test("reads a stock-review skill activation only from a valid prefill", () => {
  assert.equal(
    readAiPrefillActiveSkill({
      prefillPrompt: "复盘 AAPL",
      prefillActiveSkill: "stock-review",
      prefillAutoSend: false,
    }),
    "stock-review",
  );
  assert.equal(
    readAiPrefillActiveSkill({
      prefillPrompt: "复盘 AAPL",
      prefillActiveSkill: "stock-review",
      prefillAutoSend: true,
    }),
    null,
  );
  assert.equal(
    readAiPrefillActiveSkill({ prefillActiveSkill: "stock-review" }),
    null,
  );
});

test("reads the exact portfolio and Campaign tool scope without changing the visible prompt", () => {
  const portfolio = {
    prefillPrompt: "approved portfolio prompt",
    prefillActiveSkill: "stock-review",
    prefillAutoSend: false,
    prefillToolName: "get_stock_review",
    prefillToolArguments: {
      start_date: "2026-01-01",
      end_date: "2026-08-28",
      base_currency: "USD",
      account_id: "account-a",
      market: "US",
      benchmark_symbol: "SPY",
    },
  };
  assert.deepEqual(readAiPrefillToolContext(portfolio), {
    name: "get_stock_review",
    arguments: portfolio.prefillToolArguments,
  });
  assert.equal(readAiPrefill(portfolio), "approved portfolio prompt");

  const campaign = {
    ...portfolio,
    prefillToolArguments: {
      ...portfolio.prefillToolArguments,
      symbol: "AAPL",
      campaign_id: "campaign-7",
    },
  };
  assert.deepEqual(readAiPrefillToolContext(campaign), {
    name: "get_stock_review",
    arguments: campaign.prefillToolArguments,
  });
});

test("structured tool context is staged for exactly one next chat turn", () => {
  const pending = {
    name: "get_stock_review" as const,
    arguments: {
      start_date: "2026-01-01",
      end_date: "2026-08-28",
      base_currency: "USD",
    },
  };
  const first = consumeAiPrefillToolContext(pending);
  assert.deepEqual(first.current, pending);
  assert.equal(first.next, null);
  const second = consumeAiPrefillToolContext(first.next);
  assert.equal(second.current, null);
  assert.equal(second.next, null);
});

test("rejects incomplete, auto-send, unsupported, and extra-key tool contexts", () => {
  const base = {
    prefillPrompt: "approved prompt",
    prefillActiveSkill: "stock-review",
    prefillAutoSend: false,
    prefillToolName: "get_stock_review",
    prefillToolArguments: {
      start_date: "2026-01-01",
      end_date: "2026-08-28",
      base_currency: "USD",
    },
  };
  assert.equal(readAiPrefillToolContext({ ...base, prefillAutoSend: true }), null);
  assert.equal(readAiPrefillToolContext({ ...base, prefillToolName: "get_transactions" }), null);
  assert.equal(readAiPrefillToolContext({ ...base, prefillToolArguments: { start_date: "2026-01-01" } }), null);
  assert.equal(readAiPrefillToolContext({
    ...base,
    prefillToolArguments: { ...base.prefillToolArguments, unexpected: true },
  }), null);
});
