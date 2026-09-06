// @ts-nocheck -- This integration test loads the TSX component through Vite.
import assert from "node:assert/strict";
import test, { after, before } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import react from "@vitejs/plugin-react";
import { createServer } from "vite";

let server;
let portfolioAlertsModule;
let previousLocalStorageDescriptor;

before(async () => {
  previousLocalStorageDescriptor = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      clear() {},
      getItem() { return null; },
      removeItem() {},
      setItem() {},
    },
  });

  server = await createServer({
    appType: "custom",
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
  portfolioAlertsModule = await server.ssrLoadModule(
    "/src/pages/Alerts/PortfolioAlertsTab.tsx",
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

test("threshold percentage controls stay vertically aligned in compact horizontal rows", () => {
  const html = renderToStaticMarkup(
    React.createElement(
      MemoryRouter,
      null,
      React.createElement(portfolioAlertsModule.default),
    ),
  );

  assert.equal((html.match(/flex flex-wrap items-center gap-x-4 gap-y-2/g) ?? []).length, 2);
  assert.equal((html.match(/w-56/g) ?? []).length, 2);
  assert.doesNotMatch(html, /margin-top:8px/);
});

test("evaluation table uses the full content row without a duplicate allocation chart", () => {
  const html = renderToStaticMarkup(
    React.createElement(
      MemoryRouter,
      null,
      React.createElement(portfolioAlertsModule.default),
    ),
  );

  assert.doesNotMatch(html, /当前类别占比/);
  assert.match(html, /目标、当前与再平衡/);
  for (const heading of ["投资类别", "目标占比", "当前占比", "相对偏离", "当前金额", "目标金额", "再平衡金额", "状态"]) {
    assert.match(html, new RegExp(heading));
  }
});

test("page sections use a stable flex gap around Ant Design components", () => {
  const html = renderToStaticMarkup(
    React.createElement(
      MemoryRouter,
      null,
      React.createElement(portfolioAlertsModule.default),
    ),
  );

  assert.match(html, /class="flex flex-col gap-3"/);
  assert.doesNotMatch(html, /class="space-y-4"/);
});

test("status banner keeps its label and explanation in a compact alert", () => {
  assert.equal(typeof portfolioAlertsModule.PortfolioAlertStatusBanner, "function");

  const html = renderToStaticMarkup(
    React.createElement(portfolioAlertsModule.PortfolioAlertStatusBanner, {
      banner: "当前组合存在需要处理的配置偏离。",
      statusColor: "error",
      statusLabel: "需要调整",
    }),
  );

  assert.match(html, /需要调整/);
  assert.match(html, /当前组合存在需要处理的配置偏离/);
  assert.doesNotMatch(html, /ant-alert-with-description/);
});

test("target allocation editor renders four cards in one wide-screen row", () => {
  assert.equal(typeof portfolioAlertsModule.PortfolioAlertTargetEditor, "function");

  const rows = Array.from({ length: 4 }, (_, index) => ({
    categoryId: `category-${index}`,
    color: "#1677ff",
    icon: "📈",
    key: `category-${index}`,
    name: `类别 ${index + 1}`,
    targetPercent: 25,
  }));
  const html = renderToStaticMarkup(
    React.createElement(portfolioAlertsModule.PortfolioAlertTargetEditor, {
      onTargetChange() {},
      rows,
      targetErrors: {},
    }),
  );

  assert.match(html, /grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-4/);
  assert.equal((html.match(/rounded-lg border border-slate-200/g) ?? []).length, 4);
});
