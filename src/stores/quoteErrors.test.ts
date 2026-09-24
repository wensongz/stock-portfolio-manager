// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import { toQuoteWarning } from "./quoteErrors.ts";

test("toQuoteWarning reports the offending CN symbol without blaming the network", () => {
  for (const error of [
    "Unknown CN market prefix '60' in symbol 601069 for Xueqiu",
    "Invalid CN symbol for Xueqiu: 601069",
    "Unknown CN market prefix '60' in symbol 601069 for Xueqiu; fallback failed: Unknown CN market prefix '60' in symbol 601069",
  ]) {
    assert.equal(
      toQuoteWarning(new Error(error)),
      "A 股代码「601069」格式不正确，请添加 sh 或 sz 前缀（如 sh601069、sz000858）。",
      error,
    );
  }
});

test("toQuoteWarning gives a Cookie hint only for the exact authentication code", () => {
  for (const error of [
    "Xueqiu API error 400016: 重新登录帐号后再试",
    "Xueqiu API error for AAPL: code=400016, message=重新登录帐号后再试",
    'Xueqiu API error for realtime quotes: HTTP 400 Bad Request. Response: {"error_code":400016,"error_description":"重新登录帐号后再试"}',
    'Xueqiu API error for realtime quotes: HTTP 400 Bad Request. Response: {"error_code":"400016","description":"重新登录帐号后再试"}',
  ]) {
    assert.equal(
      toQuoteWarning(error),
      "雪球 Cookie 已过期，请到 设置 → 通用设置 → 雪球 Cookie 设置 中设置 Cookie。",
      error,
    );
  }
});

test("toQuoteWarning preserves other business error codes and descriptions", () => {
  for (const [error, expected] of [
    ["Xueqiu API error for realtime quotes: code=400017, message=Forbidden", "雪球行情服务返回错误：Forbidden（错误码：400017）"],
    ["Xueqiu API error for AAPL: code=-1, message=Unknown error", "雪球行情服务返回错误：Unknown error（错误码：-1）"],
    ["Xueqiu API error for AAPL: code=-1, message=", "雪球行情服务返回错误：未知错误（错误码：-1）"],
    ['Xueqiu API error for AAPL: HTTP 400 Bad Request. Response: {"error_code":123,"error_description":"股票不存在"}', "雪球行情服务返回错误：股票不存在（错误码：123）"],
    ['Xueqiu API error for AAPL: HTTP 400 Bad Request. Response: {"error_code":"123","description":"股票不存在"}', "雪球行情服务返回错误：股票不存在（错误码：123）"],
    ['Xueqiu API error for AAPL: HTTP 400 Bad Request. Response: {"error_code":123}', "雪球行情服务返回错误：未知错误（错误码：123）"],
  ]) {
    assert.equal(toQuoteWarning(new Error(error)), expected, error);
  }
});

test("toQuoteWarning does not mistake a symbol or description for an authentication code", () => {
  for (const error of [
    "Xueqiu API error for 400016: code=123, message=不能访问 400016",
    "Xueqiu API error for AAPL: code=123, message=参考错误码 400016",
    "Xueqiu API error for AAPL: code=1400016, message=股票不存在",
    'Xueqiu API error for 400016: HTTP 400 Bad Request. Response: {"error_code":123,"description":"参考 400016"}',
  ]) {
    assert.match(toQuoteWarning(error), /^雪球行情服务返回错误：/);
    assert.doesNotMatch(toQuoteWarning(error), /Cookie/);
  }
});

test("toQuoteWarning preserves HTTP status when no business error is available", () => {
  for (const [error, expected] of [
    ["Xueqiu API error for realtime quotes: HTTP 403 Forbidden. Response: ", "雪球行情服务返回 HTTP 403，请稍后重试。"],
    ["Xueqiu API error for AAPL: HTTP 429 Too Many Requests. Response: <html>rate limited</html>", "雪球行情服务返回 HTTP 429，请稍后重试。"],
    ["Xueqiu API error for AAPL: HTTP 500 Internal Server Error. Response: ", "雪球行情服务返回 HTTP 500，请稍后重试。"],
    ['Xueqiu API error for AAPL: HTTP 500 Internal Server Error. Response: {"error_code":0}', "雪球行情服务返回 HTTP 500，请稍后重试。"],
    ['Xueqiu API error for AAPL: HTTP 400 Bad Request. Response: {"error_code":"E_STOCK","description":"股票不存在"}', "雪球行情服务返回 HTTP 400，请稍后重试。"],
    ["Failed to initialize Xueqiu token: HTTP 403 Forbidden", "雪球行情服务返回 HTTP 403，请稍后重试。"],
  ]) {
    assert.equal(toQuoteWarning(error), expected, error);
  }
});

test("toQuoteWarning separates malformed responses from connectivity failures", () => {
  for (const error of [
    "Failed to parse Xueqiu realtime response: expected value",
    "Failed to parse Xueqiu response for AAPL: expected value. Response preview: <html>error</html>",
  ]) {
    assert.equal(toQuoteWarning(error), "雪球返回的行情数据格式异常，请稍后重试。", error);
  }
});

test("toQuoteWarning identifies incomplete quotes", () => {
  for (const error of [
    "No data from Xueqiu realtime quotes",
    "No quote data from Xueqiu for AAPL",
    "Missing stock name in Xueqiu response for AAPL",
    "Missing current price in Xueqiu response for AAPL",
    "Xueqiu realtime response omitted a symbol",
  ]) {
    assert.equal(toQuoteWarning(error), "雪球未返回完整的股票行情，请检查股票代码和市场。", error);
  }
});

test("toQuoteWarning identifies invalid local symbols", () => {
  for (const error of [
    "Invalid HK symbol for Xueqiu: AAPL",
    "Invalid symbol for Xueqiu realtime request: A,APL",
    "Xueqiu realtime symbol normalization failed",
  ]) {
    assert.equal(toQuoteWarning(error), "股票代码格式不正确，请检查股票代码和市场。", error);
  }
});

test("toQuoteWarning uses the network hint for transport failures", () => {
  for (const error of [
    "Network error fetching AAPL from Xueqiu: timed out",
    "Network error fetching AAPL from Xueqiu: connection refused",
    "Failed to initialize Xueqiu token: error sending request for url (https://xueqiu.com/)",
    "Network error fetching symbol 400016 from Xueqiu",
    "Failed to read Xueqiu response body for AAPL: connection reset",
  ]) {
    assert.equal(
      toQuoteWarning(new Error(error)),
      "访问雪球行情服务失败，请检查网络连接或稍后重试。",
      error,
    );
  }
});

test("toQuoteWarning keeps fallback errors from changing the primary failure classification", () => {
  assert.equal(
    toQuoteWarning("Network error fetching AAPL from Xueqiu: timed out; fallback failed: Xueqiu API error for AAPL: code=400016, message=重新登录帐号后再试"),
    "访问雪球行情服务失败，请检查网络连接或稍后重试。",
  );
  assert.equal(
    toQuoteWarning("Xueqiu API error for AAPL: code=123, message=股票不存在; fallback failed: unavailable"),
    "雪球行情服务返回错误：股票不存在（错误码：123）",
  );
});

test("toQuoteWarning retains unrecognized Xueqiu error details", () => {
  assert.equal(
    toQuoteWarning("Unexpected Xueqiu error: diagnostic detail"),
    "雪球行情服务错误：Unexpected Xueqiu error: diagnostic detail",
  );
});

test("toQuoteWarning preserves useful details for non-Xueqiu quote errors", () => {
  assert.equal(
    toQuoteWarning("database unavailable"),
    "行情获取失败：database unavailable",
  );
});
