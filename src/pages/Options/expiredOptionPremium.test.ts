// @ts-nocheck -- Node runs a Bun probe against the real React page and Ant Design tables.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

test("TAL completed rows and Excel copy show commission expenses as positive and rebates as negative", () => {
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
    const account = { id: "a", name: "U8931107", market: "US", description: null,
      created_at: "2026-01-01", updated_at: "2026-01-01" };
    const opening = {
      id: "tal-feb-11", account_id: "a", option_symbol: "TAL 20FEB26 10 P", underlying: "TAL",
      expiry_date: "20FEB26", strike_price: 10, option_type: "P", contracts: -11, remaining_contracts: 0,
      open_price: 0.81, open_amount: 891, commission: -17.1, traded_at: "2025-08-21, 12:42:58",
      close_price: 0, close_code: "C;Ep", close_details: null, status: "expired",
    };
    const contracts = [opening,
      { ...opening, id: "tal-feb-89", contracts: -89, open_price: 0.8, open_amount: 7120,
        commission: 41.45, traded_at: "2025-08-22, 10:20:37" },
      { ...opening, id: "tal-may", option_symbol: "TAL 15MAY26 10 P", expiry_date: "15MAY26",
        contracts: -100, open_price: 0.82, open_amount: 8200, commission: 35.13, traded_at: "2025-11-13, 10:40:58" },
      { ...opening, id: "tal-aug", option_symbol: "TAL 21AUG26 10 P", expiry_date: "21AUG26",
        contracts: -100, open_price: 1.1, open_amount: 11000, commission: -155.2, traded_at: "2026-02-25, 10:25:08" },
    ];
    const summary = { completed_campaigns: 4, active_campaigns: 0, gross_premium: 27211,
      net_premium_pnl: 27115.28, completed_gross_premium: 27211, completed_net_premium_pnl: 27115.28,
      retention_rate: null, annualized_yield_on_notional: null, worst_campaign: null };
    const campaigns = contracts.map((contract, index) => ({
      id: "option-review:a:TAL:" + contract.id, underlying: "TAL", option_symbol: contract.option_symbol,
      expiry_date: contract.expiry_date, contracts: contract.contracts,
      started_at: contract.traded_at.substring(0, 10), ended_at: ["2026-02-20", "2026-02-20", "2026-05-15", "2026-08-21"][index],
      status: "completed", inferred: true, strategy_path: ["CSP"], gross_premium: contract.open_amount,
      close_cost: 0, fees: [17.1, -41.45, -35.13, 155.2][index], net_premium_pnl: [873.9, 7161.45, 8235.13, 10844.8][index],
      secured_notional: 0, capital_days: 0, retention_rate: null, annualized_yield_on_notional: null,
    }));
    dom.__TAURI_INTERNALS__ = { invoke: async command => {
      if (command === "get_accounts") return [account];
      if (command === "get_option_contracts") return contracts;
      if (command === "get_option_review") return {
        account_id: "a", currency: "USD", period_days: null, generated_at: "2026-11-15",
        summary, underlyings: [{ ...summary, underlying: "TAL", worst_campaign_pnl: 873.9, flags: [], campaigns }],
        data_quality: { excluded_open_campaigns: 0, unmatched_records: 0, missing_trade_dates: 0, notes: [] },
      };
      throw new Error(command);
    } };
    localStorage.setItem("options_selected_account_id", "a");
    const { default: OptionsPage } = await import("./src/pages/Options/index.tsx");
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container);
    await act(async () => root.render(React.createElement(OptionsPage)));
    const tab = [...container.querySelectorAll('[role="tab"]')].find(node => node.textContent.includes("已到期"));
    await act(async () => tab.click());
    const panel = container.querySelector('[role="tabpanel"][aria-hidden="false"]');
    const cards = [...panel.querySelectorAll(".ant-card")];
    const summaryCard = cards.find(node => node.querySelector(".ant-card-head-title")?.textContent === "个股期权汇总");
    const summaryRow = [...summaryCard.querySelectorAll("tbody tr")].find(node => node.querySelector("td")?.textContent === "TAL");
    await act(async () => summaryRow.click());
    const card = cards.find(node => node.querySelector(".ant-card-head-title")?.textContent === "TAL 已完成期权记录");
    const table = card.querySelector("table");
    const rows = () => [...table.querySelectorAll("tbody tr[data-row-key]")].map(row =>
      [...row.cells].filter(cell => !cell.classList.contains("ant-table-row-expand-icon-cell")).map(cell => cell.textContent));
    const displayedRows = rows();
    const range = document.createRange(); range.selectNodeContents(table);
    document.getSelection().addRange(range);
    const clipboard = new Map();
    const event = new dom.Event("copy", { bubbles: true, cancelable: true });
    Object.defineProperty(event, "clipboardData", { value: { setData: (type, value) => clipboard.set(type, value) } });
    document.dispatchEvent(event);
    const pasted = document.createElement("div"); pasted.innerHTML = clipboard.get("text/html") ?? "";
    await act(async () => card.querySelector('[aria-label="plus"]').click());
    const result = { displayedRows, summaryCells: [...summaryRow.cells].map(cell => cell.textContent),
      prevented: event.defaultPrevented, plain: clipboard.get("text/plain"), expandedRows: rows(),
      htmlRows: [...pasted.querySelectorAll("tr")].map(row => [...row.cells].map(cell => cell.textContent)) };
    await act(async () => root.unmount());
    await dom.happyDOM.close();
    process.stdout.write(JSON.stringify(result));
  `;
  const result = JSON.parse(execFileSync("bun", ["--eval", probe], {
    cwd: projectRoot, encoding: "utf8", timeout: 30000,
  }));

  assert.deepEqual(result.displayedRows.map(row => [row[0], row[7], row[8]]), [
    ["TAL 21AUG26 10 P", "$11,000", "$155.20"],
    ["TAL 15MAY26 10 P", "$8,200", "-$35.13"],
    ["TAL 20FEB26 10 P", "$8,011", "-$24.35"],
  ]);
  assert.equal(result.prevented, true);
  const copied = result.plain.split("\r\n").map(row => row.split("\t"));
  assert.deepEqual(copied.slice(1), result.displayedRows);
  assert.deepEqual(result.htmlRows, copied);
  const money = value => Number(value.replace(/[$,]/g, ""));
  assert.equal(copied.slice(1).reduce((sum, row) => sum + money(row[7]), 0), 27211);
  const commission = copied.slice(1).reduce((sum, row) => sum + money(row[8]), 0);
  assert.equal(Number(commission.toFixed(2)), 95.72);
  assert.equal(money(result.summaryCells[1]), 27115.28);
  assert.equal(Number((27211 - commission).toFixed(2)), money(result.summaryCells[1]));
  assert.deepEqual(result.expandedRows.slice(-2).map(row => row[8]), ["$17.10", "-$41.45"]);
});
