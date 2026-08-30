// @ts-nocheck -- Runs directly in Node 26 without browser-focused Node typings.
import test from "node:test";
import assert from "node:assert/strict";

const pending: Array<{
  args: Record<string, unknown>;
  resolve: (value: unknown) => void;
  reject: (error: unknown) => void;
}> = [];

globalThis.window = {
  __TAURI_INTERNALS__: {
    invoke(command: string, args: Record<string, unknown>) {
      assert.equal(command, "get_stock_operation_review");
      assert.equal(args.benchmarkSymbol, undefined);
      assert.equal(args.campaignId, undefined);
      return new Promise((resolve, reject) => pending.push({ args, resolve, reject }));
    },
  },
};

const filters = (accountId: string) => ({
  accountId,
  periodPreset: "CUSTOM",
  startDate: "2026-07-01",
  endDate: "2026-08-30",
  market: "US",
  baseCurrency: "USD",
});

const report = (accountId: string) => ({
  query: {
    start_date: "2026-07-01",
    end_date: "2026-08-30",
    account_id: accountId,
    market: "US",
    base_currency: "USD",
  },
  summary: {},
  securities: [],
  actions: [],
  data_quality: { action_count: 0 },
  generated_at: "2026-08-30T00:00:00Z",
  algorithm_version: "stock-operation-review-lite-v1",
});

test("latest lightweight review request wins and refresh failure keeps last success", async () => {
  const { useStockOperationReviewStore } = await import("./stockOperationReviewStore.ts");
  useStockOperationReviewStore.setState({ report: null, loading: false, error: null });

  const first = useStockOperationReviewStore.getState().loadReport(filters("account-a"));
  const second = useStockOperationReviewStore.getState().loadReport(filters("account-b"));
  assert.deepEqual(pending[0].args, {
    startDate: "2026-07-01",
    endDate: "2026-08-30",
    accountId: "account-a",
    market: "US",
    baseCurrency: "USD",
  });
  pending[1].resolve(report("account-b"));
  await second;
  pending[0].resolve(report("account-a"));
  await first;
  assert.equal(useStockOperationReviewStore.getState().report.query.account_id, "account-b");
  assert.equal(useStockOperationReviewStore.getState().loading, false);

  const refresh = useStockOperationReviewStore.getState().loadReport(filters("account-b"));
  pending[2].reject(new Error("offline"));
  await refresh;
  assert.equal(useStockOperationReviewStore.getState().report.query.account_id, "account-b");
  assert.match(useStockOperationReviewStore.getState().error, /offline/);
});

test("first load failure exposes an error without fabricating a report", async () => {
  const { useStockOperationReviewStore } = await import("./stockOperationReviewStore.ts");
  useStockOperationReviewStore.setState({ report: null, loading: false, error: null });
  const request = useStockOperationReviewStore.getState().loadReport(filters("account-c"));
  pending[3].reject(new Error("first failure"));
  await request;
  assert.equal(useStockOperationReviewStore.getState().report, null);
  assert.equal(useStockOperationReviewStore.getState().loading, false);
  assert.match(useStockOperationReviewStore.getState().error, /first failure/);
});
