// @ts-nocheck -- Node drives a Bun probe against the actual React batch panel.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

function runPanelScenario(scenario) {
  const probe = String.raw`
    import { Window } from "happy-dom";
    import { mock } from "bun:test";
    const dom = new Window();
    for (const key of ["window", "document", "navigator", "HTMLElement", "Element", "Node", "MutationObserver"]) {
      Object.defineProperty(globalThis, key, { configurable: true, value: key === "window" ? dom : dom[key] });
    }
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    const React = await import("react");
    const { act } = React;
    const { createRoot } = await import("react-dom/client");
    const h = React.createElement;
    const passthrough = ({ children }) => h("div", null, children);
    mock.module("antd", () => ({
      Modal: ({ open, title, children, footer, closable, keyboard, maskClosable, onCancel }) => open
        ? h("section", { role: "dialog", "data-closable": String(closable), "data-keyboard": String(keyboard), "data-mask": String(maskClosable) },
          title, children, footer, h("button", { disabled: closable === false, onClick: onCancel }, "关闭窗口")) : null,
      Button: ({ children, onClick, disabled, loading }) => h("button", { onClick, disabled: disabled || loading }, children),
      Space: passthrough, Typography: { Text: passthrough, Title: passthrough, Paragraph: passthrough }, Tag: passthrough,
      Alert: ({ title, description }) => h("div", { role: "alert" }, title, description),
      Input: () => null, InputNumber: () => null,
      Table: ({ dataSource, rowSelection }) => h("div", null, dataSource.map(row => h("div", { key: row.key ?? row.symbol },
        rowSelection && h("input", { type: "checkbox", "data-key": row.key,
          checked: rowSelection.selectedRowKeys.includes(row.key), disabled: rowSelection.getCheckboxProps(row).disabled,
          onChange: event => rowSelection.onChange(event.target.checked ? [...rowSelection.selectedRowKeys, row.key] : rowSelection.selectedRowKeys.filter(key => key !== row.key)) }),
        row.data?.symbol, row.status, row.error))),
      message: { success() {}, warning() {}, error() {} },
    }));
    const { default: ImportBatchPanel } = await import("./src/features/imports/ImportBatchPanel.tsx");
    const scenario = ${JSON.stringify(scenario)};
    const row = (key, status = "ready") => ({ key, raw: { source_line: "原始记录-" + key }, external_id: null,
      data: { symbol: "SYMBOL-" + key, name: "名称-" + key }, status,
      error: status === "failed" ? "失败原因-" + key : null, record_id: status === "imported" ? "transaction-" + key : null });
    let batch = { id: "batch", account_id: "account", source: "CSV", file_name: "trades.csv", parser_version: "2",
      kind: "transactions", status: "preview", created_at: "2026-09-25T00:00:00Z",
      rows: [row("a"), row("b"), row("c")], reconciliation: [], can_undo: false, conflict: null };
    if (["initial", "initial-skip", "revise"].includes(scenario)) batch.rows = [row("ready"), ...Array.from({ length: 12 }, (_, index) => row("failed-" + index, "failed"))];
    const requests = [];
    let imports = 0, revisions = 0;
    dom.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
      requests.push({ command, ...args });
      if (scenario === "rejected" && requests.length === 1) throw new Error("连接中断，请重试");
      if (scenario === "happy" || scenario === "rejected" || (scenario === "retry" && requests.length === 3)) {
        return { ...batch, status: "applied", can_undo: true, rows: batch.rows.map(item => args.rowKeys.includes(item.key) ? row(item.key, "imported") : item) };
      }
      if (scenario === "suspected") return { ...batch, status: "applied", can_undo: true, rows: [row("a", "imported"),
        { ...row("b", "suspected"), error: "提交时发现疑似重复，请确认" }, row("c", "imported")] };
      return { ...batch, status: "applied", can_undo: scenario !== "total", conflict: scenario === "conflict" ? "账户已变更，无法重试此批次" : null,
        rows: [row("a", scenario === "total" ? "failed" : "imported"), row("b", "failed"), row("c", "failed")] };
    } };
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const render = () => root.render(h(React.StrictMode, null, h(ImportBatchPanel, { batch,
      onChange: updated => { batch = updated; render(); }, onImported: () => { imports += 1; },
      onReviseFailed: ["revise", "conflict"].includes(scenario) ? () => { revisions += 1; } : undefined })));
    const button = text => [...container.querySelectorAll("button")].find(item => item.textContent.includes(text));
    const click = async text => {
      const target = button(text);
      if (!target || target.disabled) return false;
      await act(async () => target.click()); return true;
    };
    const snapshot = () => {
      const dialog = container.querySelector("[role=dialog]");
      return { dialog: dialog?.textContent ?? null, closable: dialog?.getAttribute("data-closable"),
        keyboard: dialog?.getAttribute("data-keyboard"), mask: dialog?.getAttribute("data-mask"), imports, revisions,
        selected: [...container.querySelectorAll("input:checked")].map(input => input.dataset.key),
        retryDisabled: button("查看并重试")?.disabled ?? null, reviseAvailable: !!button("返回修改"),
        rows: batch.rows.map(item => ({ key: item.key, status: item.status })) };
    };
    await act(async () => render());
    let result;
    if (["initial", "initial-skip", "revise"].includes(scenario)) {
      const before = snapshot();
      await click(scenario === "revise" ? "返回修改" : scenario === "initial-skip" ? "暂时跳过" : "查看并重试");
      const after = snapshot();
      batch = { ...batch }; await act(async () => render());
      result = { before, after, rerender: snapshot(), requests };
    } else {
      if (scenario === "rejected") await act(async () => container.querySelector('[data-key="c"]').click());
      await click("导入所选行");
      const before = snapshot();
      if (["skip", "total", "conflict"].includes(scenario)) {
        await click("暂时跳过");
        batch = { ...batch }; await act(async () => render());
        result = { before, after: snapshot(), requests };
      } else if (["retry", "rejected", "suspected"].includes(scenario)) {
        await click("查看并重试");
        const afterChoice = snapshot();
        const callsAfterChoice = requests.length;
        if (scenario !== "suspected") await click("导入 / 重试所选行") || await click("导入所选行");
        const retried = snapshot();
        if (scenario === "retry") { await click("查看并重试"); await click("导入 / 重试所选行"); }
        result = { before, afterChoice, callsAfterChoice, retried, after: snapshot(), requests };
      } else result = { before, requests };
    }
    await act(async () => root.unmount());
    await dom.happyDOM.close();
    process.stdout.write(JSON.stringify(result));
  `;
  return JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
}

test("partial failures remain open until the user retries only failed rows, including repeated failures", () => {
  const result = runPanelScenario("retry");
  assert.equal(result.before.imports, 0, "partial failures must not close the parent import window");
  assert.match(result.before.dialog, /SYMBOL-b.*失败原因-b.*原始记录-b/s);
  assert.match(result.before.dialog, /SYMBOL-c.*失败原因-c.*原始记录-c/s);
  assert.equal(result.before.closable, "false");
  assert.equal(result.before.keyboard, "false");
  assert.equal(result.before.mask, "false");
  assert.equal(result.afterChoice.dialog, null);
  assert.deepEqual(result.afterChoice.selected, ["b", "c"]);
  assert.equal(result.callsAfterChoice, 1, "choosing to retry must not submit automatically");
  assert.match(result.retried.dialog, /失败原因-b/);
  assert.deepEqual(result.requests.map(request => request.rowKeys), [["a", "b", "c"], ["b", "c"], ["b", "c"]]);
  assert.equal(result.after.imports, 1);
  assert.equal(result.after.dialog, null);
});

test("initial saved failures show every failed row before any write and stay acknowledged on refresh", () => {
  const result = runPanelScenario("initial");
  for (let index = 0; index < 12; index++) {
    assert.ok(result.before.dialog?.includes("SYMBOL-failed-" + index));
    assert.ok(result.before.dialog?.includes("失败原因-failed-" + index));
    assert.ok(result.before.dialog?.includes("原始记录-failed-" + index));
  }
  assert.deepEqual(result.requests, []);
  assert.deepEqual(result.after.selected, Array.from({ length: 12 }, (_, index) => "failed-" + index));
  assert.equal(result.after.dialog, null);
  assert.equal(result.rerender.dialog, null);
  assert.equal(result.before.reviseAvailable, false);
});

test("returning to edit acknowledges the failure without writing or claiming an import", () => {
  const result = runPanelScenario("revise");
  assert.equal(result.before.reviseAvailable, true);
  assert.equal(result.after.revisions, 1);
  assert.equal(result.after.imports, 0);
  assert.equal(result.after.dialog, null);
  assert.equal(result.rerender.dialog, null);
  assert.deepEqual(result.requests, []);
});

test("skipping initial failures leaves valid rows available without importing them", () => {
  const result = runPanelScenario("initial-skip");
  assert.ok(result.before.dialog);
  assert.equal(result.after.dialog, null);
  assert.equal(result.rerender.dialog, null);
  assert.deepEqual(result.after.selected, ["ready"]);
  assert.equal(result.after.imports, 0);
  assert.deepEqual(result.requests, []);
  assert.deepEqual(result.after.rows, result.before.rows);
});

test("explicitly skipping partial failures preserves their records and may finish the successful portion", () => {
  const result = runPanelScenario("skip");
  assert.equal(result.before.imports, 0);
  assert.equal(result.after.imports, 1);
  assert.equal(result.after.dialog, null);
  assert.deepEqual(result.after.selected, []);
  assert.deepEqual(result.after.rows, [{ key: "a", status: "imported" }, { key: "b", status: "failed" }, { key: "c", status: "failed" }]);
  assert.equal(result.requests.length, 1);
});

test("skipping a total failure never reports successful imports or writes again", () => {
  const result = runPanelScenario("total");
  assert.equal(result.before.imports, 0);
  assert.match(result.before.dialog, /失败原因-a/);
  assert.equal(result.after.imports, 0);
  assert.equal(result.after.dialog, null);
  assert.deepEqual(result.after.selected, []);
  assert.equal(result.requests.length, 1);
});

test("a rejected apply preserves the batch and presents actionable retry choices", () => {
  const result = runPanelScenario("rejected");
  assert.equal(result.before.imports, 0);
  assert.match(result.before.dialog, /连接中断，请重试/);
  assert.match(result.before.dialog, /原始记录-a/);
  assert.deepEqual(result.before.rows.map(row => row.status), ["ready", "ready", "ready"]);
  assert.deepEqual(result.afterChoice.selected, ["a", "b"]);
  assert.equal(result.callsAfterChoice, 1);
  assert.deepEqual(result.requests.map(request => request.rowKeys), [["a", "b"], ["a", "b"]]);
  assert.equal(result.after.imports, 1);
});

test("conflicted failures explain why retry and returning to edit are unavailable", () => {
  const result = runPanelScenario("conflict");
  assert.equal(result.before.imports, 0);
  assert.match(result.before.dialog, /账户已变更/);
  assert.equal(result.before.retryDisabled, true);
  assert.equal(result.before.reviseAvailable, false);
  assert.equal(result.requests.length, 1);
});

test("newly suspected rows require review instead of closing or silently selecting a retry", () => {
  const result = runPanelScenario("suspected");
  assert.equal(result.before.imports, 0);
  assert.match(result.before.dialog, /疑似重复/);
  assert.deepEqual(result.afterChoice.selected, []);
  assert.equal(result.callsAfterChoice, 1);
});

test("a fully successful apply still notifies the parent once", () => {
  const result = runPanelScenario("happy");
  assert.equal(result.before.imports, 1);
  assert.equal(result.before.dialog, null);
  assert.equal(result.requests.length, 1);
});
