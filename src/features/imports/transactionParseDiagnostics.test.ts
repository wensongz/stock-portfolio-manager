// @ts-nocheck -- Runs directly in Node 26 without browser-focused Node typings.
import test from "node:test";
import assert from "node:assert/strict";
import { parseIbTransactions } from "./brokers/ibTransactions.ts";
import { parseMoomooTransactions } from "./brokers/moomooTransactions.ts";
import { parseFirstradeTransactions } from "./brokers/firstradeTransactions.ts";
import { parseThsCsv } from "../../pages/Transactions/thsCsvParser.ts";

test("IB retains valid trades and reports original lines for malformed trades", () => {
  const lines = [
    "Acct ID,Symbol,Trade Date/Time,Quantity,Price,Proceeds,Type,Comm,Fee",
    'U1234567,133.00,"2026-09-17, 01:33:50",-6000,24.62,147720,SELL,-88.41,-152.21',
    'U1234567,87.00,"2026-09-22, 21:44:46",oops,15.04,150400,SELL,-90.01,-156.29',
    "U1234567,,2026-09-22,10,15,150,BUY,0,0",
    "U1234567,AAPL,2026-02-30,10,15,150,BUY,0,0",
    "Total,,,,,,,,",
  ];
  const issues = [];
  const rows = parseIbTransactions(lines.join("\n"), "HK", issues);
  assert.deepEqual(issues.map(({ line, raw }) => ({ line, raw })), [
    { line: 3, raw: lines[2] }, { line: 4, raw: lines[3] }, { line: 5, raw: lines[4] },
  ]);
  assert.match(issues[0].message, /数量/);
  assert.match(issues[1].message, /代码/);
  assert.match(issues[2].message, /日期|时间/);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].symbol, "133.HK");
  assert.equal(rows[0].commission, 240.62);
});

test("IB reports all-invalid tables once and excludes currencies, subtotals and non-stock blocks", () => {
  const lines = [
    "Trades,Header,DataDiscriminator,Asset Category,Currency,Symbol,Date/Time,Quantity,T. Price,Proceeds,Comm/Fee",
    "Trades,Data,Order,Stocks,USD,AAPL,2026-09-17,broken,10,-100,-1",
    "Trades,Data,SubTotal,Stocks,USD,AAPL,,10,10,-100,-1",
    "Trades,Data,Total,Stocks,USD,,,10,10,-100,-1",
    "Trades,Data,Order,Stocks,HKD,,,,,,",
    "Trades,Data,Order,Options,USD,AAPL OPTION,2026-09-17,1,10,-100,-1",
    "Deposits & Withdrawals,Data,USD,2026-09-17,Deposit,100",
  ];
  const issues = [];
  const rows = parseIbTransactions(lines.join("\n"), "US", issues);
  assert.deepEqual(rows, []);
  assert.equal(issues.length, 1);
  assert.equal(issues[0].line, 2);
  assert.equal(issues[0].raw, lines[1]);
});

test("IB diagnoses truncated flat trade rows with recognized accounts while ignoring report footers", () => {
  const lines = [
    "Acct ID,Symbol,Trade Date/Time,Quantity,Price",
    "U1234567,133,2026-09-17,10,24.62",
    "U1234567,133",
    "U1234567",
    "End of Report",
    "Report generated,2026-09-17",
  ];
  const issues = [];
  const rows = parseIbTransactions(lines.join("\n"), "HK", issues);
  assert.equal(rows.length, 1);
  assert.deepEqual(issues.map(({ line, raw }) => ({ line, raw })), [
    { line: 3, raw: "U1234567,133" },
    { line: 4, raw: "U1234567" },
  ]);
  assert.match(issues[0].message, /数量/);
  assert.match(issues[1].message, /代码/);
});

test("IB diagnoses malformed dividend rows without treating report totals as dividends", () => {
  const issues = [];
  const rows = parseIbTransactions('Date,Description,Amount\n2026-09-17,AAPL(US123) Cash Dividend USD 0.5,100\n2026-09-18,AAPL(US123) Cash Dividend USD 0.5,invalid\n,Total Dividends,100\n2026-09-18,USD Credit Interest,5', "US", issues);
  assert.equal(rows.length, 1);
  assert.equal(issues.length, 1);
  assert.equal(issues[0].line, 3);
  assert.match(issues[0].message, /金额/);
});

test("IB dividend aliases do not admit invalid multi-word US symbols", () => {
  for (const input of ["bad symbol", "BRK BB", "1BRK B", "BRK.B C", "BRK B EXTRA"]) {
    const raw = `2026-09-17,${input}(US123) Cash Dividend USD 0.5,50`;
    const issues = [];
    const rows = parseIbTransactions(`Date,Description,Amount\n${raw}`, "US", issues);
    assert.deepEqual(rows, [], input);
    assert.equal(issues.length, 1, input);
    assert.equal(issues[0].raw, raw, input);
    assert.match(issues[0].message, /代码/, input);
  }
});

test("IB dividend share-class parsing preserves HK symbol extraction rules", () => {
  const valid = "2026-09-17,00133.00(HK123) Cash Dividend HKD 0.5,50";
  const invalid = "2026-09-17,133 B(HK123) Cash Dividend HKD 0.5,50";
  const issues = [];
  const rows = parseIbTransactions(`Date,Description,Amount\n${valid}\n${invalid}`, "HK", issues);
  assert.deepEqual(rows.map(row => [row.symbol, row.raw]), [["133.HK", valid]]);
  assert.equal(issues.length, 1);
  assert.equal(issues[0].raw, invalid);
  assert.match(issues[0].message, /代码/);
});

test("Moomoo diagnoses invalid fills and never attaches a new order's fills to the preceding order", () => {
  const lines = [
    "方向,代码,名称,市场,成交数量,成交价格,成交金额,成交时间,合计费用",
    "买入,AAPL,Apple,美股,1,100,100,2026/09/17 09:30:00,0",
    "卖出,MSFT,Microsoft,美股,bad,200,200,2026/09/17 09:31:00,0",
    ",,,,2,200,400,2026/09/17 09:32:00,0",
    ",,,,bad,200,400,2026/09/17 09:32:00,0",
    "合计,,,,3,,500,,",
  ];
  const issues = [];
  const rows = parseMoomooTransactions(lines.join("\n"), "US", issues);
  assert.deepEqual(issues.map(issue => issue.line), [3, 5]);
  assert.equal(issues[0].raw, lines[2]);
  assert.match(issues[0].message, /数量/);
  assert.deepEqual(rows.map(row => [row.symbol, row.transaction_type, row.shares]), [["AAPL", "BUY", 1], ["MSFT", "SELL", 2]]);
});

test("Firstrade diagnoses invalid numeric/date/symbol fields and ignores non-trading activity", () => {
  const lines = [
    "Symbol,Action,Quantity,Price,TradeDate,Amount,Commission,Fee",
    "AAPL,BUY,1,100,2026-09-17,-100,0,0",
    "MSFT,SELL,2oops,200,2026-09-17,400,0,0",
    "MSFT,SELL,2,200,2026-13-17,400,0,0",
    ",BUY,2,200,2026-09-17,-400,0,0",
    ",DIVIDEND,0,0,2026-09-17,100,0,0",
    "Total,,3,,,300,,",
    "Report Footer",
  ];
  const issues = [];
  const rows = parseFirstradeTransactions(lines.join("\n"), issues);
  assert.equal(rows.length, 1);
  assert.deepEqual(issues.map(issue => issue.line), [3, 4, 5]);
  assert.equal(issues[0].raw, lines[2]);
  assert.match(issues[0].message, /数量/);
});

test("broker trade diagnostics accept and normalize legitimate share-class symbols containing a space", () => {
  const parsers = [
    issues => parseIbTransactions("Symbol,Date/Time,Quantity,Price\nBRK B,2026-09-17,1,100", "US", issues),
    issues => parseMoomooTransactions("方向,代码,成交数量,成交价格,成交时间\n买入,BRK B,1,100,2026/09/17 09:30:00", "US", issues),
    issues => parseFirstradeTransactions("Symbol,Action,Quantity,Price,TradeDate\nBRK B,BUY,1,100,2026-09-17", issues),
  ];
  for (const parse of parsers) {
    const issues = [];
    const rows = parse(issues);
    assert.equal(rows.length, 1);
    assert.equal(rows[0].symbol, "BRK-B");
    assert.deepEqual(issues, []);
  }
});

test("Firstrade reports all invalid trades instead of yielding an unexplained empty result", () => {
  const issues = [];
  const rows = parseFirstradeTransactions("Symbol,Action,Quantity,Price,TradeDate\nAAPL,BUY,0,100,2026-09-17", issues);
  assert.deepEqual(rows, []);
  assert.equal(issues.length, 1);
  assert.equal(issues[0].line, 2);
});

test("Firstrade surfaces a missing trade action while preserving valid ISO timestamps", () => {
  const issues = [];
  const rows = parseFirstradeTransactions("Symbol,Action,Quantity,Price,TradeDate\nAAPL,,1,100,2026-09-17\nMSFT,BUY,1,200,2026-09-17T09:30:00.123+08:00\nAAPL,BUY,1,100,2026-09-17T09:30:00Z", issues);
  assert.equal(rows.length, 2);
  assert.equal(rows[0].symbol, "MSFT");
  assert.equal(issues.length, 1);
  assert.equal(issues[0].line, 2);
  assert.match(issues[0].message, /方向/);
});

test("broker transactions preserve dash placeholders for unspecified optional amounts and fees", () => {
  const parsers = [
    issues => parseIbTransactions("Symbol,Date/Time,Quantity,Price,Proceeds,Comm,Fee\nAAPL,2026-09-17,1,100,-,-,-", "US", issues),
    issues => parseMoomooTransactions("方向,代码,成交数量,成交价格,成交时间,成交金额,合计费用\n买入,AAPL,1,100,2026/09/17 09:30:00,-,-", "US", issues),
    issues => parseFirstradeTransactions("Symbol,Action,Quantity,Price,TradeDate,Amount,Commission,Fee\nAAPL,BUY,1,100,2026-09-17,-,-,-", issues),
    issues => parseThsCsv("成交日期,证券代码,操作,成交数量,成交价格,成交金额,手续费\n20260917,600036,买入,1,100,-,-", issues),
  ];
  for (const parse of parsers) {
    const issues = [];
    const rows = parse(issues);
    assert.equal(rows.length, 1);
    assert.equal(rows[0].total_amount, 100);
    assert.equal(rows[0].commission, 0);
    assert.deepEqual(issues, []);
  }
});

test("THS reports malformed trade and dividend rows while excluding fees and totals", () => {
  const lines = [
    "成交日期,证券代码,证券名称,操作,成交数量,成交价格,成交金额,发生金额",
    "20260917,600036,招商银行,买入,100,30,3000,-3000",
    "20260917,600036,招商银行,卖出,bad,30,3000,3000",
    "20260230,600036,招商银行,买入,100,30,3000,-3000",
    "20260917,,招商银行,买入,100,30,3000,-3000",
    "20260917,600036,招商银行,红利,0,0,bad,bad",
    "20260917,600036,招商银行,上海存托服务费扣收,0,0,0,-1",
    ",合计,,,100,,3000,-3000",
  ];
  const issues = [];
  const rows = parseThsCsv(lines.join("\n"), issues);
  assert.equal(rows.length, 1);
  assert.deepEqual(issues.map(issue => issue.line), [3, 4, 5, 6]);
  assert.equal(issues[0].raw, lines[2]);
});
