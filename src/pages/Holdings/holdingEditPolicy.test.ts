// @ts-nocheck -- Run directly with Node's TypeScript support.
import test from "node:test";
import assert from "node:assert/strict";
import { canEditOpening } from "./holdingEditPolicy.ts";

const holding = { id: "h", account_id: "a", symbol: "AAPL", currency: "USD" };
const opening = { id: "o", holding_id: "h", account_id: "a", symbol: "AAPL", currency: "USD", transaction_type: "OPEN" };

test("only a single opening or no history permits a direct balance correction", () => {
  assert.equal(canEditOpening(holding, []), true);
  assert.equal(canEditOpening(holding, [opening]), true);
  assert.equal(canEditOpening(holding, [opening, { ...opening, id: "o2" }]), false);
});

test("an unlinked trade still locks position amounts and identity", () => {
  assert.equal(canEditOpening(holding, [{ ...opening, holding_id: null, symbol: "aapl", transaction_type: "BUY" }]), false);
  assert.equal(canEditOpening(holding, [{ ...opening, holding_id: null, symbol: "MSFT", transaction_type: "BUY" }]), true);
});

test("stock trades lock the corresponding cash balance but not other currencies", () => {
  const cash = { ...holding, id: "cash", symbol: "$CASH-USD" };
  assert.equal(canEditOpening(cash, [opening]), true);
  assert.equal(canEditOpening(cash, [{ ...opening, transaction_type: "BUY" }]), false);
  assert.equal(canEditOpening(cash, [{ ...opening, transaction_type: "PAY" }]), false);
  assert.equal(canEditOpening(cash, [{ ...opening, transaction_type: "BUY", currency: "HKD" }]), true);
});
