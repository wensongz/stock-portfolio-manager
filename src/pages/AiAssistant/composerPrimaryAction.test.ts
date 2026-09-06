// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import { isComposerPrimaryActionDisabled } from "./composerPrimaryAction.ts";

test("pending disables a manual send before generation begins", () => {
  assert.equal(isComposerPrimaryActionDisabled({
    pending: true,
    sending: false,
    canSend: true,
  }), true);
});

test("sending always leaves the stop action enabled even while pending", () => {
  assert.equal(isComposerPrimaryActionDisabled({
    pending: true,
    sending: true,
    canSend: false,
  }), false);
});

test("ordinary send retains its existing availability rules", () => {
  assert.equal(isComposerPrimaryActionDisabled({
    pending: false,
    sending: false,
    canSend: false,
  }), true);
  assert.equal(isComposerPrimaryActionDisabled({
    pending: false,
    sending: false,
    canSend: true,
  }), false);
});
