// @ts-nocheck -- Node runs a Bun DOM probe against the actual modal and cash table.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

function runScenario(scenario, currency = "HKD") {
  const probe = String.raw`
    import { Window } from "happy-dom";
    import { mock } from "bun:test";
    const dom = new Window();
    for (const key of ["window", "document", "navigator", "localStorage", "HTMLElement", "Element", "Node", "MutationObserver", "ResizeObserver", "ShadowRoot", "SVGElement", "getComputedStyle"]) {
      Object.defineProperty(globalThis, key, { configurable: true, value: key === "window" ? dom : typeof dom[key] === "function" && key === "getComputedStyle" ? dom[key].bind(dom) : dom[key] });
    }
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    const React = await import("react");
    const { act } = React;
    const { createRoot } = await import("react-dom/client");
    const h = React.createElement;
    const antd = await import("antd");
    const errors = [];
    mock.module("antd", () => ({ ...antd,
      Modal: ({ open, title, children }) => open ? h("section", { role: "dialog" }, h("h2", null, title), children) : null,
      message: { error: text => errors.push(text) },
    }));
    let editProps;
    mock.module("./src/pages/Transactions/TransactionFormModal", () => ({ default: props => {
      editProps = props;
      return props.open ? h("output", { "data-editor": true }, props.transaction.symbol) : null;
    } }));
    const currency = ${JSON.stringify(currency)};
    const scenario = ${JSON.stringify(scenario)};
    const cashSymbol = "$CASH-" + currency;
    const base = { id: "deposit", holding_id: "cash-holding", account_id: "account-a", symbol: cashSymbol,
      name: "现金", market: "HK", transaction_type: "BUY", shares: 0, price: 0, total_amount: 1000,
      commission: 0, currency, traded_at: "2026-10-28T10:30:00", notes: "存款备注", created_at: "2026-10-28T10:30:00" };
    const rows = [
      { ...base, id: "dividend", symbol: "700.HK", name: "腾讯控股", transaction_type: "PAY", total_amount: 20, notes: "现金分红", cash_delta: 20, running_balance: 1018 },
      { ...base, id: "sell", symbol: "700.HK", name: "腾讯控股", transaction_type: "SELL", shares: 1, price: 300, total_amount: 300, commission: 1, cash_delta: 299, running_balance: 998 },
      { ...base, id: "buy", symbol: "700.HK", name: "腾讯控股", shares: 1, price: 200, total_amount: 200, commission: 1, notes: "股票买入", cash_delta: -201, running_balance: 699 },
      { ...base, id: "withdrawal", transaction_type: "SELL", total_amount: 100, cash_delta: -100, running_balance: 900 },
      { ...base, cash_delta: 1000, running_balance: 1000 },
      { ...base, id: "opening", transaction_type: "OPEN", shares: 0, total_amount: 0, cash_delta: 0, running_balance: 0 },
    ];
    const stockRows = [
      { ...rows[1], shares: 4, price: 30, total_amount: 120, traded_at: "2026-10-29T10:00:00" },
      { ...rows[2], shares: 10, price: 20, total_amount: 200 },
    ];
    const reconciliation = { holding_id: "cash-holding", account_id: "account-a", currency,
      current_balance: 1018, recommended_balance: 1018, difference: 0, revision: 1, opening_count: 1, rows };
    const calls = [];
    let releaseFirst;
    let cashReads = 0;
    let stockReads = 0;
    dom.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
      calls.push({ command, args });
      if (command === "get_transactions") {
        stockReads += 1;
        if (scenario.startsWith("stock-")) return stockRows.map(row => row.id === "sell" && stockReads > 1 ? { ...row, shares: 2, total_amount: 60 } : row);
        return args.symbol.startsWith("$CASH-") ? [base] : [rows[2]];
      }
      if (command === "get_cash_balance_reconciliation") {
        cashReads += 1;
        if (scenario === "switch" && args.id === "cash-holding") await new Promise(resolve => { releaseFirst = resolve; });
        if (scenario === "error") throw new Error("现金流水读取失败");
        return { ...reconciliation, holding_id: args.id, rows: args.id === "other-holding" ? [{ ...rows[0], id: "other-row", notes: "另一个账户的流水" }] : rows };
      }
      throw new Error("Unexpected command: " + command);
    } };
    const { useQuoteStore } = await import("./src/stores/quoteStore.ts");
    const quoteHolding = { id: "cash-holding", account_id: "account-a", symbol: "700.HK", name: "腾讯控股", market: "HK", category_id: null,
      shares: 6, avg_cost: 13.67, currency, created_at: base.created_at, updated_at: base.created_at,
      quote: { symbol: "700.HK", name: "腾讯控股", market: "HK", current_price: 35, previous_close: 34, change: 1, change_percent: 2.94, high: 35, low: 34, volume: 100, updated_at: base.created_at },
      market_value: 210, total_cost: 82, unrealized_pnl: 128, unrealized_pnl_percent: 156 };
    useQuoteStore.setState({ holdingQuotes: scenario === "stock-no-quote" ? [] : [quoteHolding] });
    const { default: Modal } = await import("./src/pages/Statistics/AccountStockTransactionsModal.tsx");
    const container = document.createElement("div"); document.body.append(container);
    const root = createRoot(container);
    const props = { open: true, holdingId: "cash-holding", accountId: "account-a", accountName: "港股账户",
      symbol: scenario.startsWith("stock") ? "700.HK" : cashSymbol, stockName: "现金", onClose() {}, onUpdated: async () => { updates += 1; } };
    let updates = 0;
    const render = async () => { await act(async () => root.render(h(Modal, { ...props }))); };
    await render();
    if (scenario === "switch") {
      props.holdingId = "other-holding"; props.accountId = "account-b";
      await render();
      if (releaseFirst) await act(async () => releaseFirst());
    }
    const before = container.textContent;
    const titleBefore = container.querySelector("h2")?.textContent;
    let editedSymbol;
    if (scenario === "edit" || scenario === "stock-edit") {
      const row = container.querySelector(scenario === "edit" ? '[data-row-key="buy"]' : '[data-row-key="sell"]');
      const button = row?.querySelector("button");
      if (button) {
        await act(async () => button.click());
        editedSymbol = container.querySelector("[data-editor]")?.textContent;
        await act(async () => editProps.onSaved({ ...(scenario === "edit" ? rows[2] : stockRows[0]), notes: "已保存的新备注" }));
      }
    }
    if (scenario === "stock-quote") {
      await act(async () => useQuoteStore.setState({ holdingQuotes: [{ ...quoteHolding, quote: { ...quoteHolding.quote, current_price: 40 } }] }));
    }
    const summary = container.querySelector('[aria-label="交易盈亏汇总"]');
    const result = { before, after: container.textContent, calls, errors, updates, editedSymbol, cashReads,
      titleBefore, titleAfter: container.querySelector("h2")?.textContent,
      summaryStyle: summary ? { textAlign: summary.style.textAlign, marginInlineStart: summary.style.marginInlineStart } : null,
      headers: [...container.querySelectorAll("thead th")].map(th => th.textContent),
      openingDisabled: container.querySelector('[data-row-key="opening"] button')?.disabled,
      rowTexts: Object.fromEntries([...container.querySelectorAll("tbody tr[data-row-key]")].map(row => [row.dataset.rowKey, row.textContent])) };
    await act(async () => root.unmount()); await dom.happyDOM.close();
    process.stdout.write(JSON.stringify(result));
  `;
  try {
    return JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8", timeout: 30000, stdio: ["ignore", "pipe", "pipe"] }));
  } catch (error) {
    throw new Error(`DOM probe failed: ${error.stderr?.slice(0, 4000) || error.message}`);
  }
}

test("account cash details use the holdings cash ledger for every supported currency", () => {
  for (const [currency, prefix] of [["HKD", "HK$"], ["USD", "$"], ["CNY", "¥"]]) {
    const result = runScenario("cash", currency);
    assert.deepEqual(result.calls, [{ command: "get_cash_balance_reconciliation", args: { id: "cash-holding" } }]);
    assert.deepEqual(result.headers, ["日期", "类型", "股票", "现金变动", "余额", "备注", "操作"]);
    assert.ok(result.rowTexts.deposit.includes("存入"));
    assert.ok(result.rowTexts.withdrawal.includes("提取"));
    assert.ok(result.rowTexts.opening.includes("期初"));
    assert.ok(result.rowTexts.buy.includes("700.HK腾讯控股"));
    assert.ok(result.rowTexts.buy.includes("-" + prefix + "201.00"));
    assert.ok(result.rowTexts.buy.includes(prefix + "699.00"));
    assert.ok(result.rowTexts.sell.includes("+" + prefix + "299.00"));
    assert.ok(result.rowTexts.dividend.includes("分红"));
    assert.ok(result.rowTexts.dividend.includes("+" + prefix + "20.00"));
    assert.equal(result.openingDisabled, true);
    assert.deepEqual(result.errors, []);
    assert.doesNotMatch(result.titleBefore, /浮动盈亏|实现盈亏/);
  }
});

test("editing a stock transaction from cash details refreshes the ledger and committed note", () => {
  const result = runScenario("edit");
  assert.equal(result.editedSymbol, "700.HK");
  assert.equal(result.cashReads, 2);
  assert.equal(result.updates, 1);
  assert.match(result.rowTexts.buy, /已保存的新备注/);
});

test("switching cash holdings ignores a late response from the previous account", () => {
  const result = runScenario("switch");
  assert.match(result.after, /另一个账户的流水/);
  assert.doesNotMatch(result.after, /股票买入/);
  assert.equal(result.calls.at(-1).args.id, "other-holding");
});

test("cash ledger failures are reported without falling back to incomplete cash-symbol trades", () => {
  const result = runScenario("error");
  assert.match(result.errors[0], /现金流水读取失败/);
  assert.deepEqual(result.rowTexts, {});
});

test("ordinary stocks retain their standard transaction details", () => {
  const result = runScenario("stock");
  assert.deepEqual(result.calls, [{ command: "get_transactions", args: { accountId: "account-a", symbol: "700.HK" } }]);
  assert.deepEqual(result.headers, ["日期", "类型", "股数", "价格", "总金额", "手续费", "备注", "操作"]);
  assert.match(result.rowTexts.buy, /买入/);
  assert.match(result.rowTexts.buy, /HK\$200\.00/);
});

test("stock detail title shows replayed floating and realized profit aligned right", () => {
  const result = runScenario("stock-pnl");
  assert.match(result.titleBefore, /浮动盈亏.*\+HK\$89\.40.*实现盈亏.*\+HK\$38\.60/);
  assert.deepEqual(result.summaryStyle, { textAlign: "right", marginInlineStart: "auto" });
  assert.deepEqual(Object.keys(result.rowTexts), ["sell", "buy"], "visible trades remain newest first");
});

test("saving a stock trade recalculates both amounts from the reloaded records", () => {
  const result = runScenario("stock-edit");
  assert.match(result.titleAfter, /浮动盈亏.*\+HK\$119\.20.*实现盈亏.*\+HK\$18\.80/);
  assert.equal(result.updates, 1);
});

test("stock title updates with cached quotes without fetching transactions again", () => {
  const result = runScenario("stock-quote");
  assert.match(result.titleAfter, /浮动盈亏.*\+HK\$119\.40.*实现盈亏.*\+HK\$38\.60/);
  assert.equal(result.calls.length, 1);
});

test("a stock without a cached quote still shows realized profit", () => {
  const result = runScenario("stock-no-quote");
  assert.match(result.titleAfter, /浮动盈亏.*—.*实现盈亏.*\+HK\$38\.60/);
});
