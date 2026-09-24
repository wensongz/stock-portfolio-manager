// @ts-nocheck -- Run directly with Node's TypeScript support.
import test from "node:test";
import assert from "node:assert/strict";
import { normalizeHoldingSymbol } from "./holdingSymbol.ts";

test("all supported stock and B-share prefixes acquire the correct exchange", () => {
  for (const [input, expected] of [
    ["601069", "sh601069"],
    ["600519", "sh600519"],
    ["603288", "sh603288"],
    ["605499", "sh605499"],
    ["688001", "sh688001"],
    ["900901", "sh900901"],
    ["000001", "sz000001"],
    ["001979", "sz001979"],
    ["002594", "sz002594"],
    ["003816", "sz003816"],
    ["004001", "sz004001"],
    ["300750", "sz300750"],
    ["301001", "sz301001"],
    ["200002", "sz200002"],
    [" 601069 ", "sh601069"],
  ]) {
    assert.equal(normalizeHoldingSymbol(input, "CN"), expected);
  }
});

test("six-digit fund codes use the requested 5, 15 and 16 exchange rules", () => {
  for (const [input, expected] of [
    ["510300", "sh510300"],
    ["588000", "sh588000"],
    ["500001", "sh500001"],
    ["599999", "sh599999"],
    ["150001", "sz150001"],
    ["159915", "sz159915"],
    ["161725", "sz161725"],
    [" 161725 ", "sz161725"],
  ]) {
    assert.equal(normalizeHoldingSymbol(input, "CN"), expected);
  }
});

test("other markets and explicit exchange prefixes are preserved", () => {
  for (const market of ["US", "HK", undefined]) {
    for (const symbol of ["601069", "900901", "200002", "510300", "159915", "161725"]) {
      assert.equal(normalizeHoldingSymbol(symbol, market), symbol);
    }
  }
  for (const symbol of ["sh601069", "SH601069", "sz000001", "Sz300750", "sh900901", "sz200002", "SH510300", "SZ159915", "sz161725", "$CASH-CNY"]) {
    assert.equal(normalizeHoldingSymbol(symbol, "CN"), symbol);
  }
});

test("incomplete, unsupported and nonnumeric codes are never assigned a guessed exchange", () => {
  for (const symbol of ["", "60106", "6010690", "60A069", "601 069", "920001", "830799", "430047", "602001", "604001", "689001", "699999", "005001", "009001", "302001", "309001", "201001", "901001", "123001", "170001", "51030", "1599150", "16A725", "00700", "AAPL"]) {
    assert.equal(normalizeHoldingSymbol(symbol, "CN"), symbol);
  }
});
