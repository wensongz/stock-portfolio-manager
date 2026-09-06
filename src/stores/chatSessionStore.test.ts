// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test, { beforeEach } from "node:test";
import assert from "node:assert/strict";

function deferred() {
  let resolve;
  const promise = new Promise((resolvePromise) => { resolve = resolvePromise; });
  return { promise, resolve };
}

let invokeImpl = () => Promise.reject(new Error("invoke not configured"));
globalThis.window = {
  __TAURI_INTERNALS__: {
    invoke(command, args) {
      return invokeImpl(command, args);
    },
  },
};

const { useChatSessionStore } = await import("./chatSessionStore.ts");

const existingSession = {
  id: "session-existing",
  name: "Existing",
  created_at: "2026-09-06T00:00:00Z",
  updated_at: "2026-09-06T00:00:00Z",
};

const createdSession = {
  id: "session-created",
  name: "新聊天 12:00",
  created_at: "2026-09-06T04:00:00Z",
  updated_at: "2026-09-06T04:00:00Z",
};

beforeEach(() => {
  invokeImpl = () => Promise.reject(new Error("invoke not configured"));
  useChatSessionStore.setState({
    sessions: [existingSession],
    currentSessionId: null,
    selectionRevision: 0,
    loading: false,
    error: null,
  });
});

test("normal create activates its exact result when selection stays unchanged", async () => {
  invokeImpl = async (command) => {
    assert.equal(command, "create_chat_session");
    return createdSession;
  };

  const session = await useChatSessionStore.getState().createSession();

  assert.equal(session.id, "session-created");
  assert.equal(useChatSessionStore.getState().currentSessionId, "session-created");
  assert.equal(useChatSessionStore.getState().selectionRevision, 1);
});

test("late normal create cannot overwrite a newer manual selection", async () => {
  const create = deferred();
  invokeImpl = async () => create.promise;

  const creating = useChatSessionStore.getState().createSession();
  useChatSessionStore.getState().setCurrentSession("session-existing");
  create.resolve(createdSession);
  const session = await creating;

  assert.equal(session.id, "session-created");
  assert.equal(useChatSessionStore.getState().currentSessionId, "session-existing");
  assert.equal(useChatSessionStore.getState().selectionRevision, 1);
  assert.equal(
    useChatSessionStore.getState().sessions.some((row) => row.id === "session-created"),
    true,
  );
});

test("detached create is activated only by an exact selection-revision claim", async () => {
  invokeImpl = async () => createdSession;

  const session = await useChatSessionStore.getState().createDetachedSession();
  assert.equal(useChatSessionStore.getState().currentSessionId, null);
  assert.equal(
    useChatSessionStore.getState().selectSessionIfRevision(0, session.id),
    true,
  );
  assert.equal(useChatSessionStore.getState().currentSessionId, "session-created");
  assert.equal(useChatSessionStore.getState().selectionRevision, 1);
});

test("detached create claim fails after manual selection without clobbering it", async () => {
  const create = deferred();
  invokeImpl = async () => create.promise;

  const creating = useChatSessionStore.getState().createDetachedSession();
  useChatSessionStore.getState().setCurrentSession("session-existing");
  create.resolve(createdSession);
  const session = await creating;

  assert.equal(
    useChatSessionStore.getState().selectSessionIfRevision(0, session.id),
    false,
  );
  assert.equal(useChatSessionStore.getState().currentSessionId, "session-existing");
  assert.equal(useChatSessionStore.getState().selectionRevision, 1);
});

test("a normal create joining a detached request keeps normal activation semantics", async () => {
  const create = deferred();
  invokeImpl = async () => create.promise;

  const detached = useChatSessionStore.getState().createDetachedSession();
  const normal = useChatSessionStore.getState().createSession();
  create.resolve(createdSession);
  await Promise.all([detached, normal]);

  assert.equal(useChatSessionStore.getState().currentSessionId, "session-created");
  assert.equal(useChatSessionStore.getState().selectionRevision, 1);
});
