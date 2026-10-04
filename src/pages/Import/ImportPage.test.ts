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
    for (const key of ["window", "document", "navigator", "HTMLElement", "Element", "Node", "MutationObserver", "File", "FileReader"]) {
      Object.defineProperty(globalThis, key, { configurable: true, value: key === "window" ? dom : dom[key] });
    }
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    const React = await import("react");
    const { act } = React;
    const { createRoot } = await import("react-dom/client");
    const h = React.createElement;
    const pass = ({ children }) => h("div", null, children);
    const Form = Object.assign(pass, { Item: pass });
    const messages = [];
    mock.module("antd", () => ({
      Card: pass, Space: pass, Typography: { Title: pass, Text: pass }, Tag: pass, Divider: pass, Form,
      Modal: ({ open, title, children, footer }) => open ? h("section", { role: "dialog" }, title, children, footer) : null,
      Button: ({ children, onClick, disabled, loading }) => h("button", { onClick, disabled: disabled || loading }, children),
      Select: ({ value, options = [], onChange, disabled, placeholder }) => h("select", { value: value ?? "", disabled, onChange: e => onChange(e.target.value) },
        h("option", { value: "" }, placeholder ?? "请选择"), options.map(option => h("option", { value: option.value, key: option.value }, option.label))),
      Upload: { Dragger: ({ beforeUpload, disabled }) => h("button", { disabled, onClick: () => beforeUpload(new File(["example"], "file.csv")) }, "上传测试文件") },
      Alert: ({ title, message, description }) => h("div", { role: "alert" }, title, message, description),
      Steps: ({ current }) => h("output", { "data-step": true }, current),
      Table: () => null,
      message: { success: text => messages.push(text), warning() {}, error() {} },
    }));
    mock.module("./src/stores/accountStore", () => ({ useAccountStore: () => ({ accounts: [{ id: "a", name: "Account" }], fetchAccounts() {} }) }));
    mock.module("./src/features/imports/ImportBatchPanel", () => ({ default: () => h("div", null, "批次已创建") }));
    mock.module("./src/features/imports/ImportBatchHistory", () => ({ default: () => null }));
    const scenario = ${JSON.stringify(scenario)};
    const calls = [];
    dom.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === "parse_import_csv" || command === "parse_options_csv") {
        if (scenario === "parse-error") throw new Error("缺少证券代码列");
        return { total_rows: 2, valid_rows: 1, error_rows: scenario.startsWith("options-") ? [] : [{ row: 3, column: "shares", message: "数量无效" }], preview_data: [{ symbol: "AAPL" }], column_mapping: {} };
      }
      if (command === "preview_csv_import_batch") return { id: "b", rows: [] };
      if (command === "import_options_csv") return { imported: 1, skipped: 0, errors: ["第 3 行：期权日期无效"] };
      throw new Error(command);
    } };
    const { default: ImportPage } = await import("./src/pages/Import/index.tsx");
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(h(ImportPage)));
    const click = async text => {
      const button = [...container.querySelectorAll("button")].find(button => button.textContent.includes(text));
      if (!button || button.disabled) throw new Error("Button unavailable: " + text);
      await act(async () => { button.click(); await dom.happyDOM.whenAsyncComplete(); });
    };
    const select = async (label, value) => {
      const element = [...container.querySelectorAll("select")].findLast(element => [...element.options].some(option => option.textContent === label));
      await act(async () => { element.value = value; element.dispatchEvent(new dom.Event("change", { bubbles: true })); });
    };
    if (scenario.startsWith("options-")) await select("期权记录", "options");
    await click("上传测试文件");
    const before = container.textContent;
    let checkDisabled;
    if (scenario === "partial-parse") {
      await select("Account", "a");
      checkDisabled = [...container.querySelectorAll("button")].find(button => button.textContent.includes("创建批次"))?.disabled;
      if (container.textContent.includes("跳过错误行，继续检查")) {
        await click("跳过错误行，继续检查");
        await click("创建批次");
      }
    } else if (scenario.startsWith("options-")) {
      await select("Account", "a");
      await click("确认导入");
      if (scenario === "options-recovery" && container.textContent.includes("仅重新上传失败记录")) {
        await click("仅重新上传失败记录");
        await click("上传测试文件");
        checkDisabled = [...container.querySelectorAll("button")].find(button => button.textContent.includes("确认导入"))?.disabled;
      }
    }
    const result = { before, after: container.textContent, checkDisabled, calls, messages };
    await act(async () => root.unmount()); await dom.happyDOM.close();
    process.stdout.write(JSON.stringify(result));
  `;
  return JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
}

test("generic CSV parsing requires explicit consent to skip invalid rows", () => {
  const result = runScenario("partial-parse");
  assert.match(result.before, /解析发现错误，如何处理/);
  assert.match(result.before, /数量无效/);
  assert.equal(result.checkDisabled, true);
  assert.equal(result.calls.filter(call => call.command === "preview_csv_import_batch").length, 1);
});

test("generic CSV file errors offer correction instead of only a toast", () => {
  const result = runScenario("parse-error");
  assert.match(result.after, /解析未完成，如何处理/);
  assert.match(result.after, /缺少证券代码列/);
  assert.match(result.after, /重新选择文件/);
});

test("partial option import asks for a decision and never reports complete success", () => {
  const result = runScenario("options-errors");
  assert.match(result.after, /部分记录导入失败，如何处理/);
  assert.match(result.after, /期权日期无效/);
  assert.match(result.after, /暂时跳过/);
  assert.equal(result.messages.length, 0);
});

test("option recovery retains a failed-records-only warning and requires explicit confirmation", () => {
  const result = runScenario("options-recovery");
  assert.match(result.after, /仅包含尚未成功导入的记录/);
  assert.equal(result.checkDisabled, true);
  assert.equal(result.calls.filter(call => call.command === "import_options_csv").length, 1);
});
