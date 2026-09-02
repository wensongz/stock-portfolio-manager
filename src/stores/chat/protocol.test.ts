// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  buildHistory,
  normalizeToolCallEvent,
} from "./protocol.ts";
import {
  finalizeStreamMessages,
  updateMessageById,
} from "./streamReducer.ts";

const message = (id, role, content, extra = {}) => ({
  id,
  role,
  content,
  createdAt: 0,
  ...extra,
});

test("outbound protocol removes invalid rows and restores role alternation", () => {
  const history = buildHistory([
    message("u1", "user", "first"),
    message("u2", "user", "second"),
    message("bad", "assistant", "", { error: "failed" }),
    message("a1", "assistant", "old"),
    message("a2", "assistant", "new"),
    message("tail", "assistant", "stray"),
  ]);

  assert.deepEqual(history, [{ role: "user", content: "first\n\nsecond" }]);
});

test("model tool calls cannot collide with the reserved host-prefill id", () => {
  assert.deepEqual(
    normalizeToolCallEvent({
      id: "prefilled-stock-review",
      name: "get_transactions",
      arguments: "{}",
      status: "running",
    }),
    {
      id: "model:prefilled-stock-review",
      name: "get_transactions",
      arguments: "{}",
      status: "running",
      origin: "model",
    },
  );
});

test("stream reducers update one row and separate visible from persistable rows", () => {
  const rows = [
    message("u", "user", "question"),
    message("a", "assistant", "partial"),
  ];
  const updated = updateMessageById(rows, "a", (row) => ({
    ...row,
    content: `${row.content} answer`,
  }));
  assert.equal(updated[1].content, "partial answer");
  assert.equal(rows[1].content, "partial", "updates must remain immutable");

  const finalized = finalizeStreamMessages([
    ...updated,
    message("empty", "assistant", ""),
    message("error", "assistant", "", { error: "boom" }),
  ]);
  assert.deepEqual(finalized.visible.map((row) => row.id), ["u", "a", "error"]);
  assert.deepEqual(finalized.persistable.map((row) => row.id), ["u", "a"]);
});
