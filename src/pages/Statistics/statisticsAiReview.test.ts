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

for (const scenario of [
  { market: "CN", name: "A股", currency: "CNY", currencyName: "人民币", otherCurrencies: /USD|HKD|美元|港元/ },
  { market: "HK", name: "港股", currency: "HKD", currencyName: "港元", otherCurrencies: /USD|CNY|美元|人民币/ },
  { market: "US", name: "美股", currency: "USD", currencyName: "美元", otherCurrencies: /CNY|HKD|人民币|港元/ },
]) {
  test(`${scenario.market} portfolio review uses its market currency and excludes other markets`, () => {
    const prefill = buildStatisticsAiReviewPrefill({
      kind: "market",
      market: scenario.market,
    });

    assert.match(prefill.prompt, new RegExp(`仅复盘${scenario.name}（${scenario.market}）`));
    assert.match(prefill.prompt, /忽略其他市场/);
    assert.match(prefill.prompt, new RegExp(`${scenario.currencyName}（${scenario.currency}）.*计价单位`));
    assert.doesNotMatch(prefill.prompt, scenario.otherCurrencies);
    assert.doesNotMatch(prefill.prompt, /整个投资组合/);
    assert.equal(prefill.activeSkill, "munger-perspective");
    assert.equal(prefill.autoSend, false);
    assert.equal(prefill.toolName, "get_portfolio_overview");
    assert.deepEqual(prefill.toolArguments, { market: scenario.market });
    assert.match(prefill.prompt, /芒格视角/);
    assert.match(prefill.prompt, /持仓集中度、能力圈、护城河、估值纪律、认知偏误与永久损失风险/);
    assert.match(prefill.prompt, /调仓建议、建议目标仓位和执行条件/);
  });
}

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
