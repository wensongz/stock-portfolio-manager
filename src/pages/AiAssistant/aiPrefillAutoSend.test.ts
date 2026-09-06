// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  decideAiSessionTransition,
  stageNonAutoAiPrefill,
  runAiPrefillAutoSend,
  shouldAutoSendPrefill,
} from "./aiPrefillAutoSend.ts";

function validRebalanceRequest(configId = "config-us") {
  return {
    prompt: "请根据当前违规生成再平衡建议。",
    activeSkill: "portfolio-rebalance",
    autoSend: true,
    toolContext: {
      name: "get_rebalance_context",
      arguments: { config_id: configId },
    },
  };
}

test("missing configuration waits without consuming the request", () => {
  let consumed = false;
  const shouldSend = shouldAutoSendPrefill({
    request: validRebalanceRequest(),
    consumed,
    configured: false,
    sending: false,
  });

  if (shouldSend) consumed = true;
  assert.equal(shouldSend, false);
  assert.equal(consumed, false);
});

test("a non-auto legacy request keeps its composer seed and ordered staging", () => {
  const calls: unknown[] = [];
  const request = {
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
  };

  const seed = stageNonAutoAiPrefill(request, {
    stageSkill: (skill) => calls.push(["skill", skill]),
    stageTool: (tool) => calls.push(["tool", tool]),
  });

  assert.equal(seed, "复盘 AAPL");
  assert.deepEqual(calls, [
    ["skill", "stock-review"],
    ["tool", request.toolContext],
  ]);
});

test("sending rerenders do not start a duplicate auto-send", () => {
  assert.equal(shouldAutoSendPrefill({
    request: validRebalanceRequest(),
    consumed: false,
    configured: true,
    sending: true,
  }), false);
});

test("a consumed request never resends after sending finishes or the session id changes", () => {
  const request = validRebalanceRequest();
  assert.equal(shouldAutoSendPrefill({
    request,
    consumed: true,
    configured: true,
    sending: true,
  }), false);
  assert.equal(shouldAutoSendPrefill({
    request,
    consumed: true,
    configured: true,
    sending: false,
  }), false);
});

test("the expected null-to-new session transition keeps the in-flight auto-send", () => {
  const first = decideAiSessionTransition({
    nextSessionId: "session-new",
    loadedSessionId: null,
    expectingSessionCreation: true,
  });
  assert.equal(first, "KEEP_IN_FLIGHT");

  const rerender = decideAiSessionTransition({
    nextSessionId: "session-new",
    loadedSessionId: "session-new",
    expectingSessionCreation: false,
  });
  assert.equal(rerender, "UNCHANGED");
});

test("ordinary session transitions retain clear and history-load behavior", () => {
  assert.equal(decideAiSessionTransition({
    nextSessionId: null,
    loadedSessionId: "session-old",
    expectingSessionCreation: false,
  }), "CLEAR");
  assert.equal(decideAiSessionTransition({
    nextSessionId: "session-history",
    loadedSessionId: "session-old",
    expectingSessionCreation: false,
  }), "LOAD");
});

test("auto-send stages trusted context before creating exactly one new session", async () => {
  const calls: unknown[] = [];
  const request = validRebalanceRequest("config-us");

  const sessionId = await runAiPrefillAutoSend(request, {
    stageSkill: (skill) => calls.push(["skill", skill]),
    stageTool: (tool) => calls.push(["tool", tool]),
    createSession: async () => {
      calls.push(["create"]);
      return "session-new";
    },
    sendMessage: async (prompt, sid) => calls.push(["send", sid, prompt]),
    touchSession: async (sid) => calls.push(["touch", sid]),
    renameSession: async (sid, prompt) => calls.push(["rename", sid, prompt]),
  });

  assert.equal(sessionId, "session-new");
  assert.deepEqual(calls, [
    ["skill", "portfolio-rebalance"],
    ["tool", { name: "get_rebalance_context", arguments: { config_id: "config-us" } }],
    ["create"],
    ["send", "session-new", "请根据当前违规生成再平衡建议。"],
    ["touch", "session-new"],
    ["rename", "session-new", "请根据当前违规生成再平衡建议。"],
  ]);
  assert.equal(calls.some((call) => JSON.stringify(call).includes("session-old")), false);
  assert.equal(calls.filter((call) => call[0] === "create").length, 1);
  assert.equal(calls.filter((call) => call[0] === "send").length, 1);
});

test("send failure propagates and a consumed request is not retried automatically", async () => {
  const calls: string[] = [];
  const failure = new Error("stream failed");
  const request = validRebalanceRequest();
  let consumed = false;
  const dependencies = {
    stageSkill: () => calls.push("skill"),
    stageTool: () => calls.push("tool"),
    createSession: async () => {
      calls.push("create:session-new");
      return "session-new";
    },
    sendMessage: async (_prompt, sessionId) => {
      calls.push(`send:${sessionId}`);
      throw failure;
    },
    touchSession: async (sessionId) => calls.push(`touch:${sessionId}`),
    renameSession: async (sessionId) => calls.push(`rename:${sessionId}`),
  };

  if (shouldAutoSendPrefill({ request, consumed, configured: true, sending: false })) {
    consumed = true;
    await assert.rejects(runAiPrefillAutoSend(request, dependencies), failure);
  }
  if (shouldAutoSendPrefill({ request, consumed, configured: true, sending: false })) {
    await runAiPrefillAutoSend(request, dependencies);
  }

  assert.equal(consumed, true);
  assert.deepEqual(calls, [
    "skill",
    "tool",
    "create:session-new",
    "send:session-new",
  ]);
});
