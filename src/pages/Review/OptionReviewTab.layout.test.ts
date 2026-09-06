// @ts-nocheck -- This integration test loads the TSX component through Vite.
import assert from "node:assert/strict";
import test, { after, before } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import react from "@vitejs/plugin-react";
import { createServer } from "vite";

let server;
let optionReviewModule;
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
    cacheDir: "/tmp/stock-portfolio-manager-option-review-vite-cache",
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
  optionReviewModule = await server.ssrLoadModule(
    "/src/pages/Review/OptionReviewTab.tsx",
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

test("review sections keep a stable small gap around Ant Design components", () => {
  const html = renderToStaticMarkup(
    React.createElement(
      MemoryRouter,
      null,
      React.createElement(optionReviewModule.default),
    ),
  );

  assert.match(html, /class="flex flex-col gap-4"/);
  assert.doesNotMatch(html, /class="space-y-4"/);
});
