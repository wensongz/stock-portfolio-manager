// @ts-nocheck -- Exercise the real page's forms and Tauri payload boundary.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

function addCash(accountMarket, currency) {
  const probe = String.raw`
    const React = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const antd = await import("antd");
    const { mock } = await import("bun:test");
    globalThis.localStorage = { getItem: () => null };
    const account = { id: "mixed", name: "Mixed Broker", market: ${JSON.stringify(accountMarket)}, description: null, created_at: "2026-01-01", updated_at: "2026-01-01" };
    mock.module("./src/stores/accountStore", () => ({ useAccountStore: () => ({ accounts: [account], fetchAccounts: async () => {} }) }));
    const forms = [], fields = [], calls = [], errors = [];
    let submit;
    const Form = ({ children, form, onFinish }) => { if (form === forms[1]) submit = onFinish; return children; };
    Form.Item = ({ name, children }) => { fields.push(name); return children; };
    Form.useForm = () => { const form = { resetFields() {}, setFieldsValue() {}, getFieldValue() {} }; forms.push(form); return [form]; };
    Form.useWatch = () => undefined;
    mock.module("antd", () => ({ ...antd, Form, Modal: ({ title, children }) => title === "添加现金" ? children : null,
      message: { success() {}, error: text => errors.push(text) } }));
    globalThis.window = { __TAURI_INTERNALS__: { invoke: async (command, args) => {
      calls.push({ command, args });
      if (command !== "create_holding") throw new Error("Unexpected command: " + command);
      return { id: "cash", account_id: args.accountId, symbol: args.symbol, name: args.name, market: args.market,
        category_id: args.categoryId ?? null, shares: args.shares, avg_cost: args.avgCost, currency: args.currency,
        created_at: "2026-01-01", updated_at: "2026-01-01" };
    } } };
    const { default: Page } = await import("./src/pages/Holdings/index.tsx");
    renderToStaticMarkup(React.createElement(Page));
    await submit({ accountId: account.id, amount: 7800, currency: ${JSON.stringify(currency)} });
    process.stdout.write(JSON.stringify({ fields, calls, errors }));
  `;
  return JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
}

test("Add Cash offers a currency and records it independently of the account market", () => {
  for (const [accountMarket, currency, expectedMarket] of [["US", "HKD", "HK"], ["HK", "USD", "US"], ["HK", "CNY", "CN"]]) {
    const result = addCash(accountMarket, currency);
    assert.ok(result.fields.includes("currency"), "cash currency must be selectable");
    assert.deepEqual(result.errors, []);
    assert.equal(result.calls.length, 1);
    const { command, args } = result.calls[0];
    assert.equal(command, "create_holding");
    assert.equal(args.accountId, "mixed");
    assert.equal(args.symbol, `$CASH-${currency}`);
    assert.equal(args.currency, currency);
    assert.equal(args.market, expectedMarket);
    assert.equal(args.shares, 7800);
    assert.equal(args.avgCost, 1);
  }
});
