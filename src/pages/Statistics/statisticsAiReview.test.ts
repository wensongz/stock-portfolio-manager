// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import { buildStatisticsAiReviewPrefill } from "./statisticsAiReview.ts";

test("overall portfolio review targets all holdings with the selected base currency", () => {
  const prefill = buildStatisticsAiReviewPrefill({
    kind: "overview",
    baseCurrency: "CNY",
  });

  assert.equal(prefill.activeSkill, "munger-perspective");
  assert.equal(prefill.autoSend, false);
  assert.equal(prefill.toolName, "get_portfolio_overview");
  assert.deepEqual(prefill.toolArguments, {});
  assert.match(prefill.prompt, /整个投资组合/);
  assert.match(prefill.prompt, /CNY/);
  assert.match(prefill.prompt, /调仓建议/);
});

test("market portfolio review limits the analysis to the selected market", () => {
  const prefill = buildStatisticsAiReviewPrefill({
    kind: "market",
    market: "HK",
  });

  assert.match(prefill.prompt, /仅复盘港股（HK）/);
  assert.doesNotMatch(prefill.prompt, /整个投资组合/);
  assert.deepEqual(prefill.toolArguments, { market: "HK" });
});

test("account portfolio review carries both the account name and stable id", () => {
  const prefill = buildStatisticsAiReviewPrefill({
    kind: "account",
    accountId: "account-a",
    accountName: "长期账户",
  });

  assert.match(prefill.prompt, /仅复盘账户「长期账户」/);
  assert.match(prefill.prompt, /account-a/);
  assert.match(prefill.prompt, /目标仓位/);
  assert.deepEqual(prefill.toolArguments, { account_id: "account-a" });
});
