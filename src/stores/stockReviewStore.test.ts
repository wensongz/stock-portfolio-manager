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

function reportWithReviewScopes(id: string, accountId = "account-a") {
  const value = report(id);
  value.methodology.query.account_id = accountId;
  const oldCampaign = campaignDetail("campaign-old").summary;
  oldCampaign.started_at = "2025-01-01T09:30:00Z";
  oldCampaign.ended_at = "2025-06-30T16:00:00Z";
  oldCampaign.campaign_status = "completed";
  oldCampaign.action_ids = ["action-old"];
  const currentCampaign = campaignDetail("campaign-current").summary;
  currentCampaign.action_ids = ["action-a"];
  value.campaigns = [oldCampaign, currentCampaign];
  value.actions = [
    {
      action_id: "action-a",
      transaction_ids: ["tx-a"],
      account_id: accountId,
      symbol: "AAPL",
      market: "US",
      action_type: "open",
      traded_at: "2026-01-02T09:30:00Z",
      weighted_average_price: 100,
      gross_amount: 1000,
      currency: "USD",
      shares_before: 0,
      shares_after: 10,
      portfolio_weight_before: 0,
      portfolio_weight_after: 0.1,
      fees: 1,
      contribution: 20,
      observation_windows: [windowMetric(60), windowMetric(120)],
      status: "available",
      fact_labels: [],
    },
  ];
  return value;
}

function savedAnnotation(overrides = {}) {
  return {
    id: "annotation-1",
    scope_type: "period",
    scope_key: "2026-01-01:2026-08-28",
    account_id: null,
    symbol: null,
    annotation_type: "note",
    value_json: "{}",
    source: "user",
    created_at: "2026-08-29T01:02:03Z",
    updated_at: "2026-08-29T01:02:03Z",
    ...overrides,
  };
}

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

test("latest load errors keep the last successful report while a new report generation clears the drawer", async () => {
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
  assert.equal(useStockReviewStore.getState().selectedCampaign, null);
  assert.match(useStockReviewStore.getState().error, /report unavailable/);

  await useStockReviewStore.getState().loadCampaignDetail(filters(), "campaign-new");
  assert.equal(useStockReviewStore.getState().report, existingReport);
  assert.equal(useStockReviewStore.getState().selectedCampaign, null);
  assert.match(useStockReviewStore.getState().error, /detail unavailable/);
});

test("saveAnnotation applies the authoritative annotation to matching current scopes only", async () => {
  const existingReport = report("annotation");
  const existingCampaign = campaignDetail("campaign-7");
  existingReport.campaigns = [existingCampaign.summary];
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

test("a new report generation invalidates a pending Campaign success", async () => {
  const detailRequest = deferred();
  const reportRequest = deferred();
  const accountAReport = reportWithReviewScopes("account-a-report", "account-a");
  useStockReviewStore.setState({
    filters: filters("account-a"),
    report: accountAReport,
    selectedCampaign: campaignDetail("campaign-old"),
  });
  invokeImpl = (command) =>
    command === "get_stock_campaign_detail" ? detailRequest.promise : reportRequest.promise;

  const detailLoad = useStockReviewStore
    .getState()
    .loadCampaignDetail(filters("account-a"), "campaign-current");
  const reportLoad = useStockReviewStore.getState().loadReport(filters("account-b"));
  assert.equal(useStockReviewStore.getState().selectedCampaign, null);
  assert.equal(useStockReviewStore.getState().campaignLoading, false);

  const accountBReport = reportWithReviewScopes("account-b-report", "account-b");
  reportRequest.resolve(accountBReport);
  await reportLoad;
  detailRequest.resolve(campaignDetail("campaign-current"));
  await detailLoad;

  assert.equal(useStockReviewStore.getState().report, accountBReport);
  assert.equal(useStockReviewStore.getState().selectedCampaign, null);
  assert.equal(useStockReviewStore.getState().campaignLoading, false);
  assert.equal(useStockReviewStore.getState().error, null);
});

test("a new report generation ignores a pending Campaign error", async () => {
  const detailRequest = deferred();
  const accountBReport = reportWithReviewScopes("account-b-report", "account-b");
  useStockReviewStore.setState({
    filters: filters("account-a"),
    report: reportWithReviewScopes("account-a-report", "account-a"),
  });
  invokeImpl = (command) =>
    command === "get_stock_campaign_detail"
      ? detailRequest.promise
      : Promise.resolve(accountBReport);

  const detailLoad = useStockReviewStore
    .getState()
    .loadCampaignDetail(filters("account-a"), "campaign-current");
  await useStockReviewStore.getState().loadReport(filters("account-b"));
  detailRequest.reject(new Error("stale Campaign error"));
  await detailLoad;

  assert.equal(useStockReviewStore.getState().report, accountBReport);
  assert.equal(useStockReviewStore.getState().selectedCampaign, null);
  assert.equal(useStockReviewStore.getState().campaignLoading, false);
  assert.equal(useStockReviewStore.getState().error, null);
});

test("saveAnnotation appends only rows visible to the current report and drawer", async () => {
  const currentReport = reportWithReviewScopes("visibility");
  const currentDetail = campaignDetail("campaign-current");
  currentDetail.summary.action_ids = ["action-a"];
  useStockReviewStore.setState({
    filters: filters("account-a"),
    report: currentReport,
    selectedCampaign: currentDetail,
  });
  const responses = [
    savedAnnotation({ id: "cross-account", account_id: "account-b" }),
    savedAnnotation({
      id: "future",
      account_id: "account-a",
      value_json: '{"effective_date":"2026-08-29"}',
    }),
    savedAnnotation({
      id: "other-campaign",
      scope_type: "campaign",
      scope_key: "campaign-old",
      account_id: "account-a",
      symbol: "AAPL",
    }),
    savedAnnotation({
      id: "current-campaign",
      scope_type: "campaign",
      scope_key: "campaign-current",
      account_id: "account-a",
      symbol: "AAPL",
    }),
    savedAnnotation({ id: "global" }),
  ];
  invokeImpl = async (command) => {
    assert.equal(command, "save_stock_review_annotation");
    return responses.shift();
  };

  for (const response of [...responses]) {
    await useStockReviewStore.getState().saveAnnotation({
      id: response.id,
      scope_type: response.scope_type,
      scope_key: response.scope_key,
      account_id: response.account_id,
      symbol: response.symbol,
      annotation_type: response.annotation_type,
      value_json: response.value_json,
      source: "user",
    });
  }

  assert.deepEqual(
    useStockReviewStore.getState().report.annotations.map((item) => item.id),
    ["other-campaign", "current-campaign", "global"],
  );
  assert.deepEqual(
    useStockReviewStore.getState().selectedCampaign.annotations.map((item) => item.id),
    ["current-campaign", "global"],
  );
  assert.equal(useStockReviewStore.getState().report.summary, currentReport.summary);
});

test("override result preserves a visible annotation that completes before it", async () => {
  const overrideRequest = deferred();
  const annotationRequest = deferred();
  const before = reportWithReviewScopes("before");
  const overridden = reportWithReviewScopes("overridden");
  useStockReviewStore.setState({ filters: filters("account-a"), report: before });
  invokeImpl = (command) =>
    command === "confirm_stock_review_override"
      ? overrideRequest.promise
      : annotationRequest.promise;

  const overrideSave = useStockReviewStore.getState().confirmOverride(filters("account-a"), {
    id: "override-1",
    override_type: "non_trade",
    transaction_ids_json: '["tx-1"]',
    value_json: "{}",
  });
  const annotationSave = useStockReviewStore.getState().saveAnnotation({
    id: "during-override",
    scope_type: "period",
    scope_key: "2026-01-01:2026-08-28",
    account_id: null,
    symbol: null,
    annotation_type: "note",
    value_json: "{}",
    source: "user",
  });
  annotationRequest.resolve(savedAnnotation({ id: "during-override" }));
  await annotationSave;
  overrideRequest.resolve(overridden);
  await overrideSave;

  assert.equal(useStockReviewStore.getState().report.methodology, overridden.methodology);
  assert.deepEqual(
    useStockReviewStore.getState().report.annotations.map((item) => item.id),
    ["during-override"],
  );
});

test("annotation completion after override resolution applies to the returned report", async () => {
  const overrideRequest = deferred();
  const annotationRequest = deferred();
  const overridden = reportWithReviewScopes("overridden-first");
  useStockReviewStore.setState({
    filters: filters("account-a"),
    report: reportWithReviewScopes("before"),
  });
  invokeImpl = (command) =>
    command === "confirm_stock_review_override"
      ? overrideRequest.promise
      : annotationRequest.promise;

  const overrideSave = useStockReviewStore.getState().confirmOverride(filters("account-a"), {
    id: "override-1",
    override_type: "non_trade",
    transaction_ids_json: '["tx-1"]',
    value_json: "{}",
  });
  const annotationSave = useStockReviewStore.getState().saveAnnotation({
    id: "after-override",
    scope_type: "period",
    scope_key: "2026-01-01:2026-08-28",
    account_id: null,
    symbol: null,
    annotation_type: "note",
    value_json: "{}",
    source: "user",
  });
  overrideRequest.resolve(overridden);
  await overrideSave;
  annotationRequest.resolve(savedAnnotation({ id: "after-override" }));
  await annotationSave;

  assert.equal(useStockReviewStore.getState().report.methodology, overridden.methodology);
  assert.deepEqual(
    useStockReviewStore.getState().report.annotations.map((item) => item.id),
    ["after-override"],
  );
});

test("an annotation error remains displayable when an overlapping override succeeds", async () => {
  const overrideRequest = deferred();
  const annotationRequest = deferred();
  useStockReviewStore.setState({
    filters: filters("account-a"),
    report: reportWithReviewScopes("before"),
  });
  invokeImpl = (command) =>
    command === "confirm_stock_review_override"
      ? overrideRequest.promise
      : annotationRequest.promise;

  const overrideSave = useStockReviewStore.getState().confirmOverride(filters("account-a"), {
    id: "override-1",
    override_type: "non_trade",
    transaction_ids_json: '["tx-1"]',
    value_json: "{}",
  });
  const annotationSave = useStockReviewStore.getState().saveAnnotation({
    id: "failed-note",
    scope_type: "period",
    scope_key: "2026-01-01:2026-08-28",
    account_id: null,
    symbol: null,
    annotation_type: "note",
    value_json: "{}",
    source: "user",
  });
  annotationRequest.reject(new Error("annotation failed"));
  await annotationSave;
  overrideRequest.resolve(reportWithReviewScopes("override-success"));
  await overrideSave;

  assert.match(useStockReviewStore.getState().error, /annotation failed/);
  assert.equal(useStockReviewStore.getState().errorSource, "annotation");
});

test("two overrides remain latest-wins independently of annotation sequencing", async () => {
  const first = deferred();
  const second = deferred();
  invokeImpl = (_command, args) =>
    args.input.id === "override-first" ? first.promise : second.promise;
  useStockReviewStore.setState({
    filters: filters("account-a"),
    report: reportWithReviewScopes("before"),
  });

  const firstSave = useStockReviewStore.getState().confirmOverride(filters("account-a"), {
    id: "override-first",
    override_type: "non_trade",
    transaction_ids_json: '["tx-1"]',
    value_json: "{}",
  });
  const secondSave = useStockReviewStore.getState().confirmOverride(filters("account-a"), {
    id: "override-second",
    override_type: "non_trade",
    transaction_ids_json: '["tx-2"]',
    value_json: "{}",
  });
  const latest = reportWithReviewScopes("latest-override");
  second.resolve(latest);
  await secondSave;
  first.resolve(reportWithReviewScopes("stale-override"));
  await firstSave;

  assert.equal(useStockReviewStore.getState().report, latest);
});

test("filter change invalidates all pending overrides", async () => {
  const first = deferred();
  const second = deferred();
  const latestReport = reportWithReviewScopes("account-b", "account-b");
  invokeImpl = (command, args) => {
    if (command === "get_stock_review_report") return Promise.resolve(latestReport);
    return args.input.id === "override-first" ? first.promise : second.promise;
  };
  useStockReviewStore.setState({
    filters: filters("account-a"),
    report: reportWithReviewScopes("account-a", "account-a"),
  });

  const firstSave = useStockReviewStore.getState().confirmOverride(filters("account-a"), {
    id: "override-first",
    override_type: "non_trade",
    transaction_ids_json: '["tx-1"]',
    value_json: "{}",
  });
  const secondSave = useStockReviewStore.getState().confirmOverride(filters("account-a"), {
    id: "override-second",
    override_type: "non_trade",
    transaction_ids_json: '["tx-2"]',
    value_json: "{}",
  });
  await useStockReviewStore.getState().loadReport(filters("account-b"));
  second.resolve(reportWithReviewScopes("stale-second"));
  first.resolve(reportWithReviewScopes("stale-first"));
  await Promise.all([firstSave, secondSave]);

  assert.equal(useStockReviewStore.getState().report, latestReport);
  assert.deepEqual(useStockReviewStore.getState().filters, filters("account-b"));
});
