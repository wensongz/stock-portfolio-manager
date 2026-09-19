// @ts-nocheck -- This integration test loads the TSX components through Vite.
import assert from "node:assert/strict";
import test, { after, before } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import react from "@vitejs/plugin-react";
import { createServer } from "vite";

let server;
let alertsModule;
let messageCenterModule;
let previousLocalStorageDescriptor;

before(async () => {
  previousLocalStorageDescriptor = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem() { return null; },
      removeItem() {},
      setItem() {},
    },
  });
  server = await createServer({
    appType: "custom",
    cacheDir: "/tmp/stock-portfolio-manager-alert-history-vite-cache",
    configFile: false,
    plugins: [react()],
    root: process.cwd(),
    server: {
      hmr: false,
      middlewareMode: true,
      watch: null,
      ws: false,
    },
  });
  alertsModule = await server.ssrLoadModule("/src/pages/Alerts/index.tsx");
  messageCenterModule = await server.ssrLoadModule(
    "/src/pages/Alerts/AlertMessageCenter.tsx",
  );
});

after(async () => {
  await server?.close();
  if (previousLocalStorageDescriptor) {
    Object.defineProperty(globalThis, "localStorage", previousLocalStorageDescriptor);
  } else {
    delete globalThis.localStorage;
  }
});

test("消息中心按钮渲染在提醒标签栏的附加操作区", () => {
  const html = renderToStaticMarkup(
    React.createElement(
      MemoryRouter,
      null,
      React.createElement(alertsModule.default),
    ),
  );

  assert.match(html, /ant-tabs-extra-content[^>]*>[\s\S]*?<button[^>]*>[\s\S]*?消息中心[\s\S]*?<\/button>/);
  assert.match(html, /组合提醒/);
  assert.match(html, /价格提醒/);
});

test("历史消息详情展示账户名称和可读快照，不展示消息或账户 ID", () => {
  const message = {
    id: "message-secret-id",
    kind: "PORTFOLIO",
    title: "组合仓位偏离",
    message: "成长类资产超过目标区间",
    scopeName: "美股组合",
    accountName: "长期投资账户",
    triggeredAt: "2026-09-19T08:09:10Z",
    details: [
      { label: "当前仓位", value: "72.5%" },
    ],
  };

  const html = renderToStaticMarkup(
    React.createElement(
      React.Fragment,
      null,
      React.createElement(messageCenterModule.AlertHistoryScope, { message }),
      React.createElement(messageCenterModule.AlertHistoryMessageDetails, { message }),
    ),
  );

  assert.match(html, /长期投资账户/);
  assert.match(html, /当前仓位/);
  assert.match(html, /72\.5%/);
  assert.doesNotMatch(html, /message-secret-id|account-secret-id/);
});

test("账户范围已经包含账户名称时不重复显示账户名称", () => {
  const message = {
    id: "message-account-scope",
    kind: "PORTFOLIO",
    title: "组合提醒",
    message: "账户组合发生变化",
    scopeName: "账户：长期投资账户",
    accountName: "长期投资账户",
    triggeredAt: "2026-09-19T08:09:10Z",
    details: [],
  };

  const html = renderToStaticMarkup(
    React.createElement(messageCenterModule.AlertHistoryScope, { message }),
  );

  assert.equal(html.match(/长期投资账户/g)?.length, 1);
});
