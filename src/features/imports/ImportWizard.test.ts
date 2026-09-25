// @ts-nocheck -- Node runs a Bun probe so the real React components can load TSX.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

function runWizardScenario(scenario) {
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
    const Modal = ({ open, children, footer, onCancel, closable }) => open
      ? h("section", null, children, footer, h("button", { onClick: onCancel, disabled: closable === false }, "关闭窗口"))
      : null;
    const Upload = { Dragger: ({ beforeUpload, disabled, fileList }) => h("div", null,
      h("output", { "data-file": true }, fileList.map(file => file.name).join(",")),
      h("button", { disabled, onClick: () => beforeUpload(new File(["example"], "trades.csv")) }, "上传测试文件")) };
    mock.module("antd", () => ({
      Modal, Upload, Space: passthrough,
      Button: ({ children, onClick, disabled, loading }) => h("button", { onClick, disabled: disabled || loading }, children),
      Steps: ({ current }) => h("output", { "data-step": true }, String(current)),
      Alert: ({ message, title, description }) => h("div", { role: "alert" }, message, title, description),
      Table: ({ dataSource }) => h("div", null, dataSource.map(row => h("div", { key: row.key ?? row.symbol }, row.status ?? row.symbol))),
      Typography: { Text: passthrough, Title: passthrough }, Tag: passthrough,
      Input: () => null, InputNumber: () => null,
      message: { warning() {}, success() {}, error() {} },
    }));
    const { default: ImportWizard } = await import("./src/features/imports/ImportWizard.tsx");
    const row = { key: "row", raw: "original", selected: true, symbol: "SH600036" };
    const requests = [];
    let deferParse, deferPreview;
    const batches = new Map();
    dom.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
      if (command === "preview_import_batch") {
        requests.push(args.request);
        const batch = { id: args.request.request_id, account_id: args.request.account_id,
          source: args.request.source, file_name: args.request.file_name, parser_version: "1",
          kind: args.request.kind, status: "preview", created_at: "2026-09-25T00:00:00Z",
          rows: args.request.rows.map(row => ({ ...row, external_id: null, status: "ready", error: null, record_id: null })),
          reconciliation: [], can_undo: false, conflict: null };
        batches.set(batch.id, batch);
        if (deferPreview) await deferPreview;
        return batch;
      }
      if (command === "apply_import_batch") {
        const batch = batches.get(args.batchId);
        return { ...batch, status: "applied", can_undo: true,
          rows: batch.rows.map(row => ({ ...row, status: "imported", record_id: "transaction" })) };
      }
      throw new Error("Unexpected command: " + command);
    } };
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    let open = true;
    let accountId = "account-a";
    let imports = 0;
    const render = () => root.render(h(ImportWizard, {
      open, title: "导入交易", accountName: accountId,
      uploadTitle: "上传", uploadDescription: "CSV", columns: () => [],
      adapter: { accountId, source: "同花顺", kind: "transactions", toData: row => ({ symbol: row.symbol }),
        parseFile: async () => { if (deferParse) await deferParse; return { rows: [row], warnings: [], sourceContent: "example" }; } },
      onClose: () => { open = false; render(); },
      // Mirrors the transaction and holding pages: successful imports hide the modal externally.
      onImported: () => { imports += 1; open = false; render(); },
    }));
    const click = async (text) => {
      const button = [...container.querySelectorAll("button")].find(button => button.textContent.includes(text));
      if (!button || button.disabled) throw new Error("Button unavailable: " + text);
      await act(async () => button.click());
    };
    const snapshot = () => ({ step: container.querySelector("[data-step]")?.textContent ?? null,
      file: container.querySelector("[data-file]")?.textContent ?? null, text: container.textContent });
    const stage = async () => { await click("上传测试文件"); await click("检查"); };
    await act(async () => render());
    let result;
    if (${JSON.stringify(scenario)} === "reopen") {
      await stage();
      const before = snapshot();
      await click("导入所选行");
      const closed = snapshot();
      open = true; await act(async () => render());
      const reopened = snapshot();
      if (reopened.step === "0") await stage();
      result = { before, closed, reopened, imports, requests };
    } else if (${JSON.stringify(scenario)} === "account") {
      await stage();
      const before = snapshot();
      // A store refresh can replace the adapter without starting a new import session.
      await act(async () => render());
      const refreshed = snapshot();
      accountId = "account-b"; await act(async () => render());
      const switched = snapshot();
      if (switched.step === "0") await stage();
      result = { before, refreshed, switched, requests };
    } else if (${JSON.stringify(scenario)} === "late-parse") {
      let resolve;
      deferParse = new Promise(done => { resolve = done; });
      await click("上传测试文件");
      open = false; await act(async () => render());
      open = true; await act(async () => render());
      await act(async () => resolve());
      result = snapshot();
    } else if (${JSON.stringify(scenario)} === "late-preview") {
      await click("上传测试文件");
      let resolve;
      deferPreview = new Promise(done => { resolve = done; });
      await click("检查");
      accountId = "account-b"; await act(async () => render());
      await act(async () => resolve());
      result = snapshot();
    } else {
      await stage(); await click("完成");
      open = true; await act(async () => render());
      result = snapshot();
    }
    await act(async () => root.unmount());
    await dom.happyDOM.close();
    process.stdout.write(JSON.stringify(result));
  `;
  return JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
}

test("successful CSV import closes externally and reopens with a new file and request identity", () => {
  const result = runWizardScenario("reopen");
  assert.equal(result.before.step, "2");
  assert.equal(result.imports, 1);
  assert.equal(result.closed.step, null);
  assert.equal(result.reopened.step, "0");
  assert.equal(result.reopened.file, "");
  assert.equal(result.requests.length, 2);
  assert.notEqual(result.requests[0].request_id, result.requests[1].request_id);
});

test("account switching clears the previous CSV batch while same-account refresh preserves it", () => {
  const result = runWizardScenario("account");
  assert.equal(result.before.step, "2");
  assert.equal(result.refreshed.step, "2");
  assert.equal(result.switched.step, "0");
  assert.equal(result.switched.file, "");
  assert.deepEqual(result.requests.map(request => request.account_id), ["account-a", "account-b"]);
  assert.notEqual(result.requests[0].request_id, result.requests[1].request_id);
});

test("a late CSV parse cannot repopulate a reopened import window", () => {
  const result = runWizardScenario("late-parse");
  assert.equal(result.step, "0");
  assert.equal(result.file, "");
});

test("a late batch preview cannot appear under another account", () => {
  const result = runWizardScenario("late-preview");
  assert.equal(result.step, "0");
  assert.equal(result.file, "");
  assert.doesNotMatch(result.text, /批次：/);
});

test("finishing the import wizard clears it before the next open", () => {
  const result = runWizardScenario("finish");
  assert.equal(result.step, "0");
  assert.equal(result.file, "");
});
