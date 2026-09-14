// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import { toRecords } from "./persistence.ts";

test("persisted reasoning distinguishes absent, empty, and whitespace-only assistant values", () => {
  const records = toRecords("session", [
    { id: "absent", role: "assistant", content: "answer", createdAt: 0 },
    { id: "empty", role: "assistant", content: "answer", createdAt: 1, reasoning: "" },
    { id: "space", role: "assistant", content: "answer", createdAt: 2, reasoning: " \t\n" },
  ]);

  assert.equal(Object.hasOwn(records[0], "reasoning"), false);
  assert.equal(records[1].reasoning, "");
  assert.equal(records[2].reasoning, " \t\n");
});

test("user and system messages do not persist assistant reasoning metadata", () => {
  const records = toRecords("session", [
    { id: "user", role: "user", content: "question", createdAt: 0, reasoning: "user value" },
    { id: "system", role: "system", content: "instruction", createdAt: 1, reasoning: "system value" },
  ]);

  assert.equal(Object.hasOwn(records[0], "reasoning"), false);
  assert.equal(Object.hasOwn(records[1], "reasoning"), false);
});
