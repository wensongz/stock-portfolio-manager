// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";

test("option review errors retain the account and period request identity", async () => {
  globalThis.window = {
    __TAURI_INTERNALS__: {
      invoke(command: string, args: { accountId: string; periodDays: number | null }) {
        assert.equal(command, "get_option_review");
        return Promise.reject(
          new Error(`${args.accountId}:${args.periodDays ?? "all"}`),
        );
      },
    },
  };

  const { useOptionReviewStore } = await import("./optionReviewStore.ts");
  useOptionReviewStore.getState().clearOptionReview();

  await useOptionReviewStore.getState().fetchOptionReview("account-a", 365);
  assert.equal(useOptionReviewStore.getState().requestedAccountId, "account-a");
  assert.equal(useOptionReviewStore.getState().requestedPeriodDays, 365);

  await useOptionReviewStore.getState().fetchOptionReview("account-b", null);
  assert.equal(useOptionReviewStore.getState().requestedAccountId, "account-b");
  assert.equal(useOptionReviewStore.getState().requestedPeriodDays, null);
  assert.match(useOptionReviewStore.getState().error, /account-b:all/);
});
