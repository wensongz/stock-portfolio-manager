// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import { toQuoteWarning } from "./quoteErrors.ts";

test("toQuoteWarning turns a Xueqiu authentication error into an actionable warning", () => {
  assert.equal(
    toQuoteWarning("Xueqiu API error 400016: 重新登录帐号后再试"),
    "雪球 Cookie 可能已经过期，请到设置页面更新雪球 Cookie。",
  );
});

test("toQuoteWarning turns other Xueqiu request errors into the service warning", () => {
  assert.equal(
    toQuoteWarning(new Error("Network error fetching AAPL from Xueqiu: timed out")),
    "访问雪球行情服务失败，请检查网络连接或稍后重试。",
  );
});

test("toQuoteWarning preserves useful details for non-Xueqiu quote errors", () => {
  assert.equal(
    toQuoteWarning("database unavailable"),
    "行情获取失败：database unavailable",
  );
});
