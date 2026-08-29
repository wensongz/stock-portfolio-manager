// @ts-nocheck -- This test runs directly in Node 26; the app intentionally
// does not include @types/node in its browser-focused TypeScript config.
import test, { afterEach, beforeEach } from "node:test";
import assert from "node:assert/strict";

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((resolvePromise, rejectPromise) => {
    resolve = resolvePromise;
    reject = rejectPromise;
  });
  return { promise, resolve, reject };
}

const availability = (status = "available", note = null) => ({ status, note });

function windowMetric(days: number) {
  return {
    trading_days: days,
    status: availability(),
    matured_actions: 1,
    pending_actions: 0,
    amount_weighted_excess_return: 0.03,
    positive_notional_ratio: 1,
  };
}

function report(id: string) {
  return {
    methodology: {
      query: {
        start_date: "2026-01-01",
        end_date: "2026-08-28",
        account_id: null,
        market: null,
        benchmark_symbol: null,
        base_currency: "USD",
      },
      actual_return_method: "twr",
      shadow_return_method: "price_only",
      benchmark_return_method: "fixed_weight",
      fixed_weights: [{ key: "US", weight: 1 }],
      benchmark_symbol: null,
      market_data_coverage: {
        availability: availability(),
        covered_days: 100,
        expected_days: 100,
        coverage_ratio: 1,
      },
      exchange_rate_coverage: {
        availability: availability(),
        covered_days: 100,
        expected_days: 100,
        coverage_ratio: 1,
      },
      algorithm_version: id,
    },
    summary: {
      result_quality: {
        availability: availability(),
        portfolio_return: 0.1,
        shadow_return: 0.08,
        benchmark_return: 0.07,
        excess_return: 0.03,
        active_return: 0.03,
      },
      max_drawdown: {
        availability: availability(),
        max_drawdown: -0.05,
        peak_date: "2026-04-01",
        trough_date: "2026-04-20",
        duration_days: 19,
        recovery_date: "2026-05-01",
        recovery_duration_days: 11,
      },
      rebalance_value_add: {
        availability: availability(),
        value_add: 0.02,
        actual_return: 0.1,
        shadow_return: 0.08,
        ending_value_difference_base: 200,
      },
      forward_effect: {
        availability: availability(),
        day_60: windowMetric(60),
        day_120: windowMetric(120),
      },
      risk_structure: {
        availability: availability(),
        opening_max_stock_weight: 0.4,
        ending_max_stock_weight: 0.35,
        opening_cr5: 0.8,
        ending_cr5: 0.75,
        opening_cash_ratio: 0.1,
        ending_cash_ratio: 0.12,
        one_way_turnover: 0.2,
        fee_drag: 0.001,
      },
    },
    curves: [
      {
        date: "2026-01-01",
        portfolio_return: 0,
        shadow_return: 0,
        benchmark_return: 0,
      },
    ],
    attribution: {
      availability: availability(),
      total_value_add: 200,
      buy_value_add: 150,
      sell_value_add: 50,
      fees: -5,
      action_contributions: [],
      contributors: [],
      detractors: [],
      dividend_contribution: 0,
      fee_contribution: -5,
      currency_contribution: 0,
      cash_contribution: 0,
      explained_value_difference: 200,
      ending_value_difference: 200,
      residual: 0,
      residual_to_average_nav: 0,
      percentage_basis_label: "期间平均净资产",
    },
    risk_structure: {
      availability: availability(),
      concentration_availability: availability(),
      turnover_availability: availability(),
      fee_availability: availability(),
      opening: {
        date: "2026-01-01",
        max_stock_weight: 0.4,
        cr5: 0.8,
        hhi: 0.2,
        cash_ratio: 0.1,
      },
      ending: {
        date: "2026-08-28",
        max_stock_weight: 0.35,
        cr5: 0.75,
        hhi: 0.18,
        cash_ratio: 0.12,
      },
      peak: {
        date: "2026-02-01",
        max_stock_weight: 0.45,
        cr5: 0.82,
        hhi: 0.22,
        cash_ratio: 0.08,
      },
      one_way_turnover: 0.2,
      fee_drag: 0.001,
      data_hints: [],
      fact_labels: [],
      market_weights: [{ key: "US", weight: 1 }],
      category_weights: [],
      top_position_weights: [{ key: "AAPL", weight: 0.35 }],
      concentration: 0.18,
      diversification_score: 0.82,
    },
    actions: [],
    campaigns: [],
    data_quality: {
      availability: availability(),
      actual_result_availability: availability(),
      shadow_value_add_availability: availability(),
      attribution_availability: availability(),
      forward_effect_availability: availability(),
      issues: [],
      market_data_coverage: 1,
      exchange_rate_coverage: 1,
      interval_drawdown_only: false,
    },
    annotations: [],
    generated_at: `2026-08-28T00:00:00Z:${id}`,
  };
}

function campaignDetail(id: string) {
  const summary = {
    campaign_id: id,
    account_ids: ["account-a"],
    action_ids: ["action-1"],
    fragments: [
      {
        fragment_id: "fragment-1",
        logical_campaign_id: id,
        account_id: "account-a",
        symbol: "AAPL",
        market: "US",
        started_at: "2026-01-02",
        ended_at: null,
        status: "active",
        action_ids: ["action-1"],
        transfer_in: null,
        transfer_out: null,
      },
    ],
    campaign_status: "active",
    availability: availability(),
    symbol: "AAPL",
    market: "US",
    started_at: "2026-01-02",
    ended_at: null,
    contribution: 20,
  };
  return {
    availability: availability(),
    pnl_availability: availability(),
    excursion_availability: availability(),
    drawdown_availability: availability(),
    benchmark_availability: availability(),
    summary,
    actions: [],
    forward_effect_20d: windowMetric(20),
    forward_effect_60d: windowMetric(60),
    forward_effect_120d: windowMetric(120),
    pnl: {
      buy_outlays_base: 1000,
      sell_proceeds_base: 0,
      dividends_base: 10,
      trading_fees_base: 1,
      remaining_shares: 10,
      remaining_market_value_base: 1200,
      total_pnl_base: 209,
      max_invested_capital_base: 1001,
      label: "含剩余持仓市值的总盈亏",
    },
    campaign_return: 0.209,
    benchmark_return: 0.1,
    excess_return: 0.109,
    mae_base: -50,
    mfe_base: 250,
    mae_percent: -0.05,
    mfe_percent: 0.25,
    holding_period_drawdown: -0.08,
    timeline: [
      {
        date: "2026-01-02",
        kind: "buy",
        amount_base: 1000,
        amount_local: 1000,
        currency: "USD",
        shares: 10,
        account_id: "account-a",
        action_id: "action-1",
      },
    ],
    fact_labels: [],
    completed_sample_count: 0,
    active_sample_count: 1,
    annotations: [],
    issues: [],
  };
}

const filters = (accountId: string | null = null) => ({
  accountId,
  periodPreset: "YTD",
  startDate: "2026-01-01",
  endDate: "2026-08-28",
  market: null,
  benchmarkSymbol: null,
  baseCurrency: "USD",
});

let invokeImpl = () => Promise.reject(new Error("invoke not configured"));
globalThis.window = {
  __TAURI_INTERNALS__: {
    invoke(command: string, args: unknown) {
      return invokeImpl(command, args);
    },
  },
};

const { useStockReviewStore } = await import("./stockReviewStore.ts");

beforeEach(() => {
  invokeImpl = () => Promise.reject(new Error("invoke not configured"));
  useStockReviewStore.setState({
    filters: null,
    reportLoading: false,
    campaignLoading: false,
    mutating: false,
    report: null,
    selectedCampaign: null,
    error: null,
    errorSource: null,
  });
});

afterEach(() => {
  invokeImpl = () => Promise.reject(new Error("invoke not configured"));
});

test("loadReport invokes the exact command with camelCase Tauri arguments", async () => {
  const expected = report("first");
  invokeImpl = async (command, args) => {
    assert.equal(command, "get_stock_review_report");
    assert.deepEqual(args, {
      startDate: "2026-01-01",
      endDate: "2026-08-28",
      accountId: "account-a",
      market: null,
      benchmarkSymbol: null,
      baseCurrency: "USD",
    });
    return expected;
  };

  await useStockReviewStore.getState().loadReport(filters("account-a"));
  assert.equal(useStockReviewStore.getState().report, expected);
  assert.equal(useStockReviewStore.getState().reportLoading, false);
  assert.equal(useStockReviewStore.getState().error, null);
});

test("stale report success and error cannot overwrite the latest filter request", async () => {
  const first = deferred();
  const second = deferred();
  invokeImpl = (_command, args) =>
    args.accountId === "account-a" ? first.promise : second.promise;

  const firstLoad = useStockReviewStore.getState().loadReport(filters("account-a"));
  const secondLoad = useStockReviewStore.getState().loadReport(filters("account-b"));
  const latest = report("latest");
  second.resolve(latest);
  await secondLoad;
  first.reject(new Error("stale failure"));
  await firstLoad;

  assert.equal(useStockReviewStore.getState().report, latest);
  assert.deepEqual(useStockReviewStore.getState().filters, filters("account-b"));
  assert.equal(useStockReviewStore.getState().error, null);
  assert.equal(useStockReviewStore.getState().reportLoading, false);
});

test("Campaign requests are independently race-safe and never clear the report", async () => {
  const existingReport = report("portfolio");
  const first = deferred();
  const second = deferred();
  useStockReviewStore.setState({ report: existingReport });
  invokeImpl = (command, args) => {
    assert.equal(command, "get_stock_campaign_detail");
    assert.deepEqual(
      Object.keys(args).sort(),
      [
        "accountId",
        "baseCurrency",
        "benchmarkSymbol",
        "campaignId",
        "endDate",
        "market",
        "startDate",
      ].sort(),
    );
    return args.campaignId === "campaign-old" ? first.promise : second.promise;
  };

  const oldLoad = useStockReviewStore
    .getState()
    .loadCampaignDetail(filters(), "campaign-old");
  const newLoad = useStockReviewStore
    .getState()
    .loadCampaignDetail(filters(), "campaign-new");
  const latest = campaignDetail("campaign-new");
  second.resolve(latest);
  await newLoad;
  first.resolve(campaignDetail("campaign-old"));
  await oldLoad;

  assert.equal(useStockReviewStore.getState().report, existingReport);
  assert.equal(useStockReviewStore.getState().selectedCampaign, latest);
  assert.equal(useStockReviewStore.getState().campaignLoading, false);
});

test("latest load errors keep the last successful report and Campaign detail", async () => {
  const existingReport = report("retained");
  const existingCampaign = campaignDetail("retained-campaign");
  useStockReviewStore.setState({
    report: existingReport,
    selectedCampaign: existingCampaign,
  });
  invokeImpl = async (command) => {
    throw new Error(command === "get_stock_review_report" ? "report unavailable" : "detail unavailable");
  };

  await useStockReviewStore.getState().loadReport(filters());
  assert.equal(useStockReviewStore.getState().report, existingReport);
  assert.match(useStockReviewStore.getState().error, /report unavailable/);

  await useStockReviewStore.getState().loadCampaignDetail(filters(), "campaign-new");
  assert.equal(useStockReviewStore.getState().report, existingReport);
  assert.equal(useStockReviewStore.getState().selectedCampaign, existingCampaign);
  assert.match(useStockReviewStore.getState().error, /detail unavailable/);
});

test("saveAnnotation applies the authoritative annotation to matching current scopes only", async () => {
  const existingReport = report("annotation");
  const existingCampaign = campaignDetail("campaign-7");
  const returned = {
    id: "annotation-1",
    scope_type: "campaign",
    scope_key: "campaign-7",
    account_id: "account-a",
    symbol: "AAPL",
    annotation_type: "thesis",
    value_json: '{"text":"authoritative"}',
    source: "user",
    created_at: "2026-08-29T01:02:03Z",
    updated_at: "2026-08-29T01:02:03Z",
  };
  useStockReviewStore.setState({
    report: existingReport,
    selectedCampaign: existingCampaign,
  });
  invokeImpl = async (command, args) => {
    assert.equal(command, "save_stock_review_annotation");
    assert.equal(args.input.source, "caller-value-overwritten-by-server");
    return returned;
  };

  const saved = await useStockReviewStore.getState().saveAnnotation({
    id: "annotation-1",
    scope_type: "campaign",
    scope_key: "campaign-7",
    account_id: "account-a",
    symbol: "AAPL",
    annotation_type: "thesis",
    value_json: '{"text":"draft"}',
    source: "caller-value-overwritten-by-server",
  });

  assert.equal(saved, returned);
  assert.deepEqual(useStockReviewStore.getState().report.annotations, [returned]);
  assert.deepEqual(useStockReviewStore.getState().selectedCampaign.annotations, [returned]);
  assert.equal(useStockReviewStore.getState().report.summary, existingReport.summary);
  assert.equal(useStockReviewStore.getState().error, null);
});

test("confirmOverride adopts the returned report directly without a reload", async () => {
  const calls = [];
  const returned = report("override-result");
  useStockReviewStore.setState({
    report: report("before-override"),
    selectedCampaign: campaignDetail("stale-detail"),
  });
  invokeImpl = async (command, args) => {
    calls.push({ command, args });
    return returned;
  };

  const result = await useStockReviewStore.getState().confirmOverride(filters(), {
    id: "override-1",
    override_type: "non_trade",
    transaction_ids_json: '["tx-1"]',
    value_json: "{}",
  });

  assert.equal(result, returned);
  assert.deepEqual(calls.map((call) => call.command), ["confirm_stock_review_override"]);
  assert.equal(useStockReviewStore.getState().report, returned);
  assert.equal(useStockReviewStore.getState().selectedCampaign, null);
});

test("concurrent mutations keep loading until all settle and stale results cannot win", async () => {
  const older = deferred();
  const newer = deferred();
  const initial = report("initial");
  const latest = report("latest-mutation");
  useStockReviewStore.setState({ report: initial });
  invokeImpl = (command) =>
    command === "save_stock_review_annotation" ? older.promise : newer.promise;

  const annotationSave = useStockReviewStore.getState().saveAnnotation({
    id: "annotation-old",
    scope_type: "period",
    scope_key: "2026-01-01:2026-08-28",
    account_id: null,
    symbol: null,
    annotation_type: "note",
    value_json: "{}",
    source: "user",
  });
  const overrideSave = useStockReviewStore.getState().confirmOverride(filters(), {
    id: "override-new",
    override_type: "non_trade",
    transaction_ids_json: '["tx-1"]',
    value_json: "{}",
  });

  newer.resolve(latest);
  await overrideSave;
  assert.equal(useStockReviewStore.getState().report, latest);
  assert.equal(useStockReviewStore.getState().mutating, true);

  older.reject(new Error("stale annotation failure"));
  await annotationSave;
  assert.equal(useStockReviewStore.getState().report, latest);
  assert.equal(useStockReviewStore.getState().mutating, false);
  assert.equal(useStockReviewStore.getState().error, null);
});

test("a report started before override confirmation cannot replace the confirmed report", async () => {
  const loadingReport = deferred();
  const confirming = deferred();
  invokeImpl = (command) =>
    command === "get_stock_review_report" ? loadingReport.promise : confirming.promise;

  const reportLoad = useStockReviewStore.getState().loadReport(filters());
  const overrideSave = useStockReviewStore.getState().confirmOverride(filters(), {
    id: "override-latest",
    override_type: "non_trade",
    transaction_ids_json: '["tx-1"]',
    value_json: "{}",
  });
  const confirmed = report("confirmed");
  confirming.resolve(confirmed);
  await overrideSave;
  loadingReport.resolve(report("obsolete-load"));
  await reportLoad;

  assert.equal(useStockReviewStore.getState().report, confirmed);
  assert.equal(useStockReviewStore.getState().reportLoading, false);
});

test("a report started after override confirmation begins remains the latest filter authority", async () => {
  const confirming = deferred();
  const loadingReport = deferred();
  invokeImpl = (command) =>
    command === "confirm_stock_review_override" ? confirming.promise : loadingReport.promise;

  const overrideSave = useStockReviewStore.getState().confirmOverride(filters("account-a"), {
    id: "override-old-scope",
    override_type: "non_trade",
    transaction_ids_json: '["tx-1"]',
    value_json: "{}",
  });
  const reportLoad = useStockReviewStore.getState().loadReport(filters("account-b"));
  const latest = report("new-filter");
  loadingReport.resolve(latest);
  await reportLoad;
  confirming.resolve(report("old-filter-override"));
  await overrideSave;

  assert.equal(useStockReviewStore.getState().report, latest);
  assert.deepEqual(useStockReviewStore.getState().filters, filters("account-b"));
});

test("an annotation response from an old report scope is not attached to a newer report", async () => {
  const saving = deferred();
  const loadingReport = deferred();
  invokeImpl = (command) =>
    command === "save_stock_review_annotation" ? saving.promise : loadingReport.promise;
  useStockReviewStore.setState({ report: report("old-filter") });

  const annotationSave = useStockReviewStore.getState().saveAnnotation({
    id: "old-note",
    scope_type: "period",
    scope_key: "2026-01-01:2026-08-28",
    account_id: null,
    symbol: null,
    annotation_type: "note",
    value_json: "{}",
    source: "user",
  });
  const reportLoad = useStockReviewStore.getState().loadReport(filters("account-b"));
  const latest = report("new-filter");
  loadingReport.resolve(latest);
  await reportLoad;
  saving.resolve({
    id: "old-note",
    scope_type: "period",
    scope_key: "2026-01-01:2026-08-28",
    account_id: null,
    symbol: null,
    annotation_type: "note",
    value_json: "{}",
    source: "user",
    created_at: "2026-08-29T01:02:03Z",
    updated_at: "2026-08-29T01:02:03Z",
  });
  await annotationSave;

  assert.equal(useStockReviewStore.getState().report, latest);
  assert.deepEqual(useStockReviewStore.getState().report.annotations, []);
});
