// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test, { beforeEach } from "node:test";
import assert from "node:assert/strict";

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

const exactContext = {
  name: "get_stock_review",
  arguments: {
    start_date: "2026-01-01",
    end_date: "2026-03-31",
    base_currency: "USD",
    account_id: "account-a",
    market: "US",
    symbol: "AAPL",
  },
};

let invokeImpl = () => Promise.reject(new Error("invoke not configured"));
let callbackId = 0;
const callbacks = new Map();
const eventHandlers = new Map();
globalThis.window = {
  __TAURI_INTERNALS__: {
    invoke(command: string, args: unknown) {
      return invokeImpl(command, args);
    },
    transformCallback(callback) {
      callbackId += 1;
      callbacks.set(callbackId, callback);
      return callbackId;
    },
  },
};

const { useChatStore } = await import("./chatStore.ts");

beforeEach(() => {
  invokeImpl = () => Promise.reject(new Error("invoke not configured"));
  useChatStore.setState({
    messages: [],
    sending: false,
    error: null,
    contextEnabled: true,
    streamingInBackground: false,
    streamingSessionIdState: null,
    viewSessionId: null,
    pendingActiveSkills: [],
    pendingToolContext: null,
  });
});

test("host provenance survives model-id collision, persistence, reload, regeneration, and a later live turn", async () => {
  const chatTurn = deferred();
  const savedSnapshots = [];
  let records = [];
  const chatRequests = [];
  invokeImpl = async (command, args) => {
    if (command === "plugin:event|listen") {
      eventHandlers.set(args.event, callbacks.get(args.handler));
      return args.handler;
    }
    if (command === "get_chat_messages") return records;
    if (command === "chat_with_ai") {
      chatRequests.push(args.req);
      return chatRequests.length === 1 ? chatTurn.promise : undefined;
    }
    if (command === "save_chat_messages") {
      savedSnapshots.push(args.messages);
      return;
    }
    if (command === "touch_chat_session") return;
    throw new Error(`unexpected command ${command}`);
  };

  useChatStore.getState().init();
  await Promise.resolve();
  useChatStore.getState().setActiveSkillsForNextTurn(["stock-review"]);
  useChatStore.getState().setToolContextForNextTurn(exactContext);
  const sending = useChatStore.getState().sendMessage("approved visible prompt", "session-1");
  await Promise.resolve();
  eventHandlers.get("ai-chat-tool-call")({
    payload: {
      id: "prefilled-stock-review",
      name: "get_stock_review",
      arguments: JSON.stringify(exactContext.arguments),
      status: "success",
      result: "{}",
      origin: "host_prefill",
    },
  });
  eventHandlers.get("ai-chat-tool-call")({
    payload: {
      id: "prefilled-stock-review",
      name: "get_transactions",
      arguments: "{}",
      status: "running",
      origin: "model",
    },
  });
  const collidingCalls = useChatStore.getState().messages[1].toolCalls;
  assert.equal(collidingCalls.length, 2, "model lifecycle must not overwrite host provenance");
  assert.deepEqual(
    collidingCalls.find((call) => call.origin === "host_prefill"),
    {
      id: "prefilled-stock-review",
      name: "get_stock_review",
      arguments: JSON.stringify(exactContext.arguments),
      status: "success",
      result: "{}",
      origin: "host_prefill",
    },
  );
  assert.equal(
    collidingCalls.find((call) => call.origin === "model").id,
    "model:prefilled-stock-review",
  );
  eventHandlers.get("ai-chat-delta")({ payload: "completed answer" });
  eventHandlers.get("ai-chat-done")({ payload: null });
  chatTurn.resolve();
  await sending;
  await new Promise((resolve) => setImmediate(resolve));

  assert.equal(useChatStore.getState().pendingToolContext, null);
  const ordinarySending = useChatStore.getState().sendMessage("ordinary follow-up", "session-1");
  await Promise.resolve();
  assert.equal(chatRequests[1].toolContext, null, "host context is live-one-shot");
  eventHandlers.get("ai-chat-delta")({ payload: "ordinary answer" });
  eventHandlers.get("ai-chat-done")({ payload: null });
  await ordinarySending;
  await new Promise((resolve) => setImmediate(resolve));

  records = savedSnapshots.findLast((snapshot) => snapshot.length === 4);
  assert.ok(records, "both completed turns should be persisted");
  const persistedToolCalls = JSON.parse(records[1].tool_calls);
  assert.equal(persistedToolCalls[0].id, "prefilled-stock-review");
  assert.equal(persistedToolCalls[0].origin, "host_prefill");

  useChatStore.setState({ messages: [], sending: false, viewSessionId: null });
  await useChatStore.getState().loadSessionMessages("session-1");
  const restored = useChatStore.getState().messages[1];
  assert.deepEqual(restored.explicitToolContext, exactContext);
  assert.deepEqual(restored.explicitSkillIds, ["stock-review"]);

  await useChatStore.getState().regenerateMessage(restored.id, "session-1");
  assert.equal(chatRequests.length, 3);
  assert.deepEqual(chatRequests[2].toolContext, exactContext);
  assert.deepEqual(chatRequests[2].activeSkills, ["stock-review"]);
  assert.deepEqual(chatRequests[2].messages, [
    { role: "user", content: "approved visible prompt" },
  ]);
});

function persistedMessage(id, sessionId, content) {
  return {
    id,
    session_id: sessionId,
    role: "user",
    content,
    prompt_tokens: 0,
    completion_tokens: 0,
    total_tokens: 0,
    cached_tokens: 0,
    created_at: "2026-09-02T00:00:00.000Z",
  };
}

test("switching away lets a background reply finish without overwriting the new session", async () => {
  const savedSnapshots = [];
  const otherRecord = persistedMessage("other-user", "session-other", "other session");
  invokeImpl = async (command, args) => {
    if (command === "plugin:event|listen") {
      eventHandlers.set(args.event, callbacks.get(args.handler));
      return args.handler;
    }
    if (command === "chat_with_ai" || command === "touch_chat_session") return;
    if (command === "get_chat_messages") return [otherRecord];
    if (command === "save_chat_messages") {
      savedSnapshots.push({ sessionId: args.sessionId, messages: args.messages });
      return;
    }
    throw new Error(`unexpected command ${command}`);
  };

  useChatStore.getState().init();
  await Promise.resolve();
  useChatStore.setState({ viewSessionId: "session-background" });
  await useChatStore.getState().sendMessage("background question", "session-background");
  await useChatStore.getState().resetForSessionSwitch();
  await useChatStore.getState().loadSessionMessages("session-other");

  assert.equal(useChatStore.getState().streamingInBackground, true);
  eventHandlers.get("ai-chat-delta")({ payload: "background answer" });
  eventHandlers.get("ai-chat-done")({ payload: null });
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    useChatStore.getState().messages.map((row) => row.content),
    ["other session"],
  );
  const completed = savedSnapshots.findLast(
    (snapshot) => snapshot.sessionId === "session-background",
  );
  assert.deepEqual(
    completed.messages.map((row) => row.content),
    ["background question", "background answer"],
  );
  assert.equal(useChatStore.getState().sending, false);
  assert.equal(useChatStore.getState().streamingInBackground, false);
});

test("switching back promotes the background buffer to the live stream", async () => {
  const savedSnapshots = [];
  invokeImpl = async (command, args) => {
    if (command === "plugin:event|listen") {
      eventHandlers.set(args.event, callbacks.get(args.handler));
      return args.handler;
    }
    if (command === "chat_with_ai" || command === "touch_chat_session") return;
    if (command === "save_chat_messages") {
      savedSnapshots.push(args.messages);
      return;
    }
    throw new Error(`unexpected command ${command}`);
  };

  useChatStore.getState().init();
  await Promise.resolve();
  useChatStore.setState({ viewSessionId: "session-return" });
  await useChatStore.getState().sendMessage("return question", "session-return");
  await useChatStore.getState().resetForSessionSwitch();
  await useChatStore.getState().loadSessionMessages("session-return");

  assert.equal(useChatStore.getState().streamingInBackground, false);
  eventHandlers.get("ai-chat-delta")({ payload: "continued answer" });
  assert.equal(useChatStore.getState().messages[1].content, "continued answer");
  eventHandlers.get("ai-chat-done")({ payload: null });
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    savedSnapshots.at(-1).map((row) => row.content),
    ["return question", "continued answer"],
  );
});

test("stopping a background stream persists its partial reply and preserves the live session", async () => {
  const savedSnapshots = [];
  const otherRecord = persistedMessage("stop-other", "session-stop-other", "keep me");
  invokeImpl = async (command, args) => {
    if (command === "plugin:event|listen") {
      eventHandlers.set(args.event, callbacks.get(args.handler));
      return args.handler;
    }
    if (
      command === "chat_with_ai" ||
      command === "touch_chat_session" ||
      command === "stop_ai_chat"
    ) {
      return;
    }
    if (command === "get_chat_messages") return [otherRecord];
    if (command === "save_chat_messages") {
      savedSnapshots.push({ sessionId: args.sessionId, messages: args.messages });
      return;
    }
    throw new Error(`unexpected command ${command}`);
  };

  useChatStore.getState().init();
  await Promise.resolve();
  useChatStore.setState({ viewSessionId: "session-stop" });
  await useChatStore.getState().sendMessage("stop question", "session-stop");
  eventHandlers.get("ai-chat-delta")({ payload: "partial answer" });
  await useChatStore.getState().resetForSessionSwitch();
  await useChatStore.getState().loadSessionMessages("session-stop-other");
  await useChatStore.getState().stopGeneration();
  await new Promise((resolve) => setImmediate(resolve));

  assert.deepEqual(
    useChatStore.getState().messages.map((row) => row.content),
    ["keep me"],
  );
  const stopped = savedSnapshots.findLast(
    (snapshot) => snapshot.sessionId === "session-stop",
  );
  assert.deepEqual(
    stopped.messages.map((row) => row.content),
    ["stop question", "partial answer"],
  );
  assert.equal(useChatStore.getState().sending, false);
  assert.equal(useChatStore.getState().streamingInBackground, false);
});
