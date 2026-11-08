// @ts-nocheck -- Node drives Bun SSR and DOM probes against the actual modal and stores.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

function submitTransaction(saveFails = false, refreshFails = false, notes = "感觉低估aaaab", symbol = "TEST", transactionType = "BUY", market = "US", currency = "USD") {
  const probe = String.raw`
    const React = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const antd = await import("antd");
    const { mock } = await import("bun:test");
    const dayjs = (await import("dayjs")).default;
    let onFinish, symbolField;
    const passthrough = ({ children }) => children;
    const Form = ({ children, onFinish: submit }) => { onFinish = submit; return children; };
    Form.Item = (props) => { if (props.name === "symbol") symbolField = props; return props.children; };
    Form.useForm = () => [{ resetFields() {}, setFieldsValue() {}, getFieldValue() {}, submit() {} }];
    Form.useWatch = (name) => ({ transactionType: "BUY", market: ${JSON.stringify(market)} })[name];
    const errors = [];
    const warnings = [];
    mock.module("antd", () => ({ ...antd, Modal: passthrough, Form, message: { success() {}, error: (text) => errors.push(text), warning: (text) => warnings.push(text) } }));
    const original = { id: "transaction-test", holding_id: "holding-test", account_id: "account-test", symbol: "TEST", name: "Example", market: "US", transaction_type: "BUY", shares: 1, price: 100, total_amount: 100, commission: 0, currency: "USD", traded_at: "2026-09-18T02:30:00.000Z", notes: "修改前备注", created_at: "2026-09-18T02:30:00.000Z" };
    const commands = [];
    const submittedPayloads = [];
    globalThis.window = { __TAURI_INTERNALS__: { invoke: async (command, args) => {
      commands.push(command);
      if (command !== "update_transaction") throw new Error("Unexpected command: " + command);
      submittedPayloads.push(args);
      if (${saveFails}) throw new Error("测试：交易保存失败");
      return { ...original, shares: args.shares, total_amount: args.totalAmount, notes: args.notes };
    } } };
    const { useTransactionStore } = await import("./src/stores/transactionStore.ts");
    const { useAccountStore } = await import("./src/stores/accountStore.ts");
    useTransactionStore.setState({ transactions: [original] });
    useAccountStore.setState({ accounts: [{ id: "account-test", name: "Test Account", market: "US" }] });
    const { default: TransactionFormModal } = await import("./src/pages/Transactions/TransactionFormModal.tsx");
    let closes = 0;
    let refreshedTransaction = null;
    let finishRefresh;
    let signalRefreshStarted;
    const refresh = new Promise((resolve) => { finishRefresh = resolve; });
    const refreshStarted = new Promise((resolve) => { signalRefreshStarted = resolve; });
    renderToStaticMarkup(React.createElement(TransactionFormModal, {
      open: true,
      transaction: original,
      onClose: () => { closes += 1; },
      onSaved: async (transaction) => {
        refreshedTransaction = transaction;
        signalRefreshStarted();
        await refresh;
        if (${refreshFails}) throw new Error("测试：刷新失败");
      },
    }));
    const submitting = onFinish({ accountId: original.account_id, symbol: ${symbol === null ? "undefined" : JSON.stringify(symbol)}, name: original.name, market: ${JSON.stringify(market)}, transactionType: ${JSON.stringify(transactionType)}, shares: 2, price: 100, totalAmount: 200, commission: 0, currency: ${JSON.stringify(currency)}, tradedAt: dayjs(original.traded_at), notes: ${JSON.stringify(notes)} });
    if (${saveFails}) {
      await submitting;
      process.stdout.write(JSON.stringify({ closes, refreshedTransaction, errors, warnings, commands, submittedPayloads, transaction: useTransactionStore.getState().transactions[0] }));
    } else {
      await Promise.race([refreshStarted, submitting]);
      const closesBeforeRefresh = closes;
      finishRefresh();
      await submitting;
      process.stdout.write(JSON.stringify({ fieldSymbol: ${symbol === null ? "null" : `symbolField.normalize?.(${JSON.stringify(symbol)}) ?? ${JSON.stringify(symbol)}`}, closesBeforeRefresh, closes, refreshedTransaction, errors, warnings, commands, submittedPayloads, transaction: useTransactionStore.getState().transactions[0] }));
    }
  `;
  return JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
}

test("editing a transaction waits for dependent data to refresh before closing", () => {
  const result = submitTransaction();
  assert.equal(result.closesBeforeRefresh, 0, "the edit modal must remain open while its refresh callback is pending");
  assert.equal(result.closes, 1);
  assert.deepEqual(result.commands, ["update_transaction"]);
  assert.equal(result.submittedPayloads[0].notes, "感觉低估aaaab");
  assert.equal(result.refreshedTransaction.shares, 2);
  assert.equal(result.refreshedTransaction.total_amount, 200);
  assert.equal(result.refreshedTransaction.notes, "感觉低估aaaab");
  assert.deepEqual(result.refreshedTransaction, result.transaction);
  assert.deepEqual(result.errors, []);
  assert.deepEqual(result.warnings, []);
});

test("failed transaction edits keep the modal open and do not refresh dependent data", () => {
  const result = submitTransaction(true);
  assert.equal(result.closes, 0);
  assert.equal(result.refreshedTransaction, null);
  assert.deepEqual(result.commands, ["update_transaction"]);
  assert.equal(result.transaction.shares, 1);
  assert.equal(result.transaction.total_amount, 100);
  assert.equal(result.submittedPayloads[0].notes, "感觉低估aaaab");
  assert.equal(result.transaction.notes, "修改前备注");
  assert.match(result.errors[0], /交易保存失败/);
});

test("a refresh failure reports the committed save and closes without treating it as a failed edit", () => {
  const result = submitTransaction(false, true);
  assert.equal(result.closesBeforeRefresh, 0);
  assert.equal(result.closes, 1);
  assert.deepEqual(result.commands, ["update_transaction"]);
  assert.equal(result.transaction.shares, 2);
  assert.equal(result.transaction.total_amount, 200);
  assert.equal(result.transaction.notes, "感觉低估aaaab");
  assert.equal(result.refreshedTransaction.notes, "感觉低估aaaab");
  assert.deepEqual(result.errors, []);
  assert.match(result.warnings[0], /交易记录已保存.*刷新.*失败/);
});

test("clearing a transaction note submits an empty string and refreshes the cleared note", () => {
  const result = submitTransaction(false, false, "");
  assert.deepEqual(result.commands, ["update_transaction"]);
  assert.equal(result.submittedPayloads[0].notes, "");
  assert.equal(result.refreshedTransaction.notes, "");
  assert.equal(result.transaction.notes, "");
  assert.equal(result.closesBeforeRefresh, 0);
  assert.equal(result.closes, 1);
  assert.deepEqual(result.errors, []);
  assert.deepEqual(result.warnings, []);
});

test("transaction submission canonicalizes only recognized mainland prefixes", () => {
  for (const [input, expected] of [["SH600036", "sh600036"], ["Sz000001", "sz000001"], ["BJ920001", "bj920001"], ["AAPL", "AAPL"], ["700.HK", "700.HK"], ["$CASH-CNY", "$CASH-CNY"], ["600036", "600036"]]) {
    assert.equal(submitTransaction(false, false, "", input, "BUY", "CN").submittedPayloads[0].symbol, expected);
  }
});

test("the transaction symbol field canonicalizes mainland prefixes during entry", () => {
  assert.equal(submitTransaction(false, false, "", "SH600036", "BUY", "CN").fieldSymbol, "sh600036");
});

test("US and HK transaction entry and submission preserve mainland-shaped symbols", () => {
  for (const [market, symbol] of [["US", "SH600036"], ["US", "Sz000001"], ["US", "BJ920001"], ["US", "AAPL"], ["HK", "SH600036"], ["HK", "Sz000001"], ["HK", "BJ920001"], ["HK", "700.HK"]]) {
    const result = submitTransaction(false, false, "", symbol, "BUY", market);
    assert.equal(result.fieldSymbol, symbol);
    assert.equal(result.submittedPayloads[0].symbol, symbol);
  }
});

test("cash transaction submission generates its cash symbol without a stock field", () => {
  assert.equal(submitTransaction(false, false, "", null, "CASH_IN").submittedPayloads[0].symbol, "$CASH-USD");
});

test("cash deposits and withdrawals follow their currency instead of the account market", () => {
  for (const transactionType of ["CASH_IN", "CASH_OUT"]) {
    for (const [accountMarket, currency, expectedMarket] of [["US", "HKD", "HK"], ["HK", "USD", "US"], ["US", "CNY", "CN"]]) {
      const result = submitTransaction(false, false, "", null, transactionType, accountMarket, currency);
      const payload = result.submittedPayloads[0];
      assert.equal(payload.market, expectedMarket);
      assert.equal(payload.currency, currency);
      assert.equal(payload.symbol, `$CASH-${currency}`);
      assert.equal(payload.transactionType, transactionType === "CASH_IN" ? "BUY" : "SELL");
      assert.deepEqual(result.errors, []);
    }
  }
});

test("a stock with zero shares can be selected when recording a dividend after liquidation", () => {
  const probe = String.raw`
    import { Window } from "happy-dom";
    import { mock } from "bun:test";
    import assert from "node:assert/strict";
    const dom = new Window();
    for (const key of ["window", "document", "navigator", "localStorage", "HTMLElement", "Element", "Node", "MutationObserver", "ResizeObserver", "ShadowRoot", "SVGElement", "getComputedStyle"]) {
      Object.defineProperty(globalThis, key, { configurable: true, value: key === "window" ? dom : key === "getComputedStyle" ? dom[key].bind(dom) : dom[key] });
    }
    globalThis.IS_REACT_ACT_ENVIRONMENT = true;
    const { act, createElement: h } = await import("react");
    const { createRoot } = await import("react-dom/client");
    const antd = await import("antd");
    mock.module("antd", () => ({ ...antd,
      Modal: ({ children }) => children,
      message: { success() {}, error: assert.fail },
    }));
    const timestamp = "2026-09-18T02:30:00.000Z";
    const account = { id: "account-test", name: "Test Account", market: "US", description: null, created_at: timestamp, updated_at: timestamp };
    const holding = { id: "holding-test", account_id: account.id, symbol: "700.HK", name: "腾讯控股", market: "HK", category_id: null, shares: 0, avg_cost: 0, currency: "HKD", created_at: timestamp, updated_at: timestamp };
    let saved;
    dom.__TAURI_INTERNALS__ = { invoke: async (command, args) => {
      if (command === "get_holdings" && args.accountId === account.id) return [holding];
      if (command === "create_transaction") {
        saved = args;
        return { id: "dividend", holding_id: holding.id, account_id: args.accountId, symbol: args.symbol, name: args.name, market: args.market, transaction_type: args.transactionType, shares: args.shares, price: args.price, total_amount: args.totalAmount, commission: args.commission, currency: args.currency, traded_at: args.tradedAt, notes: null, created_at: timestamp };
      }
      throw new Error("Unexpected command: " + command);
    } };
    const { useAccountStore } = await import("./src/stores/accountStore.ts");
    useAccountStore.setState({ accounts: [account] });
    const { default: TransactionFormModal } = await import("./src/pages/Transactions/TransactionFormModal.tsx");
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    const choose = async (field, label) => {
      await act(async () => {
        const input = document.getElementById(field);
        input.focus();
        input.dispatchEvent(new dom.MouseEvent("mousedown", { bubbles: true }));
      });
      const option = [...document.querySelectorAll(".ant-select-item-option-content")].find(item => item.textContent.includes(label));
      assert.ok(option, "Missing selectable option: " + label);
      await act(async () => option.click());
    };
    try {
      await act(async () => root.render(h(TransactionFormModal, { open: true, initialAccountId: account.id, onClose() {} })));
      await choose("symbol", "700.HK");
      assert.equal(document.getElementById("name").value, "腾讯控股");
      await choose("transactionType", "分红");
      await act(async () => {
        const amount = document.getElementById("totalAmount");
        Object.getOwnPropertyDescriptor(dom.HTMLInputElement.prototype, "value").set.call(amount, "32");
        amount.dispatchEvent(new dom.Event("input", { bubbles: true }));
      });
      await act(async () => container.querySelector("form").dispatchEvent(new dom.Event("submit", { bubbles: true, cancelable: true })));
      assert.deepEqual(
        [saved?.symbol, saved?.name, saved?.market, saved?.currency, saved?.transactionType, saved?.shares, saved?.price, saved?.totalAmount],
        ["700.HK", "腾讯控股", "HK", "HKD", "PAY", 0, 0, 32],
      );
    } finally {
      await act(async () => root.unmount());
      await dom.happyDOM.close();
    }
    // End the isolated DOM probe explicitly after assertions and teardown.
    process.exit(0);
  `;
  execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8", timeout: 15_000 });
});
