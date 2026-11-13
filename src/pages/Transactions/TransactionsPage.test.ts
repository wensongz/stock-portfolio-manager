// @ts-nocheck -- Node drives a Bun SSR probe against the actual page and table.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

test("transaction tags distinguish cash opening balances from deposits and withdrawals", () => {
  const probe = String.raw`
    globalThis.localStorage = { getItem: () => null, setItem() {}, removeItem() {} };
    const React = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { Window } = await import("happy-dom");
    const { mock } = await import("bun:test");
    const base = {
      holding_id: "holding-test", account_id: "account-test", name: "现金 (CNY)", market: "CN",
      shares: 636.94, price: 1, total_amount: 636.94, commission: 0, currency: "CNY",
      traded_at: "2026-11-08T09:49:00", notes: null, created_at: "2026-11-08T09:49:00",
    };
    const transactions = [
      { ...base, id: "cash-opening-cny", symbol: "$CASH-CNY", transaction_type: "OPEN" },
      { ...base, id: "cash-opening-usd", symbol: "$CASH-USD", currency: "USD", market: "US", transaction_type: "OPEN" },
      { ...base, id: "cash-opening-hkd", symbol: "$CASH-HKD", currency: "HKD", market: "HK", transaction_type: "OPEN" },
      { ...base, id: "cash-deposit", symbol: "$CASH-CNY", transaction_type: "BUY" },
      { ...base, id: "cash-withdrawal", symbol: "$CASH-CNY", transaction_type: "SELL" },
      { ...base, id: "stock-opening", symbol: "sh600036", name: "招商银行", transaction_type: "OPEN" },
      { ...base, id: "stock-buy", symbol: "sh600036", name: "招商银行", transaction_type: "BUY" },
      { ...base, id: "stock-sell", symbol: "sh600036", name: "招商银行", transaction_type: "SELL" },
    ];
    mock.module("./src/stores/transactionStore.ts", () => ({ useTransactionStore: () => ({
      transactions, loading: false, fetchTransactions: async () => {}, deleteTransaction: async () => {},
    }) }));
    const { default: TransactionsPage } = await import("./src/pages/Transactions/index.tsx");
    const dom = new Window();
    dom.document.body.innerHTML = renderToStaticMarkup(React.createElement(TransactionsPage));
    const rows = Object.fromEntries([...dom.document.querySelectorAll("tr[data-row-key]")].map(row => {
      const tag = row.querySelectorAll("td")[3].querySelector(".ant-tag");
      return [row.getAttribute("data-row-key"), { label: tag.textContent, className: tag.className }];
    }));
    await dom.happyDOM.close();
    process.stdout.write(JSON.stringify(rows));
  `;
  const rows = JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));

  for (const [id, label, color] of [
    ["cash-opening-cny", "期初", "blue"],
    ["cash-opening-usd", "期初", "blue"],
    ["cash-opening-hkd", "期初", "blue"],
    ["cash-deposit", "存入", "green"],
    ["cash-withdrawal", "提取", "red"],
    ["stock-opening", "建仓", "blue"],
    ["stock-buy", "买入", "green"],
    ["stock-sell", "卖出", "red"],
  ]) {
    assert.equal(rows[id]?.label, label, `${id} transaction label`);
    assert.ok(rows[id].className.split(" ").includes(`ant-tag-${color}`), `${id} tag color`);
  }
});
