// @ts-nocheck -- Node runs a Bun probe to exercise the real TSX page.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

function runScenario(scenario) {
  const probe = String.raw`
    import { Window } from "happy-dom";
    import { mock } from "bun:test";
    const dom = new Window();
    for (const key of ["window", "document", "navigator", "HTMLElement", "Element", "Node", "MutationObserver", "localStorage"]) {
      Object.defineProperty(globalThis, key, { configurable: true, value: key === "window" ? dom : dom[key] });
    }
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    const React = await import("react");
    const { act } = React;
    const { createRoot } = await import("react-dom/client");
    const h = React.createElement;
    const pass = ({ children }) => h("div", null, children);
    const messages = [];
    const scenario = ${JSON.stringify(scenario)};
    const file = { name: "options.csv", text: async () => {
      if (scenario === "file-error") throw new Error("文件读取失败");
      return "original CSV";
    } };
    mock.module("antd", () => ({
      Card: pass, Space: pass, Row: pass, Col: pass, Tag: pass,
      Typography: { Title: pass, Text: pass }, Input: () => null, InputNumber: () => null,
      Table: ({ columns = [], dataSource = [], rowKey }) => h("table", null,
        h("thead", null, h("tr", null, columns.map((column, index) => h("th", { key: index }, column.title)))),
        h("tbody", null, dataSource.map(row => h("tr", { key: typeof rowKey === "function" ? rowKey(row) : row[rowKey] },
          columns.map((column, index) => h("td", { key: index }, column.render ? column.render(row[column.dataIndex], row) : row[column.dataIndex])))))),
      Tabs: () => null,
      Modal: ({ open, title, children, onOk, onCancel, okText, cancelText, okButtonProps, cancelButtonProps, confirmLoading }) => open
        ? h("section", { role: "dialog" }, h("h2", null, title), children,
          h("button", { onClick: onCancel, disabled: cancelButtonProps?.disabled }, cancelText),
          h("button", { onClick: onOk, disabled: okButtonProps?.disabled || confirmLoading }, okText)) : null,
      Upload: ({ beforeUpload, disabled }) => h("button", { disabled, onClick: () => beforeUpload(file) }, "上传测试文件"),
      Button: ({ children, onClick, disabled, loading }) => h("button", { onClick, disabled: disabled || loading }, children),
      Select: ({ value, options, onChange }) => h("select", { value: value ?? "", onChange: e => onChange(e.target.value) },
        options.map(option => h("option", { value: option.value, key: option.value }, option.label))),
      Alert: ({ title, message, description }) => h("div", { role: "alert" }, title, message, description),
      message: { success: text => messages.push(text), warning: text => messages.push(text), error: text => messages.push(text) },
    }));
    const noop = () => {};
    mock.module("./src/stores/accountStore", () => ({ useAccountStore: () => ({
      accounts: [{ id: "a", name: "账户 A" }, { id: "b", name: "账户 B" }], fetchAccounts: noop,
    }) }));
    mock.module("./src/stores/optionReviewStore", () => ({ useOptionReviewStore: () => ({
      report: null, fetchOptionReview: noop, clearOptionReview: noop,
    }) }));
    mock.module("./src/components/charts/StatCard", () => ({ default: () => null }));
    const calls = [];
    let resolvePreview, resolveImport;
    dom.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === "get_option_contracts") return [];
      if (command === "preview_options_csv") {
        if (scenario === "preview-error") throw new Error("预览失败");
        if (scenario === "late-preview") await new Promise(resolve => { resolvePreview = resolve; });
        return { total_rows: 5, importable: scenario === "empty" ? 0 : 2, skipped: scenario === "empty" ? 5 : 3, errors: ["Row 4: invalid quantity"],
          rows: scenario === "empty" ? [] : [
            { row_number: 2, option_symbol: "AAPL 18SEP26 100 P", traded_at: "2026-08-01, 09:30:00", action: "SELL", code: "O", quantity: -2, price: 1.2345, amount: 246.9, commission: -0.6789, fee: -0.01 },
            { row_number: 6, option_symbol: "AAPL 18SEP26 100 P", traded_at: null, action: "BUY", code: "C;Ep", quantity: 2, price: 0, amount: 0, commission: 0, fee: 0 },
          ] };
      }
      if (command === "import_options_csv") {
        if (scenario === "import-error") throw new Error("写入失败");
        if (scenario === "double-confirm") await new Promise(resolve => { resolveImport = resolve; });
        return { imported: 2, skipped: 2, errors: ["Row 4: invalid quantity"] };
      }
      throw new Error(command);
    } };
    localStorage.setItem("options_selected_account_id", "a");
    const { default: OptionsPage } = await import("./src/pages/Options/index.tsx");
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(h(OptionsPage)));
    const button = text => [...container.querySelectorAll("button")].find(button => button.textContent === text);
    const click = async text => {
      const target = button(text);
      if (!target || target.disabled) throw new Error("Button unavailable: " + text);
      await act(async () => target.click());
    };
    const snapshot = () => ({ text: container.querySelector('[role="dialog"]')?.textContent ?? "",
      imports: calls.filter(call => call.command === "import_options_csv"),
      headers: [...container.querySelectorAll('[role="dialog"] th')].map(cell => cell.textContent),
      rows: [...container.querySelectorAll('[role="dialog"] tbody tr')].map(row => [...row.querySelectorAll("td")].map(cell => cell.textContent)),
      confirmDisabled: button("确认导入")?.disabled ?? null });
    await click("上传测试文件");
    const before = snapshot();
    if (scenario === "confirm" || scenario === "import-error") await click("确认导入");
    if (scenario === "cancel") await click("取消");
    if (scenario === "account" || scenario === "late-preview") {
      await act(async () => {
        const select = container.querySelector("select");
        select.value = "b"; select.dispatchEvent(new dom.Event("change", { bubbles: true }));
      });
      if (resolvePreview) await act(async () => resolvePreview());
    }
    if (scenario === "double-confirm") {
      const confirm = button("确认导入");
      await act(async () => { confirm.click(); confirm.click(); });
      await act(async () => resolveImport());
    }
    const result = { before, after: snapshot(), calls, messages };
    await act(async () => root.unmount()); await dom.happyDOM.close();
    process.stdout.write(JSON.stringify(result));
  `;
  return JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
}

test("selecting an options CSV shows counts and errors without writing records", () => {
  const { before, calls } = runScenario("preview");
  assert.equal(before.imports.length, 0);
  assert.match(before.text, /识别记录：5 条/);
  assert.match(before.text, /可导入：2 条/);
  assert.match(before.text, /将跳过：3 条/);
  assert.match(before.text, /账户 A/);
  assert.match(before.text, /options.csv/);
  assert.match(before.text, /invalid quantity/);
  assert.deepEqual(calls.find(call => call.command === "preview_options_csv").args, { accountId: "a", csvContent: "original CSV" });
});

test("confirmation imports the previewed file and refreshes option contracts", () => {
  const result = runScenario("confirm");
  assert.equal(result.before.imports.length, 0);
  assert.deepEqual(result.after.imports, [{ command: "import_options_csv", args: { accountId: "a", csvContent: "original CSV" } }]);
  assert.equal(result.after.text, "");
  assert.equal(result.calls.filter(call => call.command === "get_option_contracts").length, 2);
});

test("the confirmation table shows accepted rows with source row numbers and unrounded trade values", () => {
  const { before } = runScenario("preview");
  assert.deepEqual(before.headers, ["CSV 行号", "期权合约", "交易时间", "方向", "合约数量", "价格", "金额", "佣金", "费用", "代码"]);
  assert.deepEqual(before.rows, [
    ["2", "AAPL 18SEP26 100 P", "2026-08-01, 09:30:00", "卖出", "-2", "1.2345", "246.9", "-0.6789", "-0.01", "O"],
    ["6", "AAPL 18SEP26 100 P", "—", "买入", "2", "0", "0", "0", "0", "C;Ep"],
  ]);
  assert.equal(before.imports.length, 0);
});

test("cancelling the preview does not import", () => {
  const result = runScenario("cancel");
  assert.equal(result.after.text, "");
  assert.equal(result.after.imports.length, 0);
});

test("no importable records disables confirmation", () => {
  const result = runScenario("empty");
  assert.equal(result.before.confirmDisabled, true);
  assert.equal(result.after.imports.length, 0);
  assert.deepEqual(result.before.rows, []);
});

for (const scenario of ["account", "late-preview"]) {
  test(`${scenario}: switching accounts discards the old CSV preview`, () => {
    const result = runScenario(scenario);
    assert.equal(result.after.text, "");
    assert.equal(result.after.imports.length, 0);
  });
}

for (const scenario of ["file-error", "preview-error"]) {
  test(`${scenario}: failure does not open confirmation or import`, () => {
    const result = runScenario(scenario);
    assert.equal(result.after.text, "");
    assert.equal(result.after.imports.length, 0);
    assert.equal(result.messages.length, 1);
    assert.match(result.messages[0], /失败/);
  });
}

test("failed imports retain the preview and permit retry", () => {
  const result = runScenario("import-error");
  assert.match(result.after.text, /options.csv/);
  assert.equal(result.after.confirmDisabled, false);
  assert.match(result.messages[0], /写入失败/);
});

test("double confirmation only submits one import", () => {
  const result = runScenario("double-confirm");
  assert.equal(result.after.imports.length, 1);
});
