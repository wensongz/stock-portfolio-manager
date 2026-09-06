// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  buildPortfolioRebalanceNavigation,
  navigateToPortfolioRebalance,
} from "./portfolioRebalancePrefill.ts";

test("rebalance navigation always targets a new AI session with only a trusted config id", () => {
  const action = buildPortfolioRebalanceNavigation("config-us");

  assert.equal(action.path, "/ai-assistant");
  assert.equal(action.sessionId, null);
  assert.deepEqual(Object.keys(action.state).sort(), [
    "prefillActiveSkill",
    "prefillAutoSend",
    "prefillPrompt",
    "prefillToolArguments",
    "prefillToolName",
  ]);
  assert.equal(action.state.prefillActiveSkill, "portfolio-rebalance");
  assert.equal(action.state.prefillAutoSend, true);
  assert.equal(action.state.prefillToolName, "get_rebalance_context");
  assert.deepEqual(action.state.prefillToolArguments, { config_id: "config-us" });
});

test("rebalance prompt requests a detailed no-added-capital plan without derived values", () => {
  const { prefillPrompt } = buildPortfolioRebalanceNavigation("config-us").state;

  for (const required of ["不追加资金", "增配", "减配", "标的", "约计金额", "调整后"]) {
    assert.match(prefillPrompt, new RegExp(required));
  }
  assert.equal(prefillPrompt.includes("config-us"), false);
  assert.equal(/\d+(?:\.\d+)?\s*(?:%|USD|CNY|HKD|美元|人民币|港元)/.test(prefillPrompt), false);
});

test("rebalance navigation rejects an empty config id", () => {
  assert.throws(() => buildPortfolioRebalanceNavigation("  "), /配置 ID/);
});

test("portfolio button clears the AI session before performing navigation", () => {
  const calls: unknown[] = [];

  navigateToPortfolioRebalance("config-us", {
    setCurrentSession: (sessionId) => calls.push(["session", sessionId]),
    navigate: (path, options) => calls.push(["navigate", path, options]),
  });

  const expectedState = buildPortfolioRebalanceNavigation("config-us").state;
  assert.deepEqual(calls, [
    ["session", null],
    ["navigate", "/ai-assistant", { state: expectedState }],
  ]);
});
