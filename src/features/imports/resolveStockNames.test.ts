// @ts-nocheck -- Runs directly in Node 26 without browser-focused Node typings.
import test from "node:test";
import assert from "node:assert/strict";
import { resolveStockNames } from "./resolveStockNames.ts";

test("resolves unique symbols from holdings, then remote lookup, then symbol fallback", async () => {
  const calls: { command: string; args: unknown }[] = [];
  const fakeInvoke = async (command: string, args: unknown) => {
    calls.push({ command, args });
    if (command === "get_holdings") return [{ symbol: "AAPL", name: "Apple" }];
    if (args.symbol === "MSFT") return "Microsoft";
    throw new Error("lookup unavailable");
  };

  const names = await resolveStockNames(["aapl", "MSFT", "MSFT", "NOPE"], fakeInvoke);

  assert.deepEqual([...names], [
    ["AAPL", "Apple"],
    ["MSFT", "Microsoft"],
    ["NOPE", "NOPE"],
  ]);
  assert.deepEqual(calls, [
    { command: "get_holdings", args: { accountId: null } },
    { command: "lookup_stock_name_by_symbol", args: { symbol: "MSFT" } },
    { command: "lookup_stock_name_by_symbol", args: { symbol: "NOPE" } },
  ]);
});
