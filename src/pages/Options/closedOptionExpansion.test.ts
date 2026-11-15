// @ts-nocheck -- Node runs a Bun probe against the real React page and Ant Design tables.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

function runScenario(grouped = false, terminalStatus = "closed") {
  const probe = String.raw`
    import { Window } from "happy-dom";
    import { mock } from "bun:test";
    const dom = new Window({ url: "http://localhost" });
    for (const key of ["window", "document", "navigator", "HTMLElement", "HTMLDocument", "Element", "Node", "SVGElement", "ShadowRoot", "DocumentFragment", "MutationObserver", "ResizeObserver", "localStorage"]) {
      Object.defineProperty(globalThis, key, { configurable: true, value: key === "window" ? dom : dom[key] });
    }
    globalThis.getComputedStyle = dom.getComputedStyle.bind(dom);
    globalThis.requestAnimationFrame = dom.requestAnimationFrame.bind(dom);
    globalThis.cancelAnimationFrame = dom.cancelAnimationFrame.bind(dom);
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    const React = await import("react");
    const { act } = React;
    const { createRoot } = await import("react-dom/client");
    const h = React.createElement;
    const noop = () => {};
    const accounts = [{ id: "a", name: "测试账户", market: "US" }];
    mock.module("./src/stores/accountStore", () => ({ useAccountStore: () => ({ accounts, fetchAccounts: noop }) }));
    const report = { account_id: "a", currency: "USD", period_days: null, underlyings: [] };
    mock.module("./src/stores/optionReviewStore", () => ({ useOptionReviewStore: () => ({
      report, loading: false, error: null, requestedAccountId: "a", requestedPeriodDays: null,
      fetchOptionReview: noop, clearOptionReview: noop,
    }) }));
    mock.module("./src/components/charts/StatCard", () => ({ default: () => null }));
    const contract = {
      id: "baba-open", account_id: "a", option_symbol: "BABA 15JAN27 160 C", underlying: "BABA",
      expiry_date: "15JAN27", strike_price: 160, option_type: "C", contracts: -10, remaining_contracts: 0,
      open_price: 11.3, open_amount: 11300, commission: -4.9, traded_at: "2024-10-01, 10:01:41",
      close_price: 5.2, close_code: "C", status: "closed",
      close_details: { close_cost: 5200, fees: 9.34, realized_pnl: 6090.66, records: [
        { id: "baba-close", code: "C", quantity: 10, price: 5.2, amount: 5200, commission: 4.44, fee: 0, traded_at: "2024-12-04, 09:57:34" },
      ] },
    };
    const terminalStatus = ${JSON.stringify(terminalStatus)};
    if (terminalStatus !== "closed") Object.assign(contract, {
      option_symbol: "BABA 16SEP22 170 C", expiry_date: "16SEP22", strike_price: 170,
      contracts: -50, open_price: 2.53, open_amount: 12650, commission: -34.86,
      traded_at: "2022-06-28", close_price: 0,
      close_code: terminalStatus === "expired" ? "C;Ep" : "A;C", status: terminalStatus,
      close_details: { close_cost: 600, fees: 82.40, realized_pnl: 11967.60, records: [
        { id: "baba-buyback", code: "C;P", quantity: 40, price: 0.15, amount: 600, commission: 47.54, fee: 0, traded_at: "2022-07-28, 11:22:00" },
        { id: "baba-terminal", code: terminalStatus === "expired" ? "C;Ep" : "A;C", quantity: 10, price: 0, amount: 0, commission: 0, fee: 0, traded_at: "2022/9/16" },
      ] },
    });
    const contracts = [contract, { ...contract, id: "expired", underlying: "MSFT", option_symbol: "MSFT 15JAN27 160 C", status: "expired", close_price: 0, close_code: "C;Ep", close_details: null }];
    if (${JSON.stringify(grouped)}) contracts.push({ ...contract, id: "baba-open-2", open_price: 12, open_amount: 12000,
      close_details: { ...contract.close_details, realized_pnl: 6790.66 } });
    if (${JSON.stringify(grouped)} && terminalStatus !== "closed") contracts.unshift({
      ...contract, id: "expiry-only", open_amount: 1000, commission: -1, close_details: null,
    });
    dom.__TAURI_INTERNALS__ = { invoke: async command => {
      if (command === "get_option_contracts") return contracts;
      throw new Error(command);
    } };
    localStorage.setItem("options_selected_account_id", "a");
    const { default: OptionsPage } = await import("./src/pages/Options/index.tsx");
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container);
    function copyNode(node) {
      const selection = document.getSelection();
      const range = document.createRange();
      range.selectNodeContents(node);
      selection.removeAllRanges(); selection.addRange(range);
      const data = new Map();
      const event = new dom.Event("copy", { bubbles: true, cancelable: true });
      Object.defineProperty(event, "clipboardData", { value: {
        setData: (type, value) => data.set(type, value),
      } });
      // Native menus may dispatch outside the inner table's React event subtree.
      document.dispatchEvent(event);
      return { prevented: event.defaultPrevented, plain: data.get("text/plain"), html: data.get("text/html") };
    }
    await act(async () => root.render(h(OptionsPage)));
    const tab = [...container.querySelectorAll('[role="tab"]')].find(node => node.textContent.includes("已到期"));
    await act(async () => tab.click());
    const buttons = () => [...container.querySelectorAll("button")].filter(node =>
      node.textContent.includes("已平仓") || node.textContent.includes("含平仓"));
    let target = buttons()[0];
    const before = { buttonCount: buttons().length, expanded: target?.getAttribute("aria-expanded") ?? null,
      text: target?.textContent, tagClass: target?.querySelector(".ant-tag")?.className };
    let after = null, collapsed = null, returned = null, openingCopy = null;
    if (target) {
      await act(async () => target.click());
      if (${JSON.stringify(grouped)}) {
        openingCopy = copyNode(target.closest("tr").nextElementSibling);
        target = buttons().find(node => node.closest("table") !== target.closest("table"));
        if (target) await act(async () => target.click());
      }
      if (target) {
        const row = target.closest("tr");
        const detailRow = row.nextElementSibling;
        after = { expanded: target.getAttribute("aria-expanded"), text: detailRow?.textContent ?? "", below: detailRow?.querySelector("table") != null, copy: copyNode(detailRow) };
        await act(async () => target.click());
        collapsed = { expanded: target.getAttribute("aria-expanded"), visible: detailRow?.style.display !== "none" };
        const summary = [...container.querySelectorAll(".ant-card")].find(node => node.querySelector(".ant-card-head-title")?.textContent === "个股期权汇总" && node.closest('[role="tabpanel"]')?.getAttribute("aria-hidden") !== "true");
        const summaryRow = symbol => [...summary.querySelectorAll("tbody tr")].find(node => node.querySelector("td")?.textContent.includes(symbol));
        await act(async () => target.click());
        await act(async () => summaryRow("MSFT").click());
        await act(async () => summaryRow("BABA").click());
        returned = buttons()[0]?.getAttribute("aria-expanded");
      }
    }
    const result = { before, after, collapsed, returned, openingCopy };
    await act(async () => root.unmount());
    await dom.happyDOM.close();
    process.stdout.write(JSON.stringify(result));
  `;
  return JSON.parse(execFileSync("bun", ["--eval", probe], {
    cwd: projectRoot, encoding: "utf8", timeout: 30000,
  }));
}

test("clicking a closed status reveals its closing trades and net P&L below the opening row, then collapses", () => {
  const result = runScenario();
  assert.equal(result.before.buttonCount, 1);
  assert.equal(result.before.expanded, "false");
  assert.equal(result.after.expanded, "true");
  assert.equal(result.after.below, true);
  assert.match(result.after.text, /2024-12-04/);
  assert.match(result.after.text, /\$5\.20/);
  assert.match(result.after.text, /\$5,200\.00/);
  assert.match(result.after.text, /\$9\.34/);
  assert.match(result.after.text, /\+\$6,090\.66/);
  assert.equal(result.collapsed.expanded, "false");
  assert.equal(result.collapsed.visible, false);
  assert.equal(result.returned, "false");
});

test("grouped completed contracts still allow each opening record to reveal its own closing details", () => {
  const result = runScenario(true);
  assert.equal(result.after?.expanded, "true");
  assert.match(result.after.text, /\+\$6,090\.66/);
  assert.equal(result.collapsed.expanded, "false");
});

test("native document copy preserves expanded opening rows and closing details as Excel cells", () => {
  const result = runScenario(true);
  assert.equal(result.openingCopy.prevented, true);
  const openingRows = result.openingCopy.plain.split("\r\n").map(row => row.split("\t"));
  assert.equal(openingRows.length, 3);
  assert.equal(openingRows[0][0], "期权标识");
  assert.equal(openingRows[1][0], "BABA 15JAN27 160 C");
  assert.equal(openingRows[2][0], "BABA 15JAN27 160 C");
  assert.ok(openingRows.every(row => row.length === 11));
  assert.equal(result.after.copy.prevented, true);
  const closingRows = result.after.copy.plain.split("\r\n").map(row => row.split("\t"));
  assert.deepEqual(closingRows[0], ["平仓时间", "合约数", "平仓价", "平仓支出", "平仓费用", "方式"]);
  assert.equal(closingRows[1][0], "2024-12-04, 09:57:34");
  assert.match(closingRows[2][3], /已实现净盈亏：\+\$6,090\.66/);
  assert.equal((result.after.copy.html.match(/<table>/g) ?? []).length, 1);
});

for (const [status, method] of [["expired", "到期"], ["assigned", "被执行"]]) {
  test(`${status} completed records reveal earlier partial buybacks and copy every closing cash flow`, () => {
    const result = runScenario(false, status);
    assert.equal(result.before.buttonCount, 1);
    assert.equal(result.before.text, "已平仓");
    assert.match(result.before.tagClass, /ant-tag-blue/);
    assert.equal(result.after?.expanded, "true");
    assert.match(result.after.text, /\$600\.00/);
    assert.match(result.after.text, /\$47\.54/);
    assert.match(result.after.text, /总费用：\$82\.40/);
    assert.match(result.after.text, /已实现净盈亏：\+\$11,967\.60/);
    const rows = result.after.copy.plain.split("\r\n").map(row => row.split("\t"));
    assert.deepEqual(rows[1], ["2022-07-28, 11:22:00", "40", "$0.15", "$600.00", "$47.54", "买入平仓"]);
    assert.deepEqual(rows[2], ["2022-09-16", "10", "$0.00", "$0.00", "$0.00", method]);
    assert.equal(result.collapsed.expanded, "false");
    assert.equal(result.returned, "false");
  });
}

test("a completed group exposes a later opening's buyback when its first opening only expired", () => {
  const result = runScenario(true, "expired");
  assert.equal(result.before.text, "已平仓");
  assert.match(result.before.tagClass, /ant-tag-blue/);
  assert.equal(result.after?.expanded, "true");
  assert.match(result.after.text, /\$600\.00/);
  assert.match(result.after.text, /\$47\.54/);
});
