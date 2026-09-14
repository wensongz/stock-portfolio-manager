// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import type { HoldingDetail } from "../../types/index.ts";
import { aggregateDashboardHoldings, filterDashboardHoldings } from "./aggregateDashboardHoldings.ts";

function holding(overrides: Partial<HoldingDetail> = {}): HoldingDetail {
  return {
    id: "holding-a", account_id: "account-a", account_name: "账户 A",
    symbol: "AAPL", name: "苹果", market: "US", category_name: "科技",
    category_color: "#1677ff", shares: 10, avg_cost: 100, current_price: 120,
    market_value: 1200, market_value_usd: 1200, cost_value: 1000,
    pnl: 200, pnl_percent: 20, daily_pnl: 40, daily_change_percent: 3.45,
    currency: "USD", ...overrides,
  };
}

test("合并同一股票跨账户持仓，以总成本计算均价和盈亏率，保留当日行情涨跌幅", () => {
  const rows = aggregateDashboardHoldings([
    holding(),
    holding({ id: "holding-b", account_id: "account-b", account_name: "账户 B",
      shares: 30, avg_cost: 80, market_value: 3600, market_value_usd: 3600,
      cost_value: 2400, pnl: 1200, pnl_percent: 50, daily_pnl: 120 }),
  ]);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].shares, 40);
  assert.equal(rows[0].avg_cost, 85);
  assert.equal(rows[0].cost_value, 3400);
  assert.equal(rows[0].market_value, 4800);
  assert.equal(rows[0].pnl, 1400);
  assert.ok(Math.abs(rows[0].pnl_percent! - 41.1764705882) < 1e-8);
  assert.equal(rows[0].daily_change_percent, 3.45);
  assert.equal(rows[0].position_pct, 100);
  assert.deepEqual(rows[0].accountRows.map((row) => [row.account_id, row.shares, row.position_pct]), [
    ["account-b", 30, 75], ["account-a", 10, 25],
  ]);
});

test("账户子表按账户 ID 合并重复记录，同名账户仍可区分", () => {
  const rows = aggregateDashboardHoldings([
    holding(), holding({ id: "lot-2", avg_cost: 80, cost_value: 800, pnl: 400 }),
    holding({ id: "holding-b", account_id: "account-b" }),
  ]);
  assert.equal(rows[0].accountRows.length, 2);
  const account = rows[0].accountRows[0];
  assert.equal(account.account_id, "account-a");
  assert.equal(account.shares, 20);
  assert.equal(account.avg_cost, 90);
  assert.equal(account.pnl, 600);
  assert.ok(Math.abs(account.pnl_percent! - 33.3333333333) < 1e-8);
});

test("仓位和市值排序使用美元口径，分账户仓位沿用组合总额", () => {
  const rows = aggregateDashboardHoldings([
    holding(),
    holding({ id: "hk", symbol: "0700.HK", market: "HK", currency: "HKD",
      market_value: 7800, market_value_usd: 1000 }),
  ]);
  assert.deepEqual(rows.map((row) => row.symbol), ["AAPL", "0700.HK"]);
  assert.ok(Math.abs(rows[0].position_pct - 54.5454545455) < 1e-8);
  assert.equal(rows[0].accountRows[0].position_pct, rows[0].position_pct);
});

test("不同市场或币种的同代码不相加，同市场代码忽略大小写和首尾空格", () => {
  const rows = aggregateDashboardHoldings([
    holding(), holding({ id: "case", symbol: " aapl ", market: "us" }),
    holding({ id: "other-market", market: "HK", currency: "HKD" }),
    holding({ id: "other-currency", currency: "CNY" }),
  ]);
  assert.equal(rows.length, 3);
  assert.equal(rows.find((row) => row.market === "US" && row.currency === "USD")?.shares, 20);
  assert.equal(new Set(rows.map((row) => row.key)).size, 3);
});

test("同币种现金跨市场合并且不显示行情涨跌幅，零持仓不进入概览", () => {
  const cash = holding({ symbol: "$CASH-USD", name: "美元现金", shares: 100.25,
    current_price: 1, avg_cost: 1, market_value: 100.25, market_value_usd: 100.25,
    cost_value: 100.25, pnl: 0, pnl_percent: 0, daily_pnl: 0 });
  const rows = aggregateDashboardHoldings([
    cash, { ...cash, id: "cash-hk", market: "HK", account_id: "account-b" },
    holding({ shares: 0, market_value: 0, market_value_usd: 0 }),
  ]);
  assert.equal(rows.length, 1);
  assert.equal(rows[0].shares, 200.5);
  assert.equal(rows[0].daily_change_percent, null);
  assert.equal(rows[0].accountRows.length, 2);
});

test("缺失或无效涨跌幅显示未知，零涨跌保持零，零成本不产生无效盈亏率", () => {
  for (const change of [undefined, null, NaN, Infinity]) {
    const row = aggregateDashboardHoldings([holding({ daily_change_percent: change })])[0];
    assert.equal(row.daily_change_percent, null);
  }
  const row = aggregateDashboardHoldings([holding({ daily_change_percent: 0, cost_value: 0, avg_cost: 0 })])[0];
  assert.equal(row.daily_change_percent, 0);
  assert.equal(row.pnl_percent, null);
  assert.equal(row.accountRows[0].pnl_percent, null);
  assert.equal(aggregateDashboardHoldings([holding({ market_value_usd: 0 })])[0].position_pct, 0);
  assert.deepEqual(aggregateDashboardHoldings([]), []);
});

test("先按账户和市场筛选再聚合，仓位分母仅包含所选持仓", () => {
  const holdings = [holding(), holding({ id: "b", account_id: "account-b" }),
    holding({ id: "hk", account_id: "account-a", market: "HK", symbol: "0700.HK" })];
  const visible = filterDashboardHoldings(holdings, ["account-a"], ["US"]);
  assert.deepEqual(visible.map((row) => row.id), ["holding-a"]);
  const rows = aggregateDashboardHoldings(visible);
  assert.equal(rows[0].shares, 10);
  assert.equal(rows[0].position_pct, 100);
  assert.equal(rows[0].accountRows.length, 1);
  assert.equal(filterDashboardHoldings(holdings, [], []).length, 3);
});
