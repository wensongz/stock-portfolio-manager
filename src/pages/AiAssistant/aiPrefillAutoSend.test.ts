// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  cancelAiPrefillAutoSendOperation,
  createAiPrefillAutoSendOperation,
  decideAiPrefillAutoSendStart,
  decideAiSessionTransition,
  runAiPrefillAutoSend,
  shouldAutoSendPrefill,
  stageNonAutoAiPrefill,
} from "./aiPrefillAutoSend.ts";

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

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

function lifecycleHarness(createResult) {
  const calls = [];
  const state = {
    selectionRevision: 7,
    currentSessionId: null,
    staging: { ownerToken: null, skillIds: [], toolContext: null },
    stagingRevision: 0,
  };
  const dependencies = {
    stageOwnedContext: (ownerToken, skill, toolContext) => {
      calls.push(["stage", ownerToken, skill, toolContext]);
      state.staging = { ownerToken, skillIds: [skill], toolContext };
    },
    ownsStagedContext: (ownerToken, skill, toolContext) => (
      state.staging.ownerToken === ownerToken &&
      state.staging.skillIds.length === 1 &&
      state.staging.skillIds[0] === skill &&
      JSON.stringify(state.staging.toolContext) === JSON.stringify(toolContext)
    ),
    clearOwnedContext: (ownerToken) => {
      calls.push(["clear", ownerToken]);
      if (state.staging.ownerToken === ownerToken) {
        state.staging = { ownerToken: null, skillIds: [], toolContext: null };
      }
    },
    getStagingRevision: () => state.stagingRevision,
    createSession: async () => {
      calls.push(["create"]);
      return createResult instanceof Promise ? await createResult : createResult;
    },
    getSelectionState: () => ({
      revision: state.selectionRevision,
      currentSessionId: state.currentSessionId,
    }),
    claimSession: (expectedRevision, sessionId) => {
      calls.push(["claim", expectedRevision, sessionId]);
      if (
        state.selectionRevision !== expectedRevision ||
        state.currentSessionId !== null
      ) {
        return false;
      }
      state.currentSessionId = sessionId;
      state.selectionRevision += 1;
      return true;
    },
    sendMessage: async (prompt, sessionId) => {
      calls.push(["send", sessionId, prompt]);
      state.staging = { ownerToken: null, skillIds: [], toolContext: null };
      return { ok: true };
    },
    touchSession: async (sessionId) => calls.push(["touch", sessionId]),
    renameSession: async (sessionId, prompt) => calls.push(["rename", sessionId, prompt]),
  };
  return { calls, state, dependencies };
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

test("auto-send reserves before readiness and a manual selection cancels it permanently", () => {
  const operation = createAiPrefillAutoSendOperation(7, 3, null);
  const calls = [];
  let consumed = false;

  assert.equal(decideAiPrefillAutoSendStart({
    operation,
    configured: false,
    sending: true,
    selectionRevision: 7,
    currentSessionId: null,
    stagingRevision: 3,
  }), "WAIT");
  assert.equal(operation.phase, "RESERVED");

  const afterManualSelection = decideAiPrefillAutoSendStart({
    operation,
    configured: true,
    sending: false,
    selectionRevision: 8,
    currentSessionId: "session-manual",
    stagingRevision: 3,
  });
  assert.equal(afterManualSelection, "CANCEL");
  consumed = true;
  cancelAiPrefillAutoSendOperation(operation, {
    clearOwnedContext: () => calls.push("clear"),
  });

  assert.equal(decideAiPrefillAutoSendStart({
    operation,
    configured: true,
    sending: false,
    selectionRevision: 8,
    currentSessionId: "session-manual",
    stagingRevision: 3,
  }), "IGNORE");
  assert.equal(consumed, true);
  assert.deepEqual(calls, ["clear"]);
});

test("a staging change while waiting cancels before staging or creating", async () => {
  const operation = createAiPrefillAutoSendOperation(7, 3, null);
  const harness = lifecycleHarness(Promise.resolve("orphan-session"));
  harness.state.stagingRevision = 4;

  assert.equal(decideAiPrefillAutoSendStart({
    operation,
    configured: true,
    sending: false,
    selectionRevision: 7,
    currentSessionId: null,
    stagingRevision: 4,
  }), "CANCEL");
  cancelAiPrefillAutoSendOperation(operation, harness.dependencies);
  assert.deepEqual(
    await runAiPrefillAutoSend(validRebalanceRequest(), operation, harness.dependencies),
    { status: "cancelled" },
  );
  assert.equal(harness.calls.some((call) => call[0] === "stage"), false);
  assert.equal(harness.calls.some((call) => call[0] === "create"), false);
  assert.equal(harness.calls.some((call) => call[0] === "send"), false);
});

test("an unchanged reservation starts once when readiness opens", async () => {
  const operation = createAiPrefillAutoSendOperation(7, 0, null);
  const harness = lifecycleHarness(Promise.resolve("session-created"));

  assert.equal(decideAiPrefillAutoSendStart({
    operation,
    configured: false,
    sending: false,
    selectionRevision: 7,
    currentSessionId: null,
    stagingRevision: 0,
  }), "WAIT");
  assert.equal(decideAiPrefillAutoSendStart({
    operation,
    configured: true,
    sending: false,
    selectionRevision: 7,
    currentSessionId: null,
    stagingRevision: 0,
  }), "START");

  await runAiPrefillAutoSend(validRebalanceRequest(), operation, harness.dependencies);
  assert.equal(harness.calls.filter((call) => call[0] === "create").length, 1);
  assert.equal(harness.calls.filter((call) => call[0] === "send").length, 1);
  assert.equal(decideAiPrefillAutoSendStart({
    operation,
    configured: true,
    sending: false,
    selectionRevision: 8,
    currentSessionId: "session-created",
    stagingRevision: 0,
  }), "IGNORE");
});

test("an already-selected session cancels before owned staging or detached create", async () => {
  const operation = createAiPrefillAutoSendOperation(7, 0, null);
  const harness = lifecycleHarness(Promise.resolve("orphan-session"));
  harness.state.selectionRevision = 8;
  harness.state.currentSessionId = "session-manual";

  assert.deepEqual(
    await runAiPrefillAutoSend(validRebalanceRequest(), operation, harness.dependencies),
    { status: "cancelled" },
  );
  assert.equal(operation.phase, "CANCELLED");
  assert.equal(harness.calls.some((call) => call[0] === "stage"), false);
  assert.equal(harness.calls.some((call) => call[0] === "create"), false);
});

test("a non-auto legacy request keeps its composer seed and ordered staging", () => {
  const calls = [];
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

test("sending and consumed rerenders never start a duplicate auto-send", () => {
  const request = validRebalanceRequest();
  assert.equal(shouldAutoSendPrefill({
    request,
    consumed: false,
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

test("deferred creation treats a manual session selection as cancel-and-load", async () => {
  const creation = deferred();
  const harness = lifecycleHarness(creation.promise);
  const operation = createAiPrefillAutoSendOperation(7);
  const running = runAiPrefillAutoSend(
    validRebalanceRequest(),
    operation,
    harness.dependencies,
  );
  assert.equal(operation.phase, "CREATING");

  harness.state.currentSessionId = "session-manual";
  harness.state.selectionRevision = 8;
  const transition = decideAiSessionTransition({
    nextSessionId: "session-manual",
    loadedSessionId: null,
    expectedCreatedSessionId: null,
    autoSendOperation: operation,
  });
  assert.equal(transition, "CANCEL_AUTO_AND_LOAD");
  cancelAiPrefillAutoSendOperation(operation, harness.dependencies);
  creation.resolve("session-late");

  assert.deepEqual(await running, { status: "cancelled" });
  assert.equal(harness.state.currentSessionId, "session-manual");
  assert.equal(harness.calls.some((call) => call[0] === "claim"), false);
  assert.equal(harness.calls.some((call) => call[0] === "send"), false);
});

test("an arbitrary non-null session is never mistaken for the owned created session", () => {
  const operation = createAiPrefillAutoSendOperation(3);
  operation.phase = "CLAIMING";
  operation.expectedSessionId = "session-created";

  assert.equal(decideAiSessionTransition({
    nextSessionId: "session-manual",
    loadedSessionId: null,
    expectedCreatedSessionId: null,
    autoSendOperation: operation,
  }), "CANCEL_AUTO_AND_LOAD");
  assert.equal(decideAiSessionTransition({
    nextSessionId: "session-created",
    loadedSessionId: null,
    expectedCreatedSessionId: null,
    autoSendOperation: operation,
  }), "KEEP_IN_FLIGHT");
});

test("after cancellation a manually selected session loads normally", () => {
  const operation = createAiPrefillAutoSendOperation(3);
  cancelAiPrefillAutoSendOperation(operation, { clearOwnedContext: () => {} });

  assert.equal(decideAiSessionTransition({
    nextSessionId: "session-manual",
    loadedSessionId: null,
    expectedCreatedSessionId: null,
    autoSendOperation: operation,
  }), "LOAD");
});

test("changing staged skill or tool during creation aborts without erasing user values", async () => {
  const creation = deferred();
  const harness = lifecycleHarness(creation.promise);
  const operation = createAiPrefillAutoSendOperation(7);
  const running = runAiPrefillAutoSend(
    validRebalanceRequest(),
    operation,
    harness.dependencies,
  );

  harness.state.staging = {
    ownerToken: null,
    skillIds: ["stock-review"],
    toolContext: {
      name: "get_stock_review",
      arguments: {
        start_date: "2026-01-01",
        end_date: "2026-09-06",
        base_currency: "USD",
      },
    },
  };
  creation.resolve("session-created");

  assert.deepEqual(await running, { status: "cancelled" });
  assert.deepEqual(harness.state.staging, {
    ownerToken: null,
    skillIds: ["stock-review"],
    toolContext: {
      name: "get_stock_review",
      arguments: {
        start_date: "2026-01-01",
        end_date: "2026-09-06",
        base_currency: "USD",
      },
    },
  });
  assert.equal(harness.calls.some((call) => call[0] === "send"), false);
});

test("unchanged ownership claims the exact created ID and sends once", async () => {
  const harness = lifecycleHarness(Promise.resolve("session-created"));
  const operation = createAiPrefillAutoSendOperation(7);

  const result = await runAiPrefillAutoSend(
    validRebalanceRequest(),
    operation,
    harness.dependencies,
  );

  assert.deepEqual(result, { status: "sent", sessionId: "session-created" });
  assert.equal(operation.phase, "SUCCEEDED");
  assert.equal(harness.state.currentSessionId, "session-created");
  assert.deepEqual(harness.calls.map((call) => call[0]), [
    "stage",
    "create",
    "claim",
    "send",
    "touch",
    "rename",
  ]);
  assert.deepEqual(harness.calls[0].slice(2), [
    "portfolio-rebalance",
    {
      name: "get_rebalance_context",
      arguments: { config_id: "config-us" },
    },
  ]);
  assert.deepEqual(harness.calls.filter((call) => call[0] === "claim"), [
    ["claim", 7, "session-created"],
  ]);
  assert.deepEqual(harness.calls.filter((call) => call[0] === "send"), [
    ["send", "session-created", "请根据当前违规生成再平衡建议。"],
  ]);
});

test("blank created session ID fails before claim, send, touch, or rename", async () => {
  const harness = lifecycleHarness(Promise.resolve("   "));
  const operation = createAiPrefillAutoSendOperation(7);

  await assert.rejects(
    runAiPrefillAutoSend(validRebalanceRequest(), operation, harness.dependencies),
    /会话 ID/,
  );

  assert.equal(operation.phase, "FAILED");
  for (const forbidden of ["claim", "send", "touch", "rename"]) {
    assert.equal(harness.calls.some((call) => call[0] === forbidden), false);
  }
  assert.deepEqual(harness.state.staging, {
    ownerToken: null,
    skillIds: [],
    toolContext: null,
  });
});

test("a real failed send outcome propagates without touch, rename, or retry", async () => {
  const harness = lifecycleHarness(Promise.resolve("session-created"));
  harness.dependencies.sendMessage = async (_prompt, sessionId) => {
    harness.calls.push(["send-failed", sessionId]);
    harness.state.staging = { ownerToken: null, skillIds: [], toolContext: null };
    return { ok: false, error: "backend failed" };
  };
  const operation = createAiPrefillAutoSendOperation(7);
  let consumed = false;
  const request = validRebalanceRequest();

  if (shouldAutoSendPrefill({ request, consumed, configured: true, sending: false })) {
    consumed = true;
    await assert.rejects(
      runAiPrefillAutoSend(request, operation, harness.dependencies),
      /backend failed/,
    );
  }

  assert.equal(consumed, true);
  assert.equal(operation.phase, "FAILED");
  assert.equal(harness.calls.filter((call) => call[0] === "send-failed").length, 1);
  assert.equal(harness.calls.some((call) => call[0] === "touch"), false);
  assert.equal(harness.calls.some((call) => call[0] === "rename"), false);
  assert.equal(shouldAutoSendPrefill({ request, consumed, configured: true, sending: false }), false);
});
