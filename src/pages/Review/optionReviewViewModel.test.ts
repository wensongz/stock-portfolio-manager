// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test from "node:test";
import assert from "node:assert/strict";
import {
  OPTION_REVIEW_ANNUALIZED_YIELD_LABEL,
  buildOptionReviewPrompt,
  formatReviewPercent,
  getOptionReviewEmptyDescription,
  loadOptionReviewPeriodDays,
  saveOptionReviewPeriodDays,
  selectDefaultUnderlying,
  shouldShowNetPremium,
  sortOptionCampaigns,
  sortUnderlyingReviews,
} from "./optionReviewViewModel.ts";

function memoryStorage(initial: Record<string, string> = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem(key: string) {
      return values.get(key) ?? null;
    },
    setItem(key: string, value: string) {
      values.set(key, value);
    },
  };
}

test("option review period remembers the latest valid selection", () => {
  const storage = memoryStorage();

  saveOptionReviewPeriodDays(storage, null);
  assert.equal(loadOptionReviewPeriodDays(storage), null);

  saveOptionReviewPeriodDays(storage, 365);
  assert.equal(loadOptionReviewPeriodDays(storage), 365);

  saveOptionReviewPeriodDays(storage, 730);
  assert.equal(loadOptionReviewPeriodDays(storage), 730);
});

test("option review period falls back to 365 days for missing or invalid data", () => {
  for (const value of [undefined, "", "0", "30", "lots"]) {
    const storage = memoryStorage(
      value === undefined ? {} : { review_option_period_days: value },
    );
    assert.equal(loadOptionReviewPeriodDays(storage), 365);
  }
});

test("recent option review prompt carries executable tool arguments", () => {
  const prompt = buildOptionReviewPrompt({
    accountId: "acct-options-123",
    accountName: "Options Account",
    symbol: "AAPL",
    periodDays: 365,
  });

  assert.match(prompt, /Options Account/);
  assert.match(prompt, /"accountId":"acct-options-123"/);
  assert.match(prompt, /"symbol":"AAPL"/);
  assert.match(prompt, /"periodDays":365/);
  assert.match(prompt, /确定性期权复盘数据/);
  assert.match(prompt, /样本限制/);
});

test("all-history option review prompt requests explicit allHistory semantics", () => {
  const prompt = buildOptionReviewPrompt({
    accountId: "acct-options-123",
    accountName: "Options Account",
    symbol: "MSFT",
    periodDays: null,
  });

  assert.match(prompt, /"accountId":"acct-options-123"/);
  assert.match(prompt, /"symbol":"MSFT"/);
  assert.match(prompt, /"allHistory":true/);
  assert.doesNotMatch(prompt, /"periodDays"/);
  assert.match(prompt, /确定性期权复盘数据/);
  assert.match(prompt, /样本限制/);
});

test("annualized yield label states the secured-notional basis", () => {
  assert.equal(
    OPTION_REVIEW_ANNUALIZED_YIELD_LABEL,
    "年化收益率（担保名义资本口径）",
  );
});

const underlying = (symbol: string, pnl: number, flags: string[] = []) => ({
  underlying: symbol,
  completed_campaigns: 1,
  active_campaigns: 0,
  gross_premium: 100,
  net_premium_pnl: pnl,
  retention_rate: 0.5,
  annualized_yield_on_notional: 0.05,
  worst_campaign_pnl: pnl,
  flags,
  campaigns: [],
});

test("underlyings sort by absolute net premium, then symbol", () => {
  const sorted = sortUnderlyingReviews([
    underlying("MSFT", 500, ["高留存"]),
    underlying("AAPL", -200, ["净亏损"]),
    underlying("NVDA", 900, ["低留存"]),
  ] as never);
  assert.deepEqual(sorted.map((row) => row.underlying), ["NVDA", "MSFT", "AAPL"]);
});

test("default selection uses the largest absolute net premium", () => {
  const report = { underlyings: [underlying("MSFT", 500), underlying("AAPL", -200, ["净亏损"])] } as never;
  assert.equal(selectDefaultUnderlying(report), "MSFT");
});

test("campaigns sort by expiry descending, then id", () => {
  const campaign = (id: string, expiryDate: string, startedAt: string) => ({
    id,
    expiry_date: expiryDate,
    started_at: startedAt,
  });

  const sorted = sortOptionCampaigns([
    campaign("later-id", "2026-09-30", "2026-08-01"),
    campaign("earlier-expiry", "2026-08-28", "2026-08-20"),
    campaign("earlier-id", "2026-09-30", "2026-08-15"),
  ] as never);

  assert.deepEqual(sorted.map((row) => row.id), ["earlier-id", "later-id", "earlier-expiry"]);
});

test("percentage formatter preserves negative values and missing state", () => {
  assert.equal(formatReviewPercent(-0.31), "-31.0%");
  assert.equal(formatReviewPercent(null), "—");
});

test("cumulative net premium remains visible for active-only reports", () => {
  assert.equal(
    shouldShowNetPremium({ completed_campaigns: 0, active_campaigns: 2 }),
    true,
  );
  assert.equal(
    shouldShowNetPremium({ completed_campaigns: 0, active_campaigns: 0 }),
    false,
  );
});

test("empty review with excluded records directs to the quality notice", () => {
  for (const quality of [
    { unmatched_records: 2, missing_trade_dates: 0 },
    { unmatched_records: 0, missing_trade_dates: 3 },
  ]) {
    assert.equal(
      getOptionReviewEmptyDescription(quality),
      "当前暂无可分析的Campaign，请查看上方数据质量说明",
    );
  }
});

test("empty review without excluded records directs to CSV import", () => {
  assert.equal(
    getOptionReviewEmptyDescription({ unmatched_records: 0, missing_trade_dates: 0 }),
    "该账户暂无可复盘的期权记录，请去期权管理导入CSV",
  );
});
