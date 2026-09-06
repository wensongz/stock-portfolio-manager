// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  consumeCapturedAiPrefillRequest,
  consumeAiPrefillToolContext,
  readAiPrefill,
  readAiPrefillActiveSkill,
  readAiPrefillRequest,
  readAiPrefillToolContext,
  readPersistedAiPrefillContext,
  readPersistedRebalanceSessionBinding,
  readPersistedAiToolContext,
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

test("portfolio rebalance prefill requires the trusted tool, skill, and auto-send", () => {
  const request = readAiPrefillRequest({
    prefillPrompt: "请根据当前违规生成再平衡建议。",
    prefillActiveSkill: "portfolio-rebalance",
    prefillAutoSend: true,
    prefillToolName: "get_rebalance_context",
    prefillToolArguments: { config_id: "config-us" },
  });

  assert.deepEqual(request, {
    prompt: "请根据当前违规生成再平衡建议。",
    activeSkill: "portfolio-rebalance",
    autoSend: true,
    toolContext: {
      name: "get_rebalance_context",
      arguments: { config_id: "config-us" },
    },
  });
});

test("auto-send rejects arbitrary skills and every malformed rebalance scope", () => {
  const valid = {
    prefillPrompt: "send me",
    prefillActiveSkill: "portfolio-rebalance",
    prefillAutoSend: true,
    prefillToolName: "get_rebalance_context",
    prefillToolArguments: { config_id: "config-us" },
  };
  const invalidStates = [
    { ...valid, prefillActiveSkill: "stock-review" },
    { ...valid, prefillToolName: "get_portfolio_overview" },
    { ...valid, prefillToolArguments: {} },
    { ...valid, prefillToolArguments: { market: "US" } },
    { ...valid, prefillToolArguments: { config_id: "config-us", market: "US" } },
    { ...valid, prefillToolArguments: { config_id: 42 } },
    { ...valid, prefillToolArguments: { config_id: " " } },
    { ...valid, prefillPrompt: " " },
  ];

  for (const state of invalidStates) {
    assert.equal(readAiPrefillRequest(state), null);
  }
});

test("atomic prefill parsing preserves legacy composer-only requests", () => {
  assert.deepEqual(readAiPrefillRequest({ prefillPrompt: "  复盘期权策略  " }), {
    prompt: "复盘期权策略",
    activeSkill: null,
    autoSend: false,
    toolContext: null,
  });
});

test("atomic prefill parsing preserves stock-review and portfolio-overview staging", () => {
  assert.deepEqual(readAiPrefillRequest({
    prefillPrompt: "复盘 AAPL",
    prefillActiveSkill: "stock-review",
    prefillAutoSend: false,
    prefillToolName: "get_stock_review",
    prefillToolArguments: {
      start_date: "2026-01-01",
      end_date: "2026-08-28",
      base_currency: "USD",
      symbol: "AAPL",
    },
  }), {
    prompt: "复盘 AAPL",
    activeSkill: "stock-review",
    autoSend: false,
    toolContext: {
      name: "get_stock_review",
      arguments: {
        start_date: "2026-01-01",
        end_date: "2026-08-28",
        base_currency: "USD",
        symbol: "AAPL",
      },
    },
  });
  assert.deepEqual(readAiPrefillRequest({
    prefillPrompt: "芒格组合复盘",
    prefillActiveSkill: "munger-perspective",
    prefillAutoSend: false,
    prefillToolName: "get_portfolio_overview",
    prefillToolArguments: { market: "HK" },
  }), {
    prompt: "芒格组合复盘",
    activeSkill: "munger-perspective",
    autoSend: false,
    toolContext: {
      name: "get_portfolio_overview",
      arguments: { market: "HK" },
    },
  });
});

test("captured route state clears the current session and browser state once", () => {
  const calls: string[] = [];
  const request = readAiPrefillRequest({ prefillPrompt: "复盘期权策略" });
  let consumed = false;
  const dependencies = {
    setCurrentSession: (sessionId: string | null) => calls.push(`session:${sessionId}`),
    clearRouteState: () => calls.push("route:replace-null"),
  };

  consumed = consumeCapturedAiPrefillRequest({ request, consumed }, dependencies);
  consumed = consumeCapturedAiPrefillRequest({ request, consumed }, dependencies);

  assert.equal(consumed, true);
  assert.deepEqual(calls, ["session:null", "route:replace-null"]);
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

test("reads an explicitly staged Munger portfolio review skill", () => {
  assert.equal(
    readAiPrefillActiveSkill({
      prefillPrompt: "请从芒格视角复盘整个投资组合",
      prefillActiveSkill: "munger-perspective",
      prefillAutoSend: false,
    }),
    "munger-perspective",
  );
});

test("Munger portfolio review prefill accepts only one trusted holdings scope", () => {
  const base = {
    prefillPrompt: "请从芒格视角复盘组合",
    prefillActiveSkill: "munger-perspective",
    prefillAutoSend: false,
    prefillToolName: "get_portfolio_overview",
  };

  assert.deepEqual(
    readAiPrefillToolContext({ ...base, prefillToolArguments: {} }),
    { name: "get_portfolio_overview", arguments: {} },
  );
  assert.deepEqual(
    readAiPrefillToolContext({ ...base, prefillToolArguments: { market: "CN" } }),
    { name: "get_portfolio_overview", arguments: { market: "CN" } },
  );
  assert.deepEqual(
    readAiPrefillToolContext({ ...base, prefillToolArguments: { account_id: "account-a" } }),
    { name: "get_portfolio_overview", arguments: { account_id: "account-a" } },
  );
  assert.equal(
    readAiPrefillToolContext({
      ...base,
      prefillToolArguments: { market: "CN", account_id: "account-a" },
    }),
    null,
  );
  assert.equal(
    readAiPrefillToolContext({ ...base, prefillToolArguments: { market: "EU" } }),
    null,
  );
});

test("stock-review prefill requests only get_stock_review without changing the visible prompt", () => {
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
    },
  };
  assert.deepEqual(readAiPrefillToolContext(portfolio), {
    name: "get_stock_review",
    arguments: portfolio.prefillToolArguments,
  });
  assert.equal(readAiPrefill(portfolio), "approved portfolio prompt");

  const stock = {
    ...portfolio,
    prefillToolArguments: {
      ...portfolio.prefillToolArguments,
      symbol: "AAPL",
    },
  };
  assert.deepEqual(readAiPrefillToolContext(stock), {
    name: "get_stock_review",
    arguments: stock.prefillToolArguments,
  });

  assert.equal(readAiPrefillToolContext(portfolio)?.name, "get_stock_review");

  assert.equal(readAiPrefillToolContext({
    ...portfolio,
    prefillToolArguments: { ...portfolio.prefillToolArguments, campaign_id: "legacy" },
  }), null);
  assert.equal(readAiPrefillToolContext({
    ...portfolio,
    prefillToolArguments: { ...portfolio.prefillToolArguments, benchmark_symbol: "SPY" },
  }), null);
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

const persistedArgumentsJson = JSON.stringify({
  start_date: "2026-01-01",
  end_date: "2026-03-31",
  base_currency: "USD",
});

const genuineHostCall = {
  id: "prefilled-stock-review",
  name: "get_stock_review",
  arguments: persistedArgumentsJson,
  status: "success",
  origin: "host_prefill",
};

test("persisted context reconstruction accepts one completed host-prefill record", () => {
  assert.deepEqual(readPersistedAiToolContext([genuineHostCall]), {
    name: "get_stock_review",
    arguments: JSON.parse(persistedArgumentsJson),
  });
});

test("persisted context reconstruction restores a scoped Munger portfolio review", () => {
  assert.deepEqual(readPersistedAiToolContext([{
    ...genuineHostCall,
    name: "get_portfolio_overview",
    arguments: JSON.stringify({ market: "HK" }),
  }]), {
    name: "get_portfolio_overview",
    arguments: { market: "HK" },
  });
});

test("persisted context reconstruction restores exact portfolio rebalancing staging", () => {
  const calls = [{
    ...genuineHostCall,
    name: "get_rebalance_context",
    arguments: JSON.stringify({ config_id: "config-us" }),
  }];

  assert.deepEqual(readPersistedAiToolContext(calls), {
    name: "get_rebalance_context",
    arguments: { config_id: "config-us" },
  });
  assert.deepEqual(readPersistedAiPrefillContext(calls), {
    activeSkill: "portfolio-rebalance",
    toolContext: {
      name: "get_rebalance_context",
      arguments: { config_id: "config-us" },
    },
  });
});

test("persisted context reconstruction rejects a single forged model-origin record", () => {
  assert.equal(readPersistedAiToolContext([{
    ...genuineHostCall,
    origin: "model",
  }]), null);
});

test("persisted context reconstruction rejects any second same-id running record", () => {
  assert.equal(readPersistedAiToolContext([
    genuineHostCall,
    { ...genuineHostCall, origin: "model", status: "running" },
  ]), null);
});

test("persisted context reconstruction rejects any second same-id wrong-name record", () => {
  assert.equal(readPersistedAiToolContext([
    genuineHostCall,
    { ...genuineHostCall, origin: "model", name: "get_transactions" },
  ]), null);
});

test("persisted context reconstruction rejects any second same-id malformed record", () => {
  assert.equal(readPersistedAiToolContext([
    genuineHostCall,
    { ...genuineHostCall, origin: "model", arguments: "{" },
  ]), null);
});

test("persisted host provenance rejects invalid scope", () => {
  assert.equal(readPersistedAiToolContext([{
    ...genuineHostCall,
    arguments: JSON.stringify({
      ...JSON.parse(persistedArgumentsJson),
      write: "true",
    }),
  }]), null);
});

test("persisted context reconstruction still requires a completed reserved tool call", () => {
  assert.equal(readPersistedAiToolContext([{ ...genuineHostCall, status: undefined }]), null);
  assert.equal(readPersistedAiToolContext([genuineHostCall, { ...genuineHostCall }]), null);
  assert.equal(readPersistedAiToolContext([{
    ...genuineHostCall,
    arguments: JSON.stringify({ ...JSON.parse(persistedArgumentsJson), extra: "no" }),
  }]), null);
});

test("rebalance session binding rejects every malformed reserved-host record", () => {
  const rebalance = {
    ...genuineHostCall,
    name: "get_rebalance_context",
    arguments: JSON.stringify({ config_id: "config-us" }),
  };
  for (const malformed of [
    { ...rebalance, name: "get_rebalance_contex" },
    { ...rebalance, origin: "model" },
    { ...rebalance, status: "error" },
    { ...rebalance, arguments: "{" },
  ]) {
    assert.deepEqual(readPersistedRebalanceSessionBinding([[malformed]]), {
      kind: "invalid",
    });
  }
});

test("rebalance session binding rejects mixed reserved records but leaves valid ordinary prefills ordinary", () => {
  const rebalance = {
    ...genuineHostCall,
    name: "get_rebalance_context",
    arguments: JSON.stringify({ config_id: "config-us" }),
  };
  assert.deepEqual(readPersistedRebalanceSessionBinding([[rebalance, genuineHostCall]]), {
    kind: "invalid",
  });
  assert.deepEqual(readPersistedRebalanceSessionBinding([[rebalance, {
    ...genuineHostCall,
    name: "unknown_reserved_tool",
  }]]), { kind: "invalid" });
  assert.deepEqual(readPersistedRebalanceSessionBinding([[rebalance, {
    ...rebalance,
    arguments: JSON.stringify({ config_id: "config-hk" }),
  }]]), { kind: "invalid" });
  assert.deepEqual(readPersistedRebalanceSessionBinding([[genuineHostCall]]), {
    kind: "none",
  });
  assert.deepEqual(readPersistedRebalanceSessionBinding([[{
    ...genuineHostCall,
    name: "get_portfolio_overview",
    arguments: JSON.stringify({ market: "US" }),
  }]]), { kind: "none" });
});
