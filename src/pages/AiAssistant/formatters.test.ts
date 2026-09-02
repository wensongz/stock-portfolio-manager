// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  formatRelativeTime,
  formatTime,
  statusPlaceholder,
  toolLabel,
} from "./formatters.ts";

test("AI message formatters preserve user-facing labels", () => {
  const localTime = new Date(2026, 8, 2, 7, 5).getTime();

  assert.equal(formatTime(localTime), "07:05");
  assert.equal(toolLabel("get_market_overview"), "大盘总览");
  assert.equal(toolLabel("future_tool"), "future_tool");
});

test("relative session time covers recent and dated timestamps", () => {
  const now = new Date(2026, 8, 2, 12, 0).getTime();

  assert.equal(formatRelativeTime(new Date(now - 30_000).toISOString(), now), "刚刚");
  assert.equal(formatRelativeTime(new Date(now - 5 * 60_000).toISOString(), now), "5 分钟前");
  assert.equal(formatRelativeTime(new Date(now - 3 * 3_600_000).toISOString(), now), "3 小时前");
  assert.equal(formatRelativeTime(new Date(now - 24 * 3_600_000).toISOString(), now), "昨天");
  assert.equal(formatRelativeTime("not-a-date", now), "");
});

test("streaming placeholder reflects the current assistant activity", () => {
  assert.equal(
    statusPlaceholder({ toolCalls: [{ status: "running" }] }),
    "正在查询数据…",
  );
  assert.equal(statusPlaceholder({ reasoning: "thinking" }), "正在思考…");
  assert.equal(statusPlaceholder({}), "思考中…");
});
