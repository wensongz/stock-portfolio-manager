// @ts-nocheck -- Load the real TSX component and Ant Design table through Vite.
import assert from "node:assert/strict";
import test, { after, before } from "node:test";
import React from "react";
import { renderToStaticMarkup } from "react-dom/server";
import react from "@vitejs/plugin-react";
import { Window } from "happy-dom";
import { createServer } from "vite";

let server;
let ClosedOptionDetails;
let dom;
let previousLocalStorageDescriptor;

const closingRecord = {
  id: "baba-close",
  code: "C",
  quantity: 10,
  price: 5.2,
  amount: 5200,
  commission: 4.44,
  fee: 0,
  traded_at: "2024-12-04, 10:30:00",
};

const closeDetails = {
  records: [closingRecord],
  close_cost: 5200,
  fees: 9.34,
  realized_pnl: 6090.66,
};

before(async () => {
  previousLocalStorageDescriptor = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    value: {
      getItem(key) { return key === "pnl_color_scheme" ? "green-up" : null; },
      setItem() {},
    },
  });
  dom = new Window();
  server = await createServer({
    appType: "custom",
    cacheDir: "/tmp/stock-portfolio-manager-closed-option-details-vite-cache",
    configFile: false,
    plugins: [react()],
    root: process.cwd(),
    server: { hmr: false, middlewareMode: true, watch: null, ws: false },
  });
  ({ default: ClosedOptionDetails } = await server.ssrLoadModule(
    "/src/pages/Options/ClosedOptionDetails.tsx",
  ));
});

after(async () => {
  await server?.close();
  await dom?.happyDOM.close();
  if (previousLocalStorageDescriptor) {
    Object.defineProperty(globalThis, "localStorage", previousLocalStorageDescriptor);
  } else {
    delete globalThis.localStorage;
  }
});

function renderDetails(details, currency = "USD") {
  dom.document.body.innerHTML = renderToStaticMarkup(
    React.createElement(ClosedOptionDetails, { details, openAmount: 11300, currency }),
  );
  return dom.document.body;
}

function recordRows(container) {
  return [...container.querySelectorAll("tbody tr[data-row-key]")].map(row =>
    [...row.querySelectorAll("td")].map(cell => cell.textContent),
  );
}

function pnlSummary(container) {
  return [...container.querySelectorAll("strong")].find(element =>
    element.textContent.includes("已实现净盈亏"),
  );
}

test("shows the actual BABA buyback and its net profit after all fees", () => {
  const container = renderDetails(closeDetails);
  assert.deepEqual(recordRows(container), [[
    "2024-12-04, 10:30:00", "10", "$5.20", "$5,200.00", "$4.44", "买入平仓",
  ]]);
  assert.match(container.textContent, /开仓收入[：\s]*\$11,300\.00/);
  assert.match(container.textContent, /平仓支出[：\s]*\$5,200\.00/);
  assert.match(container.textContent, /总费用[：\s]*\$9\.34/);
  const pnl = pnlSummary(container);
  assert.ok(pnl, "net profit is emphasized in the summary");
  assert.match(pnl.textContent, /\+\$6,090\.66/);
  assert.equal((pnl.parentElement.style.color || pnl.style.color).toLowerCase(), "#22c55e");
});

test("shows signed losses in the account currency and preferred loss color", () => {
  const container = renderDetails({
    records: [{ ...closingRecord, price: 12, amount: 12000 }],
    close_cost: 12000,
    fees: 9.34,
    realized_pnl: -709.34,
  }, "HKD");
  const pnl = pnlSummary(container);
  assert.ok(pnl, "net loss is emphasized in the summary");
  assert.match(pnl.textContent, /-HK\$709\.34/);
  assert.doesNotMatch(pnl.textContent, /\+/);
  assert.equal((pnl.parentElement.style.color || pnl.style.color).toLowerCase(), "#ef4444");
  assert.match(container.textContent, /平仓支出[：\s]*HK\$12,000\.00/);
});

test("keeps every closing record and identifies expiry and assignment", () => {
  const container = renderDetails({
    ...closeDetails,
    records: [
      { ...closingRecord, id: "buyback", code: "C;P", quantity: 4, amount: 2080, commission: 1.776, fee: 0.224 },
      { ...closingRecord, id: "expiry", code: "C;Ep", quantity: 3, price: 0, amount: 0, commission: 0, traded_at: "2024/12/20" },
      { ...closingRecord, id: "assignment", code: "A", quantity: 3, price: 0, amount: 0, commission: 0, traded_at: null },
    ],
  });
  assert.deepEqual(recordRows(container), [
    ["2024-12-04, 10:30:00", "4", "$5.20", "$2,080.00", "$2.00", "买入平仓"],
    ["2024-12-20", "3", "$0.00", "$0.00", "$0.00", "到期"],
    ["—", "3", "$0.00", "$0.00", "$0.00", "被执行"],
  ]);
});

for (const details of [null, undefined, { records: [], close_cost: 0, fees: 0, realized_pnl: 0 }]) {
  test(`does not invent zero profit when closing details are ${details === null ? "null" : details === undefined ? "undefined" : "empty"}`, () => {
    const container = renderDetails(details);
    assert.match(container.textContent, /暂无平仓明细/);
    assert.equal(container.querySelector("table"), null);
    assert.doesNotMatch(container.textContent, /已实现净盈亏|\$0\.00/);
  });
}
