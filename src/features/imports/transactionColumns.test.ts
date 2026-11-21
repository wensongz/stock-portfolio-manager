// @ts-nocheck -- Node runs a Bun probe to load the actual React column renderers.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

test("cash transfer rows show their direction and allow edits to the cash amount", () => {
  const probe = String.raw`
    import { renderToStaticMarkup } from "react-dom/server";
    import { transactionColumns } from "./src/features/imports/transactionColumns.tsx";

    const cash = { key: "cash", selected: true, isCash: true, symbol: "$CASH-CNY", stock_name: "现金 (CNY)",
      traded_at: "2026-09-25T09:30:00", price: 0, shares: 0, total_amount: 150, commission: 0 };
    const patches = [];
    const editable = transactionColumns("CN", (key, patch) => patches.push({ key, patch }), 1, true);
    const review = transactionColumns("CN", () => {}, 2, true);
    const render = (columns, key, row) => columns.find(column => column.key === key).render(undefined, row);
    const edit = { ...cash, transaction_type: "BUY" };
    const amount = render(editable, "total_amount", edit);
    amount.props.onChange(null);
    amount.props.onChange(0);
    amount.props.onChange(138.67);
    const result = {
      deposit: renderToStaticMarkup(render(editable, "transaction_type", edit)),
      withdrawal: renderToStaticMarkup(render(review, "transaction_type", { ...cash, transaction_type: "SELL" })),
      stockPurchase: renderToStaticMarkup(render(editable, "transaction_type", { ...edit, isCash: false, symbol: "sh600036" })),
      stockSymbolStillEditable: typeof render(editable, "symbol", { ...edit, isCash: false, symbol: "$CASH-" }).props.onChange,
      stockFields: ["symbol", "stock_name", "price", "shares", "commission"]
        .map(key => typeof render(editable, key, edit)?.props?.onChange),
      patches,
    };
    process.stdout.write(JSON.stringify(result));
  `;
  const result = JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
  assert.match(result.deposit, /存入现金/);
  assert.match(result.withdrawal, /提取现金/);
  assert.match(result.stockPurchase, /买入/);
  assert.equal(result.stockSymbolStillEditable, "function");
  assert.deepEqual(result.stockFields, Array(5).fill("undefined"));
  assert.deepEqual(result.patches, [{ key: "cash", patch: { total_amount: 138.67 } }]);
});
