// @ts-nocheck -- Node drives a browser DOM probe through Bun.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../", import.meta.url));

test("selecting a base currency updates other selectors and persists across remounts", () => {
  const probe = String.raw`
    import { Window } from "happy-dom";
    const dom = new Window();
    for (const key of ["window", "document", "navigator", "HTMLElement", "SVGElement", "Element", "Node", "ShadowRoot", "MutationObserver", "ResizeObserver", "localStorage"]) {
      Object.defineProperty(globalThis, key, { configurable: true, value: key === "window" ? dom : dom[key] });
    }
    globalThis.getComputedStyle = dom.getComputedStyle.bind(dom);
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    dom.localStorage.setItem("base_currency", "CNY");
    const React = await import("react");
    const { act } = React;
    const { createRoot } = await import("react-dom/client");
    const { default: BaseCurrencySelect } = await import("./src/components/BaseCurrencySelect.tsx");
    const { useExchangeRateStore } = await import("./src/stores/exchangeRateStore.ts");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const h = React.createElement;
    await act(async () => root.render(h(React.Fragment, null, h(BaseCurrencySelect), h(BaseCurrencySelect))));
    const before = container.textContent;
    const input = container.querySelector("input[role=combobox]");
    await act(async () => input.dispatchEvent(new dom.MouseEvent("mousedown", { bubbles: true })));
    const option = [...document.querySelectorAll(".ant-select-item-option")].find(item => item.textContent === "HKD 港元");
    if (!option) throw new Error("HKD option did not open");
    await act(async () => option.click());
    const selections = [...container.querySelectorAll(".ant-select")].map(select => select.textContent);
    const currency = useExchangeRateStore.getState().baseCurrency;
    const saved = dom.localStorage.getItem("base_currency");
    await act(async () => root.render(null));
    await act(async () => root.render(h(BaseCurrencySelect)));
    const remounted = container.textContent;
    await act(async () => root.unmount());
    await dom.happyDOM.close();
    // End this isolated probe after unmount; UI library timers can outlive the DOM.
    process.stdout.write(JSON.stringify({ before, selections, currency, saved, remounted }), () => process.exit(0));
  `;
  const result = JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8", timeout: 15000 }));
  assert.match(result.before, /CNY 人民币/);
  assert.equal(result.currency, "HKD");
  assert.equal(result.saved, "HKD");
  assert.deepEqual(result.selections, ["HKD 港元", "HKD 港元"]);
  assert.match(result.remounted, /HKD 港元/);
});
