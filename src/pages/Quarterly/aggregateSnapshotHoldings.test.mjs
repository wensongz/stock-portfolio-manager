import test from "node:test";
import assert from "node:assert/strict";
import { aggregateSnapshotHoldings } from "./aggregateSnapshotHoldings.ts";
import * as snapshotValues from "./aggregateSnapshotHoldings.ts";

const baseHolding = {
  id: "holding-1", quarterly_snapshot_id: "snapshot-1", account_id: "account-1",
  account_name: "Main", symbol: "AAPL", name: "Apple", market: "US",
  category_name: "科技", category_color: "#1677ff", shares: 10, avg_cost: 100,
  close_price: 130, market_value: 1300, cost_value: 1000, pnl: 300,
  pnl_percent: 30, weight: 13, notes: "长期持有",
};

test("同一股票代码跨账户合并，同时保留账户级快照子行", () => {
  const result = aggregateSnapshotHoldings([
    baseHolding,
    { ...baseHolding, id: "holding-2", account_id: "account-2", account_name: "Retirement", shares: 20, avg_cost: 110, market_value: 2600, cost_value: 2200, pnl: 400, pnl_percent: 18.1818, weight: 26 },
    { ...baseHolding, id: "holding-3", symbol: "MSFT", name: "Microsoft", shares: 5, avg_cost: 200, close_price: 210, market_value: 1050, cost_value: 1000, pnl: 50, pnl_percent: 5, weight: 10.5, notes: null },
  ]);

  assert.equal(result.length, 2);
  assert.equal(result[0].symbol, "AAPL");
  assert.equal(result[0].shares, 30);
  assert.equal(result[0].avg_cost, 3200 / 30);
  assert.equal(result[0].market_value, 3900);
  assert.equal(result[0].pnl, 700);
  assert.equal(result[0].pnl_percent, 21.875);
  assert.equal(result[0].weight, 39);
  assert.deepEqual(result[0].accountRows.map((row) => [row.id, row.account_name]), [["holding-2", "Retirement"], ["holding-1", "Main"]]);
});

test("跨市场持仓按统一为美元后的市值降序排列", () => {
  const result = aggregateSnapshotHoldings([
    { ...baseHolding, symbol: "CN.BABA", market: "CN", market_value: 720, cost_value: 600, pnl: 120 },
    { ...baseHolding, id: "holding-2", symbol: "AAPL", market: "US", market_value: 200, cost_value: 150, pnl: 50 },
  ], { usd_cny: 7.2, usd_hkd: 7.8, cny_hkd: 1.0833, updated_at: "2026-08-20" });

  assert.deepEqual(result.map((row) => row.symbol), ["AAPL", "CN.BABA"]);
  assert.deepEqual(result.map((row) => row.market_value_base), [200, 100]);
});

const rates = { usd_cny: 7, usd_hkd: 7.8, cny_hkd: 7.8 / 7, updated_at: "2026-03-31" };

test("港股账户的美元现金按实际币种折算，保留负余额和账户笔记", () => {
  const result = aggregateSnapshotHoldings([
    { ...baseHolding, symbol: "$CASH-USD", market: "HK", currency: "USD", shares: -200, avg_cost: 1, close_price: 1, market_value: -200, cost_value: -200, pnl: 0, weight: -25 },
    { ...baseHolding, market_value: 1000, cost_value: 1000, pnl: 0 },
  ], rates);

  assert.equal(result.reduce((total, row) => total + row.market_value_base, 0), 800);
  const cash = result.find((row) => row.symbol === "$CASH-USD");
  assert.equal(cash.market_value_base, -200);
  assert.equal(cash.avg_cost, 1);
  assert.equal(cash.accountRows[0].notes, "长期持有");
});

test("旧现金快照从现金代码恢复币种，纯现金季度可汇总", () => {
  const [cash] = aggregateSnapshotHoldings([
    { ...baseHolding, symbol: "$CASH-USD", market: "HK", currency: "", shares: 500, avg_cost: 1, close_price: 1, market_value: 500, cost_value: 500, pnl: 0 },
  ], rates);
  assert.equal(cash.currency, "USD");
  assert.equal(cash.market_value_base, 500);
});

test("相同代码的不同市场或币种不混合原币金额", () => {
  const result = aggregateSnapshotHoldings([
    { ...baseHolding, symbol: "SAME", market: "US", currency: "USD" },
    { ...baseHolding, id: "hk", symbol: "SAME", market: "HK", currency: "HKD" },
    { ...baseHolding, id: "usd", symbol: "SAME", market: "HK", currency: "USD" },
  ], rates);
  assert.equal(result.length, 3);
  assert.equal(new Set(result.map((row) => row.id)).size, 3);
});

test("整体分类把美元和人民币折成美元，市场分类按实际币种折成市场本币", () => {
  const holdings = [
    { ...baseHolding, market: "US", currency: "USD", market_value: 100 },
    { ...baseHolding, symbol: "CN", market: "CN", currency: "CNY", market_value: 700 },
    { ...baseHolding, symbol: "HK", market: "HK", currency: "HKD", market_value: 780 },
    { ...baseHolding, symbol: "$CASH-USD", market: "HK", currency: "USD", category_name: "现金类", market_value: 100 },
  ];
  const overall = snapshotValues.buildSnapshotComposition(holdings, rates);
  assert.equal(overall.currency, "USD");
  assert.equal(overall.total, 400);
  assert.deepEqual(overall.categories.map(({ name, value }) => [name, value]), [["现金类", 100], ["科技", 300]]);
  const hongKong = snapshotValues.buildSnapshotComposition(holdings, rates, "HK");
  assert.equal(hongKong.currency, "HKD");
  assert.equal(hongKong.total, 1560);
  assert.deepEqual(hongKong.pieSlices.map(({ value }) => value), [780, 780]);
});

test("负现金保留在分类净额中，饼图不丢弃负数或取绝对值", () => {
  const result = snapshotValues.buildSnapshotComposition([
    { ...baseHolding, currency: "USD", market_value: 1000 },
    { ...baseHolding, currency: "USD", symbol: "$CASH-USD", category_name: "现金类", market_value: -200 },
  ], rates);
  assert.equal(result.total, 800);
  assert.deepEqual(result.categories.map(({ name, value }) => [name, value]), [["现金类", -200], ["科技", 1000]]);
  assert.equal(result.hasNegativeValues, true);
  assert.deepEqual(result.pieSlices, []);
});

test("零余额没有饼图，纯现金正余额展示完整分类", () => {
  const cash = { ...baseHolding, symbol: "$CASH-USD", currency: "USD", category_name: "现金类", market_value: 0 };
  assert.deepEqual(snapshotValues.buildSnapshotComposition([cash], rates).pieSlices, []);
  const positive = snapshotValues.buildSnapshotComposition([{ ...cash, market_value: 500 }], rates);
  assert.deepEqual(positive.pieSlices.map(({ name, value }) => [name, value]), [["现金类", 500]]);
});

test("缺失或无效历史汇率不伪造跨币种总额，原币明细保留", () => {
  for (const saved of [undefined, "{}", "bad json", '{"usd_cny":7,"usd_hkd":0,"cny_hkd":1}', '{"usd_cny":1e999}']) {
    assert.equal(snapshotValues.parseSnapshotExchangeRates(saved), null);
  }
  const holdings = [{ ...baseHolding, market: "CN", currency: "CNY", market_value: 700 }];
  const [row] = aggregateSnapshotHoldings(holdings, null);
  assert.equal(row.market_value, 700);
  assert.equal(row.market_value_base, null);
  const overall = snapshotValues.buildSnapshotComposition(holdings, null);
  assert.equal(overall.total, null);
  assert.equal(overall.hasMissingRates, true);
  assert.deepEqual(overall.pieSlices, []);
  assert.equal(snapshotValues.buildSnapshotComposition(holdings, null, "CN").total, 700);
  assert.equal(snapshotValues.buildSnapshotComposition([{ ...baseHolding, currency: "USD", market_value: 100 }], null).total, 100);
});

test("零值外币现金无需汇率也保留零合计和明细", () => {
  const holdings = [{ ...baseHolding, symbol: "$CASH-CNY", market: "CN", currency: "CNY", shares: 0, market_value: 0, cost_value: 0, pnl: 0 }];
  assert.equal(aggregateSnapshotHoldings(holdings, null)[0].market_value_base, 0);
  const composition = snapshotValues.buildSnapshotComposition(holdings, null);
  assert.equal(composition.total, 0);
  assert.equal(composition.hasMissingRates, false);
});

test("市场本币分布和美元总额沿用同一美元汇率，避免交叉汇率舍入差异", () => {
  const roundedRates = { ...rates, cny_hkd: 1.11 };
  const holdings = [
    { ...baseHolding, symbol: "$CASH-CNY", market: "HK", currency: "CNY", category_name: "现金类", market_value: 700 },
    { ...baseHolding, symbol: "$CASH-USD", market: "HK", currency: "USD", category_name: "现金类", market_value: 100 },
  ];
  assert.equal(snapshotValues.buildSnapshotComposition(holdings, roundedRates).total, 200);
  assert.equal(snapshotValues.buildSnapshotComposition(holdings, roundedRates, "HK").total, 1560);
  assert.equal(snapshotValues.buildSnapshotComposition([{ ...baseHolding, market: "CN", currency: "HKD", market_value: 780 }], roundedRates, "CN").total, 700);
});

test("旧快照仅保存人民币汇率时可折算人民币，港币换算仍明确缺失", () => {
  const partialRates = snapshotValues.parseSnapshotExchangeRates('{"usd_cny":7}');
  const cny = { ...baseHolding, market: "CN", currency: "CNY", market_value: 700 };
  const hkd = { ...baseHolding, market: "HK", currency: "HKD", market_value: 780 };
  assert.equal(aggregateSnapshotHoldings([cny], partialRates)[0].market_value_base, 100);
  assert.equal(snapshotValues.buildSnapshotComposition([cny], partialRates).total, 100);
  assert.equal(aggregateSnapshotHoldings([hkd], partialRates)[0].market_value_base, null);
  assert.equal(snapshotValues.buildSnapshotComposition([hkd], partialRates).hasMissingRates, true);
  assert.equal(snapshotValues.buildSnapshotComposition([{ ...cny, market: "HK" }], partialRates, "HK").total, null);
});

test("美元基准汇率可用时不依赖未保存或舍入的人民币港币交叉汇率", () => {
  const cash = { ...baseHolding, symbol: "$CASH-CNY", market: "HK", currency: "CNY", market_value: 700 };
  for (const saved of ['{"usd_cny":7,"usd_hkd":7.8}', '{"usd_cny":7,"usd_hkd":7.8,"cny_hkd":1.11}']) {
    const parsed = snapshotValues.parseSnapshotExchangeRates(saved);
    assert.equal(snapshotValues.buildSnapshotComposition([cash], parsed, "HK").total, 780);
  }
});

test("所需汇率为非数字、非正数或无限值时不输出无效金额", () => {
  const cash = { ...baseHolding, symbol: "$CASH-CNY", market: "CN", currency: "CNY", market_value: 700 };
  for (const usd_cny of ["7", 0, -7, Infinity]) {
    const parsed = snapshotValues.parseSnapshotExchangeRates(JSON.stringify({ usd_cny }));
    assert.equal(snapshotValues.buildSnapshotComposition([cash], parsed).total, null);
    assert.equal(snapshotValues.buildSnapshotComposition([cash], { usd_cny }).total, null);
  }
});
