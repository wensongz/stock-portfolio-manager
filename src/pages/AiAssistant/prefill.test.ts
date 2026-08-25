// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import { readAiPrefill, resolveAiPrefillSessionId } from "./prefill.ts";

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
