// @ts-nocheck -- Runs directly in Node 26 without browser-focused Node typings.
import test from "node:test";
import assert from "node:assert/strict";
import { parseIbHoldings } from "./brokers/ibHoldings.ts";
import { parseMoomooHoldings } from "./brokers/moomooHoldings.ts";
import { parseFirstradeHoldings } from "./brokers/firstradeHoldings.ts";
import { parseCnHoldings } from "./brokers/cnHoldings.ts";

const formats = [
  {
    name: "IB", parse: (text) => parseIbHoldings(text, "US"),
    header: "Symbol,Quantity,Cost Price", good: "AAPL,2,200", symbol: "AAPL",
    bad: [",2,200", "MSFT,2x,100", "NVDA,2,100usd", "AMZN,Infinity,10", "META,2,", "BAD!,2,10", "ORCL,Data,10"],
    ignored: ["Total,10,500", "Stocks,,", "USD,,", "Symbol,Quantity,Cost Price", "SHORT,-2,20", "CLOSED,0,"],
  },
  {
    name: "Moomoo", parse: (text) => parseMoomooHoldings(text, "US"),
    header: "代码,名称,持有数量,摊薄成本价,币种", good: "AAPL,Apple,2,200,USD", symbol: "AAPL",
    bad: [",Missing,2,200,USD", "MSFT,Microsoft,2x,100,USD", "NVDA,Nvidia,2,100usd,USD", "AMZN,Amazon,Infinity,10,USD", "META,Meta,2,,USD", "BAD!,Bad,2,10,USD"],
    ignored: ["合计,,10,500,USD", "美股,,,,", "USD,,,,", "代码,名称,持有数量,摊薄成本价,币种", "SHORT,Short,-2,20,USD", "CLOSED,Closed,0,,USD"],
  },
  {
    name: "Firstrade", parse: parseFirstradeHoldings,
    header: "代号,名称,股数,单位成本", good: "AAPL,Apple,2,200", symbol: "AAPL",
    bad: [",Missing,2,200", "MSFT,Microsoft,2x,100", "NVDA,Nvidia,2,100usd", "AMZN,Amazon,Infinity,10", "META,Meta,2,", "12345,Invalid,2,10"],
    ignored: ["Total,,10,500", "Summary,,10,500", "USD,,,", "代号,名称,股数,单位成本", "SHORT,Short,-2,20", "CLOSED,Closed,0,"],
  },
  {
    name: "CN", parse: parseCnHoldings,
    header: "证券代码,证券名称,参考持股,成本价", good: "600519,贵州茅台,2,200", symbol: "sh600519",
    bad: [",缺少代码,2,200", "000001,平安银行,2x,100", "000002,万科,2,100元", "600036,招商银行,Infinity,10", "600000,浦发银行,2,", "600519X,无效代码,2,10"],
    ignored: ["合计,,10,500", "股票,,,", "人民币,,,", "证券代码,证券名称,参考持股,成本价", "000001,空头,-2,20", "000002,空仓,0,"],
  },
];

for (const format of formats) {
  test(`${format.name} reports each malformed holding with its physical line and original content`, () => {
    const lines = ["\uFEFF报告", format.header, format.good, "", ...format.bad, ...format.ignored, "持仓明细"];
    const result = format.parse(lines.join("\r\n"));
    assert.deepEqual(result.rows.map((row) => row.symbol), [format.symbol]);
    assert.deepEqual(result.issues?.map(({ line, raw }) => ({ line, raw })),
      format.bad.map((raw, index) => ({ line: index + 5, raw })));
    assert.ok(result.issues.every((issue) => issue.message.trim().length > 0));
  });

  test(`${format.name} retains diagnostics when every candidate holding is malformed`, () => {
    const result = format.parse(`${format.header}\n${format.bad[0]}`);
    assert.equal(result.rows.length, 0);
    assert.equal(result.issues?.length, 1);
    assert.equal(result.issues[0].line, 2);
    assert.equal(result.issues[0].raw, format.bad[0]);
  });

  test(`${format.name} does not add empty diagnostics for an unchanged valid import`, () => {
    const result = format.parse(`${format.header}\n${format.good}`);
    assert.equal(Object.hasOwn(result, "issues"), false);
  });
}

test("IB holdings preserve share-class symbols with a space", () => {
  const result = parseIbHoldings("Symbol,Quantity,Cost Price\nBRK B,2,400", "US");
  assert.equal(result.rows[0]?.symbol, "BRK B");
  assert.equal(result.issues, undefined);
});

test("currency and category words remain valid US tickers when they carry holding values", () => {
  const moomoo = parseMoomooHoldings("代码,持有数量,摊薄成本价,币种\nUSD,2,40,USD", "US");
  const firstrade = parseFirstradeHoldings("代号,股数,单位成本\nUSD,2,40\nCASH,3,50");
  assert.deepEqual(moomoo.rows.map((row) => row.symbol), ["USD"]);
  assert.deepEqual(firstrade.rows.map((row) => row.symbol), ["USD", "CASH"]);
  assert.equal(moomoo.issues, undefined);
  assert.equal(firstrade.issues, undefined);
});

test("IB structured holding diagnostics stay within stock blocks and respect repeated headers", () => {
  const lines = [
    "Open Positions,Header,DataDiscriminator,Asset Category,Currency,Symbol,Quantity,Cost Price",
    "Open Positions,Data,Summary,Stocks,USD,AAPL,2,200",
    "Open Positions,Data,Summary,Stocks,USD,MSFT,bad,100",
    "Open Positions,Data,Summary,Stocks,USD,,2,100",
    "Open Positions,Data,Summary,Stocks,USD,Total,4,600",
    "Open Positions,Data,Summary,Options,USD,AAPL 261218C00200000,1,bad",
    "Open Positions,Data,Summary,Forex,USD,EUR,10,bad",
    "Open Positions,Total,Summary,Stocks,USD,,4,600",
    "Dividends,Header,Currency,Date,Description,Amount",
    "Dividends,Data,USD,2026-09-01,AAPL,20",
    "Open Positions,Header,Asset Category,Currency,Symbol,Cost Price,Quantity",
    "Open Positions,Data,Stocks,USD,NVDA,50,3",
    "Open Positions,Data,Stocks,USD,META,bad,2",
  ];
  const result = parseIbHoldings(lines.join("\n"), "US");
  assert.deepEqual(result.rows.map(({ symbol, shares, avgCost }) => ({ symbol, shares, avgCost })), [
    { symbol: "AAPL", shares: 2, avgCost: 200 },
    { symbol: "NVDA", shares: 3, avgCost: 50 },
  ]);
  assert.deepEqual(result.issues?.map(({ line, raw }) => ({ line, raw })), [
    { line: 3, raw: lines[2] }, { line: 4, raw: lines[3] }, { line: 13, raw: lines[12] },
  ]);
  assert.equal(new Set(result.rows.map((row) => row.key)).size, result.rows.length);
});
