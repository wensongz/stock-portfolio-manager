// @ts-nocheck -- Runs directly in Node 26 without browser-focused Node typings.
import test from "node:test";
import assert from "node:assert/strict";
import { parseIbTransactions } from "./brokers/ibTransactions.ts";
import { parseMoomooTransactions } from "./brokers/moomooTransactions.ts";
import { parseFirstradeTransactions } from "./brokers/firstradeTransactions.ts";
import { parseIbHoldings } from "./brokers/ibHoldings.ts";
import { parseMoomooHoldings } from "./brokers/moomooHoldings.ts";
import { parseFirstradeHoldings } from "./brokers/firstradeHoldings.ts";
import { parseCnHoldings } from "./brokers/cnHoldings.ts";

test("normalizes an IB structured trade and preserves fees and HK symbols", () => {
  const csv = `Trades,Header,Acct ID,Symbol,Trade Date/Time,Quantity,Price,Proceeds,Type,Comm,Fee
Trades,Data,U1234567,00700,"2026-08-25, 10:28:37",-100,520,-52000,SELL,-5,-1`;

  assert.deepEqual(parseIbTransactions(csv, "HK"), [{
    key: "1", selected: true, transaction_type: "SELL", stock_name: "00700",
    symbol: "700.HK", traded_at: "2026-08-25T10:28:37", price: 520,
    shares: 100, total_amount: 52000, commission: 6,
  }]);
});

test("merges Moomoo sub-executions into one normalized order", () => {
  const csv = `方向,代码,名称,市场,成交数量,成交价格,成交金额,成交时间,合计费用
买入,00700,腾讯控股,港股,100,500,50000,2026/08/25 09:30:00,5
,,,,50,510,25500,2026/08/25 09:31:00,2`;

  assert.deepEqual(parseMoomooTransactions(csv, "US"), [{
    key: "0", selected: true, transaction_type: "BUY", stock_name: "腾讯控股",
    symbol: "700.HK", traded_at: "2026-08-25T09:30:00", price: 503.3333,
    shares: 150, total_amount: 75500, commission: 7,
  }]);
});

test("normalizes Firstrade trades and combines commission with fee", () => {
  const csv = `Symbol,Quantity,Price,Action,TradeDate,Amount,Commission,Fee
AAPL,10,200,BUY,2026/8/25,-2000,-1.25,-0.5`;

  assert.deepEqual(parseFirstradeTransactions(csv), [{
    key: "1", selected: true, transaction_type: "BUY", stock_name: "AAPL",
    symbol: "AAPL", traded_at: "2026-08-25T10:30:00", price: 200,
    shares: 10, total_amount: 2000, commission: 1.75,
  }]);
});

test("normalizes IB open positions and skips summary rows", () => {
  const csv = `Open Positions,Header,Symbol,Quantity,Cost Price
Open Positions,Data,00121,200,84.5
Open Positions,Data,Total,200,84.5`;

  assert.deepEqual(parseIbHoldings(csv, "HK"), {
    rows: [{ key: "0", selected: true, symbol: "121.HK", name: "00121", shares: 200, avgCost: 84.5 }],
    warnings: [],
  });
});

test("derives each Moomoo holding market from its currency", () => {
  const csv = `代码,名称,持有数量,摊薄成本价,币种
00700,腾讯控股,100,500,HKD
AAPL,Apple,2,200,USD`;

  assert.deepEqual(parseMoomooHoldings(csv, "HK"), {
    rows: [
      { key: "0", selected: true, symbol: "700.HK", name: "腾讯控股", shares: 100, avgCost: 500, currency: "HKD", market: "HK" },
      { key: "1", selected: true, symbol: "AAPL", name: "Apple", shares: 2, avgCost: 200, currency: "USD", market: "US" },
    ],
    warnings: [],
  });
});

test("normalizes Firstrade holdings and skips the total row", () => {
  const csv = `代号,名称,股数,单位成本
msft,Microsoft,3,410.25
Total,,3,410.25`;

  assert.deepEqual(parseFirstradeHoldings(csv), {
    rows: [{ key: "0", selected: true, symbol: "MSFT", name: "Microsoft", shares: 3, avgCost: 410.25 }],
    warnings: [],
  });
});

test("keeps CN holding cash detection and exchange symbol formatting", () => {
  const csv = `市种,余额,可用
人民币,279.08,279.08
证券代码,证券名称,参考持股,成本价
600519,贵州茅台,100,1400`;

  assert.deepEqual(parseCnHoldings(csv), {
    rows: [
      { key: "cash-1", selected: true, isCash: true, symbol: "$CASH-CNY", name: "现金 (CNY)", shares: 279.08, avgCost: 1 },
      { key: "0", selected: true, isCash: false, symbol: "sh600519", name: "贵州茅台", shares: 100, avgCost: 1400 },
    ],
    warnings: [],
  });
});
