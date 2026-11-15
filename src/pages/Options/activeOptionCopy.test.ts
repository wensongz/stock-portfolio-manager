// @ts-nocheck -- Node runs a Bun probe against the real React page and Ant Design tables.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

test("native document copy preserves active option records as Excel cells, including expanded openings", () => {
  const probe = String.raw`
    import { Window } from "happy-dom";
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
    const account = { id: "a", name: "测试账户", market: "US", description: null,
      created_at: "2026-01-01", updated_at: "2026-01-01" };
    const contract = {
      id: "baba-open", account_id: "a", option_symbol: "BABA 15JAN27 160 C", underlying: "BABA",
      expiry_date: "15JAN27", strike_price: 160, option_type: "C", contracts: -10, remaining_contracts: -10,
      open_price: 11.3, open_amount: 11300, commission: -4.9, traded_at: "2026-10-01, 10:01:41",
      close_price: null, close_code: null, close_details: null, status: "active",
    };
    const contracts = [contract, { ...contract, id: "baba-open-2", traded_at: null },
      { ...contract, id: "msft-open", underlying: "MSFT", option_symbol: "MSFT 15JAN27 160 C" }];
    dom.__TAURI_INTERNALS__ = { invoke: async command => {
      if (command === "get_accounts") return [account];
      if (command === "get_option_contracts") return contracts;
      if (command === "get_option_review") return {
        account_id: "a", currency: "USD", period_days: null, generated_at: "2026-11-15",
        summary: { completed_campaigns: 0, active_campaigns: 0, gross_premium: 0, net_premium_pnl: 0,
          completed_gross_premium: 0, completed_net_premium_pnl: 0, retention_rate: null,
          annualized_yield_on_notional: null, worst_campaign: null },
        underlyings: [], data_quality: { excluded_open_campaigns: 0, unmatched_records: 0, missing_trade_dates: 0, notes: [] },
      };
      throw new Error(command);
    } };
    localStorage.setItem("options_selected_account_id", "a");
    const { default: OptionsPage } = await import("./src/pages/Options/index.tsx");
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(React.createElement(OptionsPage)));
    const card = [...container.querySelectorAll(".ant-card")].find(node =>
      node.querySelector(".ant-card-head-title")?.textContent === "BABA 进行中期权记录");
    const table = card.querySelector("table");
    function copyNode(node) {
      const range = document.createRange();
      range.selectNodeContents(node);
      const selection = document.getSelection();
      selection.removeAllRanges(); selection.addRange(range);
      const data = new Map();
      const event = new dom.Event("copy", { bubbles: true, cancelable: true });
      Object.defineProperty(event, "clipboardData", { value: {
        setData: (type, value) => data.set(type, value),
      } });
      document.dispatchEvent(event);
      const pasted = document.createElement("div");
      pasted.innerHTML = data.get("text/html") ?? "";
      return { prevented: event.defaultPrevented, plain: data.get("text/plain"),
        htmlRows: [...pasted.querySelectorAll("tr")].map(row => [...row.cells].map(cell => cell.textContent)) };
    }
    const collapsed = copyNode(table);
    await act(async () => card.querySelector('[aria-label="plus"]').click());
    const expanded = copyNode(table);
    const singleCell = copyNode(table.querySelector("tbody td"));
    const result = { collapsed, expanded, singleCell };
    await act(async () => root.unmount());
    const afterUnmount = copyNode(table);
    await dom.happyDOM.close();
    process.stdout.write(JSON.stringify({ ...result, afterUnmount }));
  `;
  const result = JSON.parse(execFileSync("bun", ["--eval", probe], {
    cwd: projectRoot, encoding: "utf8", timeout: 30000,
  }));

  assert.equal(result.collapsed.prevented, true);
  const headers = ["期权标识", "股票", "到期日", "行权价", "类型", "开仓合约数", "未平仓合约数", "开仓价", "开仓权利金", "佣金", "交易时间"];
  const summary = ["BABA 15JAN27 160 C", "BABA", "15JAN27", "$160.00", "Call", "-20", "-20", "$11.30", "$22,600", "$9.8", "2026-10-01"];
  assert.deepEqual(result.collapsed.plain.split("\r\n").map(row => row.split("\t")), [headers, summary]);
  assert.deepEqual(result.collapsed.htmlRows, [headers, summary]);
  assert.equal(result.expanded.prevented, true);
  const opening = ["BABA 15JAN27 160 C", "BABA", "15JAN27", "$160.00", "Call", "-10", "-10", "$11.30", "$11,300", "$4.9", "2026-10-01"];
  const expandedRows = [headers, summary, opening, [...opening.slice(0, -1), "-"]];
  assert.deepEqual(result.expanded.plain.split("\r\n").map(row => row.split("\t")), expandedRows);
  assert.deepEqual(result.expanded.htmlRows, expandedRows);
  assert.equal(result.singleCell.prevented, false);
  assert.equal(result.afterUnmount.prevented, false);
});
