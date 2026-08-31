// @ts-nocheck -- Runs directly in Node 26 without browser-focused Node typings.
import test from "node:test";
import assert from "node:assert/strict";
import { parseThsCsv } from "./thsCsvParser.ts";

test("preserves legacy THS operation and cash-flow parsing", () => {
  const csv = `成交日期,成交时间,证券代码,证券名称,操作,成交数量,成交价格,成交金额,发生金额,手续费,印花税,附加费,交易所名称,过户费
20260825,10:28:37,601318,中国平安,卖出,1000,54.94,54940,54906.20,5.79,27.48,0,上海Ａ股,0.53
20260709,17:00:00,600036,招商银行,股息入账,0,1.003,130344.87,130344.87,0,0,0,上海Ａ股,0`;

  const rows = parseThsCsv(csv);

  assert.equal(rows.length, 2);
  assert.deepEqual(rows[0], {
    key: "0",
    selected: true,
    transaction_type: "SELL",
    symbol: "sh601318",
    stock_name: "中国平安",
    traded_at: "2026-08-25T10:28:37",
    price: 54.94,
    shares: 1000,
    total_amount: 54940,
    commission: 33.8,
    notes: undefined,
  });
  assert.deepEqual(rows[1], {
    key: "1",
    selected: true,
    transaction_type: "PAY",
    symbol: "sh600036",
    stock_name: "招商银行",
    traded_at: "2026-07-09T17:00:00",
    price: 0,
    shares: 0,
    total_amount: 130344.87,
    commission: 0,
    notes: "分红派息",
  });
});

test("keeps the legacy cash-flow fallback when no operation column is available", () => {
  const csv = `成交日期,成交时间,证券代码,证券名称,成交数量,成交价格,成交金额,发生金额,交易所名称
20260825,10:28:37,601318,中国平安,1000,54.94,54940,54906.20,上海Ａ股
20260825,10:29:12,600036,招商银行,1400,39.64,55496,-55502.40,上海Ａ股`;

  assert.deepEqual(
    parseThsCsv(csv).map((row) => row.transaction_type),
    ["SELL", "BUY"],
  );
});

test("recognizes CITIC action and settlement-amount column aliases", () => {
  const csv = `\uFEFF成交日期,成交时间,证券代码,证券名称,买卖标志,成交数量,成交价格,成交金额,股份余额,委托编号,币种类别,成交编号,手续费,印花税,附加费,清算金额,备注,交易所名称,清算日期,申请编号,委托日期,资金帐号,客户代码,股东姓名,过户费,交易所清算费,基金手续费,真实操作
20260709,17:00:00,600000,样本沪股一,红利,0,0.1,100,1000,0,人民币,0,0,0,0,100,股息入账:样本沪股一600000,上海Ａ股,20260709,0,20260709,acct,customer,测试用户,0,0,0,
20260713,13:20:19,601318,样本沪股二,买入,100,10,1000,100,1,人民币,1,1,0,0,-1001.1,"买入601318,数量100.00",上海Ａ股,20260713,request,20260713,acct,customer,测试用户,0.1,0,0,0
20260714,13:29:54,000001,样本深股一,买入,200,5,1000,200,2,人民币,2,1,0,0,-1001.1,"买入000001,数量200.00",深圳Ａ股,20260714,request,20260714,acct,customer,测试用户,0.1,0,0,0
20260715,17:00:00,000002,样本深股二,红利,0,0.2,200,2000,0,人民币,0,0,0,0,200,股息入账:样本深股二000002,深圳Ａ股,20260715,0,20260715,acct,customer,测试用户,0,0,0,
20260727,09:57:12,511880,样本基金一,买入,10,100,1000,10,3,人民币,3,0,0,0,-1000,"买入511880,数量10.00",上海Ａ股,20260727,request,20260727,acct,customer,测试用户,0,0,0,0
20260730,17:00:00,000003,样本深股三,红利,0,0.3,300,3000,0,人民币,0,0,0,0,300,股息入账:样本深股三000003,深圳Ａ股,20260730,0,20260730,acct,customer,测试用户,0,0,0,
20260812,17:00:00,000004,样本深股四,红利,0,0.4,400,4000,0,人民币,0,0,0,0,400,股息入账:样本深股四000004,深圳Ａ股,20260812,0,20260812,acct,customer,测试用户,0,0,0,
20260818,17:00:00,000005,样本深股五,红利,0,0.5,500,5000,0,人民币,0,0,0,0,500,股息入账:样本深股五000005,深圳Ａ股,20260818,0,20260818,acct,customer,测试用户,0,0,0,
20260825,10:29:12,600001,样本沪股三,买入,100,10,1000,100,4,人民币,4,1,0,0,-1001.1,"买入600001,数量100.00",上海Ａ股,20260825,request,20260825,acct,customer,测试用户,0.1,0,0,0
20260825,10:28:37,600002,样本沪股四,卖出,100,12,1200,0,5,人民币,5,1,1.2,0,1197.7,"卖出600002,数量100.00",上海Ａ股,20260825,request,20260825,acct,customer,测试用户,0.1,0,0,0
20260825,10:14:38,000006,样本深股六,买入,100,10,1000,100,6,人民币,6,1,0,0,-1001.1,"买入000006,数量100.00",深圳Ａ股,20260825,request,20260825,acct,customer,测试用户,0.1,0,0,0
20260825,10:14:23,000007,样本深股七,卖出,100,12,1200,0,7,人民币,7,1,1.2,0,1197.7,"卖出000007,数量100.00",深圳Ａ股,20260825,request,20260825,acct,customer,测试用户,0.1,0,0,0`;

  const rows = parseThsCsv(csv);

  assert.equal(rows.length, 12);
  assert.deepEqual(
    rows.reduce(
      (counts, row) => ({ ...counts, [row.transaction_type]: counts[row.transaction_type] + 1 }),
      { BUY: 0, SELL: 0, PAY: 0 },
    ),
    { BUY: 5, SELL: 2, PAY: 5 },
  );
  assert.equal(rows[0].shares, 0);
  assert.equal(rows[0].price, 0);
  assert.equal(rows[0].notes, "分红派息");
  assert.equal(rows[1].commission, 1.1);
  assert.equal(rows[9].transaction_type, "SELL");
  assert.equal(rows[9].commission, 2.3);
  assert.equal(rows[11].symbol, "sz000007");
});

test("prefers an explicit buy or sell action over the cash-flow sign", () => {
  const csv = `成交日期,成交时间,证券代码,证券名称,买卖标志,成交数量,成交价格,成交金额,清算金额,交易所名称
20260825,10:28:37,601318,中国平安,卖出,1000,54.94,54940,-54906.2,上海Ａ股
20260825,10:29:12,600036,招商银行,买入,1400,39.64,55496,55502.4,上海Ａ股`;

  assert.deepEqual(
    parseThsCsv(csv).map((row) => row.transaction_type),
    ["SELL", "BUY"],
  );
});

test("falls back to trade amount when a dividend has no cash-flow column", () => {
  const csv = `成交日期,成交时间,证券代码,证券名称,买卖标志,成交数量,成交价格,成交金额,交易所名称
20260730,17:00:00,002594,比亚迪,红利,0,96.2,2362.8,深圳Ａ股`;

  const rows = parseThsCsv(csv);

  assert.equal(rows.length, 1);
  assert.equal(rows[0].transaction_type, "PAY");
  assert.equal(rows[0].total_amount, 2362.8);
});
