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
    benchmark_symbol: "SPY",
    symbol: "AAPL",
    campaign_id: "campaign-7",
  },
};

let invokeImpl = () => Promise.reject(new Error("invoke not configured"));
let callbackId = 0;
const callbacks = new Map();
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

test("persist, reload, then regenerate restores and revalidates prefilled stock-review context", async () => {
  const chatTurn = deferred();
  const eventHandlers = new Map();
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
    },
  });
  eventHandlers.get("ai-chat-delta")({ payload: "completed answer" });
  eventHandlers.get("ai-chat-done")({ payload: null });
  chatTurn.resolve();
  await sending;
  await new Promise((resolve) => setImmediate(resolve));

  records = savedSnapshots.findLast((snapshot) => snapshot.length === 2);
  assert.ok(records, "completed turn should persist both messages");
  assert.equal(JSON.parse(records[1].tool_calls)[0].id, "prefilled-stock-review");

  useChatStore.setState({ messages: [], sending: false, viewSessionId: null });
  await useChatStore.getState().loadSessionMessages("session-1");
  const restored = useChatStore.getState().messages[1];
  assert.deepEqual(restored.explicitToolContext, exactContext);
  assert.deepEqual(restored.explicitSkillIds, ["stock-review"]);

  await useChatStore.getState().regenerateMessage(restored.id, "session-1");
  assert.equal(chatRequests.length, 2);
  assert.deepEqual(chatRequests[1].toolContext, exactContext);
  assert.deepEqual(chatRequests[1].activeSkills, ["stock-review"]);
  assert.deepEqual(chatRequests[1].messages, [
    { role: "user", content: "approved visible prompt" },
  ]);
});
