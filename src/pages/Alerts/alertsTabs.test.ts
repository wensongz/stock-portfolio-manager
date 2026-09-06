// @ts-nocheck -- This test runs directly in Node 26.
import test from "node:test";
import assert from "node:assert/strict";

test("investment alerts tab model keeps portfolio before price with portfolio selected by default", async () => {
  const { buildInvestmentAlertsTabs } = await import("./alertsTabs.ts");

  const tabs = buildInvestmentAlertsTabs({
    portfolioTab: { label: "portfolio-placeholder", children: "portfolio-panel" },
    priceTab: { label: "price-placeholder", children: "price-panel" },
  });

  assert.equal(tabs.defaultActiveKey, "portfolio");
  assert.deepEqual(
    tabs.items.map((item) => item.key),
    ["portfolio", "price"],
  );
  assert.deepEqual(
    tabs.items.map((item) => item.children),
    ["portfolio-panel", "price-panel"],
  );
});
