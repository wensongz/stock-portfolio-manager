// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import { toQuoteWarning } from "./quoteErrors.ts";

test("toQuoteWarning turns a Xueqiu authentication error into an actionable warning", () => {
  assert.equal(
    toQuoteWarning("Xueqiu API error 400016: 重新登录帐号后再试"),
    "雪球 Cookie 已过期，请到 设置 → 通用设置 → 雪球 Cookie 设置 中设置 Cookie。",
  );
});

test("toQuoteWarning treats any Xueqiu response error code as a Cookie warning", () => {
  for (const error of [
    "Xueqiu API error for realtime quotes: HTTP 403 Forbidden. Response: ",
    "Xueqiu API error for AAPL: HTTP 500 Internal Server Error. Response: ",
    "Xueqiu API error for realtime quotes: code=400017, message=Forbidden",
    "Xueqiu API error for AAPL: code=-1, message=Unknown error",
    "Failed to initialize Xueqiu token: HTTP 403 Forbidden",
  ]) {
    assert.equal(
      toQuoteWarning(new Error(error)),
      "雪球 Cookie 已过期，请到 设置 → 通用设置 → 雪球 Cookie 设置 中设置 Cookie。",
      error,
    );
  }
});

test("toQuoteWarning turns other Xueqiu request errors into the service warning", () => {
  for (const error of [
    "Network error fetching AAPL from Xueqiu: timed out",
    "Network error fetching AAPL from Xueqiu: connection refused",
    "Failed to initialize Xueqiu token: error sending request for url (https://xueqiu.com/)",
    "Network error fetching symbol 400016 from Xueqiu",
    "Failed to parse Xueqiu realtime response: expected value",
    "No data from Xueqiu realtime quotes",
  ]) {
    assert.equal(
      toQuoteWarning(new Error(error)),
      "访问雪球行情服务失败，请检查网络连接或稍后重试。",
      error,
    );
  }
});

test("toQuoteWarning preserves useful details for non-Xueqiu quote errors", () => {
  assert.equal(
    toQuoteWarning("database unavailable"),
    "行情获取失败：database unavailable",
  );
});
