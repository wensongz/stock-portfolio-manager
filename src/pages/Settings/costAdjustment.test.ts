// @ts-nocheck -- Runs directly in Node 26 without browser-focused Node typings.
import test from "node:test";
import assert from "node:assert/strict";
import { updateCostAdjustmentPolicy } from "./costAdjustment.ts";

test("cost adjustment is persisted by one atomic backend command", async () => {
  const calls = [];
  const current = {
    us_provider: "xueqiu",
    hk_provider: "xueqiu",
    cn_provider: "xueqiu",
    xueqiu_cookie: null,
    xueqiu_u: null,
    cn_adjust_sell_pay_cost: true,
    us_adjust_sell_pay_cost: false,
    hk_adjust_sell_pay_cost: false,
  };

  const updated = await updateCostAdjustmentPolicy(
    async (command, args) => calls.push({ command, args }),
    current,
    "us_adjust_sell_pay_cost",
    true
  );

  assert.equal(updated.us_adjust_sell_pay_cost, true);
  assert.deepEqual(calls, [
    {
      command: "update_quote_provider_config",
      args: { config: updated },
    },
  ]);
});
