// @ts-nocheck -- Node drives a Bun SSR probe against the actual page and stores.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

function exerciseHoldingForm(cases) {
  const probe = String.raw`
    const React = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const antd = await import("antd");
    const ActualButton = antd.Button;
    const { mock } = await import("bun:test");
    globalThis.localStorage = { getItem: () => null };
    let fields, mainForm, formCalls, symbolField, onFinish, openModal;
    const makeForm = () => ({
      resetFields() { fields = {}; },
      setFieldsValue(values) { Object.assign(fields, values); },
      getFieldValue(name) { return fields[name]; },
    });
    const Form = ({ children, form, onFinish: submit }) => {
      if (form === mainForm) onFinish = submit;
      return children;
    };
    Form.Item = (props) => {
      if (props.name === "symbol") symbolField = props;
      return props.children;
    };
    Form.useForm = () => [formCalls++ === 0 ? mainForm : makeForm()];
    Form.useWatch = (name, form) => form.getFieldValue(name);
    const errors = [];
    mock.module("antd", () => ({
      ...antd,
      Form,
      Modal: ({ title, children }) => title === "新增持仓" ? children : null,
      Button: (props) => {
        if (props.children === "新增持仓") openModal = props.onClick;
        return React.createElement(ActualButton, props);
      },
      message: { success() {}, error: (text) => errors.push(text) },
    }));
    let commands;
    const timestamp = "2026-09-24T02:00:00.000Z";
    globalThis.window = { __TAURI_INTERNALS__: { invoke: async (command, args) => {
      commands.push({ command, args });
      if (["get_cn_quote", "get_us_quote", "get_hk_quote"].includes(command)) {
        return { data: {
          symbol: args.symbol, name: "查得的股票名称", market: fields.market,
          current_price: 10, previous_close: 10, change: 0, change_percent: 0,
          high: 10, low: 10, volume: 100, updated_at: timestamp,
        }, warning: null, refreshedAt: timestamp };
      }
      if (command === "create_holding") {
        return {
          id: "new-holding", account_id: args.accountId, symbol: args.symbol,
          name: args.name, market: args.market, category_id: args.categoryId ?? null,
          shares: args.shares, avg_cost: args.avgCost, currency: args.currency,
          created_at: timestamp, updated_at: timestamp,
        };
      }
      throw new Error("Unexpected command: " + command);
    } } };
    const { useHoldingStore } = await import("./src/stores/holdingStore.ts");
    const { default: HoldingsPage } = await import("./src/pages/Holdings/index.tsx");
    const results = [];
    for (const scenario of ${JSON.stringify(cases)}) {
      fields = {};
      commands = [];
      errors.length = 0;
      formCalls = 0;
      mainForm = makeForm();
      useHoldingStore.setState({ holdings: [] });
      renderToStaticMarkup(React.createElement(HoldingsPage));
      openModal();
      mainForm.setFieldsValue({
        accountId: "account-test", name: "手动填写的名称", shares: 100, avgCost: 10,
        currency: { CN: "CNY", US: "USD", HK: "HKD" }[scenario.market],
        market: scenario.market, symbol: scenario.symbol,
      });
      if (scenario.action === "type") {
        // Invoke the real Form.Item normalization callback at the Ant Design boundary.
        fields.symbol = symbolField.normalize?.(scenario.symbol, "", fields) ?? scenario.symbol;
      } else if (scenario.action === "blur") {
        symbolField.children.props.onBlur();
        await new Promise((resolve) => setImmediate(resolve));
      } else {
        // Bypass blur and field normalization to exercise the submit guard itself.
        await onFinish({ ...fields });
      }
      results.push({ fields: { ...fields }, commands, errors: [...errors], holdings: useHoldingStore.getState().holdings });
    }
    process.stdout.write(JSON.stringify(results));
  `;
  return JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
}

const cnSymbols = [
  { symbol: "601069", expected: "sh601069" },
  { symbol: "603019", expected: "sh603019" },
  { symbol: "605117", expected: "sh605117" },
  { symbol: "900901", expected: "sh900901" },
  { symbol: "510300", expected: "sh510300" },
  { symbol: "588000", expected: "sh588000" },
  { symbol: "000001", expected: "sz000001" },
  { symbol: "001001", expected: "sz001001" },
  { symbol: "003001", expected: "sz003001" },
  { symbol: "004001", expected: "sz004001" },
  { symbol: "300750", expected: "sz300750" },
  { symbol: "200002", expected: "sz200002" },
  { symbol: "159915", expected: "sz159915" },
  { symbol: "161725", expected: "sz161725" },
];

test("the Add Holding field completes mainland stock and fund symbols immediately", () => {
  const results = exerciseHoldingForm(cnSymbols.map(({ symbol }) => ({ symbol, market: "CN", action: "type" })));
  results.forEach((result, index) => {
    assert.equal(result.fields.symbol, cnSymbols[index].expected);
    assert.deepEqual(result.commands, [], "typing must not request a quote or create a holding");
  });
});

test("blurring a new mainland stock or fund symbol updates the field and looks up its prefixed symbol", () => {
  const results = exerciseHoldingForm(cnSymbols.map(({ symbol }) => ({ symbol, market: "CN", action: "blur" })));
  results.forEach((result, index) => {
    assert.deepEqual(result.commands, [{ command: "get_cn_quote", args: { symbol: cnSymbols[index].expected } }]);
    assert.equal(result.fields.symbol, cnSymbols[index].expected);
    assert.equal(result.fields.name, "查得的股票名称");
    assert.deepEqual(result.errors, []);
  });
});

test("submitting a new mainland stock or fund holding normalizes the stored symbol even without blur", () => {
  const results = exerciseHoldingForm(cnSymbols.map(({ symbol }) => ({ symbol, market: "CN", action: "submit" })));
  results.forEach((result, index) => {
    assert.equal(result.commands.length, 1);
    assert.equal(result.commands[0].command, "create_holding");
    assert.equal(result.commands[0].args.symbol, cnSymbols[index].expected);
    assert.equal(result.commands[0].args.market, "CN");
    assert.equal(result.holdings[0].symbol, cnSymbols[index].expected);
    assert.deepEqual(result.errors, []);
  });
});

test("unrecognized mainland prefixes stay unchanged during entry and creation", () => {
  const symbols = ["302001", "699999", "920001"];
  const results = exerciseHoldingForm(symbols.flatMap((symbol) => [
    { symbol, market: "CN", action: "type" },
    { symbol, market: "CN", action: "submit" },
  ]));
  symbols.forEach((symbol, index) => {
    const typed = results[index * 2];
    const saved = results[index * 2 + 1];
    assert.equal(typed.fields.symbol, symbol);
    assert.equal(saved.commands[0].args.symbol, symbol);
    assert.equal(saved.holdings[0].symbol, symbol);
    assert.deepEqual(saved.errors, []);
  });
});

test("prefixed, US, and HK symbols remain unchanged during entry, lookup, and creation", () => {
  const scenarios = [
    { symbol: "sh601069", market: "CN", command: "get_cn_quote" },
    { symbol: "sz000001", market: "CN", command: "get_cn_quote" },
    { symbol: "601069", market: "US", command: "get_us_quote" },
    { symbol: "AAPL", market: "US", command: "get_us_quote" },
    { symbol: "601069", market: "HK", command: "get_hk_quote" },
    { symbol: "0700.HK", market: "HK", command: "get_hk_quote" },
  ];
  const results = exerciseHoldingForm(scenarios.flatMap(({ symbol, market }) => [
    { symbol, market, action: "type" },
    { symbol, market, action: "blur" },
    { symbol, market, action: "submit" },
  ]));
  scenarios.forEach((scenario, index) => {
    const typed = results[index * 3];
    const lookup = results[index * 3 + 1];
    const saved = results[index * 3 + 2];
    assert.equal(typed.fields.symbol, scenario.symbol);
    assert.deepEqual(lookup.commands, [{ command: scenario.command, args: { symbol: scenario.symbol } }]);
    assert.equal(lookup.fields.symbol, scenario.symbol);
    assert.equal(saved.commands[0].args.symbol, scenario.symbol);
    assert.equal(saved.holdings[0].symbol, scenario.symbol);
    assert.deepEqual(saved.errors, []);
  });
});
