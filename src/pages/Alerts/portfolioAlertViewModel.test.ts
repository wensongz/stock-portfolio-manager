// @ts-nocheck
import test from "node:test";
import assert from "node:assert/strict";
import {
  accountScope,
  buildPortfolioAlertDisplayModel,
  buildPortfolioAlertNotificationPresentation,
  buildPortfolioAlertScopeOptions,
  decideDeletedPortfolioAlertScopeTransition,
  marketScope,
  mergePortfolioAlertDraftCategories,
  overallScope,
  resolvePortfolioAlertCurrency,
  resolvePortfolioAlertScope,
  validatePortfolioAlertDraft,
} from "./portfolioAlertViewModel.ts";

function account(id, name, market) {
  return {
    id,
    name,
    market,
    description: null,
    created_at: "2026-09-06",
    updated_at: "2026-09-06",
  };
}

function category(id, name, sortOrder, color = "#1677ff", icon = "📈") {
  return {
    id,
    name,
    color,
    icon,
    is_system: false,
    sort_order: sortOrder,
    created_at: "2026-09-06",
  };
}

function draft(targetPercents, overrides = {}) {
  return {
    id: null,
    scope: overallScope(),
    baseCurrency: "USD",
    deviationThreshold: 20,
    concentrationThreshold: 20,
    isActive: true,
    targets: targetPercents.map((targetPercent, index) => ({
      categoryId: `category-${index}`,
      targetPercent,
    })),
    ...overrides,
  };
}

function allocation(overrides = {}) {
  return {
    categoryId: "growth",
    categoryName: "成长",
    categoryColor: "#00aa00",
    categoryIcon: "🚀",
    targetPercent: 50,
    currentPercent: 60,
    relativeDeviationPercent: 20,
    currentMarketValue: 600.123456,
    targetMarketValue: 500,
    rebalanceAmount: -100.123456,
    direction: null,
    ...overrides,
  };
}

function snapshot(overrides = {}) {
  return {
    configId: "config-overall",
    scope: overallScope(),
    baseCurrency: "USD",
    evaluatedAt: "2026-09-06T08:00:00Z",
    totalMarketValue: 1000.123456,
    categories: [
      allocation(),
      allocation({
        categoryId: "cash",
        categoryName: "现金",
        categoryColor: "#ffaa00",
        categoryIcon: "💵",
        targetPercent: 40,
        currentPercent: 30,
        relativeDeviationPercent: 25,
        currentMarketValue: 300,
        targetMarketValue: 400,
        rebalanceAmount: 100,
        direction: "UNDERWEIGHT",
      }),
      allocation({
        categoryId: null,
        categoryName: "未分类",
        categoryColor: "#999999",
        categoryIcon: "❓",
        targetPercent: 0,
        currentPercent: 10,
        relativeDeviationPercent: null,
        currentMarketValue: 100,
        targetMarketValue: 0,
        rebalanceAmount: -100,
        direction: "OVERWEIGHT",
      }),
    ],
    concentrations: [],
    ...overrides,
  };
}

function config(overrides = {}) {
  return {
    id: "config-overall",
    scope: overallScope(),
    baseCurrency: "USD",
    deviationThreshold: 20,
    concentrationThreshold: 20,
    isActive: true,
    targets: [
      { categoryId: "growth", targetPercent: 50 },
      { categoryId: "cash", targetPercent: 40 },
    ],
    lastSnapshot: null,
    lastEvaluatedAt: null,
    ...overrides,
  };
}

function evaluation(overrides = {}) {
  return {
    status: "READY",
    snapshot: snapshot(),
    stale: false,
    missingData: [],
    activeBreaches: [],
    newlyTriggered: [],
    ...overrides,
  };
}

function view(evaluationOverrides = {}, configOverrides = {}) {
  return {
    config: config(configOverrides),
    evaluation: evaluation(evaluationOverrides),
  };
}

test("scope options contain overall, three markets, then every account", () => {
  const options = buildPortfolioAlertScopeOptions([
    account("acct-us", "美股主账户", "US"),
    account("acct-hk", "港股账户", "HK"),
  ]);

  assert.deepEqual(options.map((item) => item.label), [
    "整体组合",
    "A股组合",
    "美股组合",
    "港股组合",
    "美股主账户",
    "港股账户",
  ]);
  assert.deepEqual(options.map((item) => item.scope.kind), [
    "OVERALL",
    "MARKET",
    "MARKET",
    "MARKET",
    "ACCOUNT",
    "ACCOUNT",
  ]);
});

test("a deleted account selection falls back to overall", () => {
  const options = buildPortfolioAlertScopeOptions([]);
  assert.deepEqual(
    resolvePortfolioAlertScope(accountScope("deleted-account"), options),
    overallScope(),
  );
});

test("market and account scopes derive their native currency", () => {
  const accounts = [account("acct-cn", "A股", "CN"), account("acct-hk", "港股", "HK")];

  assert.equal(resolvePortfolioAlertCurrency(overallScope(), accounts, "CNY"), "CNY");
  assert.equal(resolvePortfolioAlertCurrency(marketScope("US"), accounts, "CNY"), "USD");
  assert.equal(resolvePortfolioAlertCurrency(marketScope("HK"), accounts, "CNY"), "HKD");
  assert.equal(resolvePortfolioAlertCurrency(accountScope("acct-cn"), accounts, "USD"), "CNY");
  assert.equal(resolvePortfolioAlertCurrency(accountScope("acct-hk"), accounts, "USD"), "HKD");
});

test("fresh Settings categories preserve existing targets, add zero targets, and remove deleted categories", () => {
  const merged = mergePortfolioAlertDraftCategories(
    draft([], {
      targets: [
        { categoryId: "growth", targetPercent: 70 },
        { categoryId: "deleted", targetPercent: 30 },
      ],
    }),
    [category("new", "新类别", 20), category("growth", "成长", 10)],
  );

  assert.deepEqual(merged.targets, [
    { categoryId: "growth", targetPercent: 70 },
    { categoryId: "new", targetPercent: 0 },
  ]);
});

test("draft validation requires target total within one basis point of 100", () => {
  assert.equal(validatePortfolioAlertDraft(draft([60, 39.98])).valid, false);
  assert.equal(validatePortfolioAlertDraft(draft([60, 39.99])).valid, true);
  assert.equal(validatePortfolioAlertDraft(draft([60, 40.01])).valid, true);
  assert.equal(validatePortfolioAlertDraft(draft([60, 40.02])).valid, false);
});

test("draft validation accepts floating-point endpoint noise but rejects values just outside it", () => {
  assert.equal(validatePortfolioAlertDraft(draft([33.33, 66.66])).valid, true);
  assert.equal(validatePortfolioAlertDraft(draft([33.34, 66.67])).valid, true);
  assert.equal(validatePortfolioAlertDraft(draft([60, 39.989999])).valid, false);
  assert.equal(validatePortfolioAlertDraft(draft([60, 40.010001])).valid, false);
});

test("draft validation rejects non-finite and out-of-range fields", () => {
  const result = validatePortfolioAlertDraft(draft([100, Number.NaN], {
    deviationThreshold: Number.POSITIVE_INFINITY,
    concentrationThreshold: 0,
  }));

  assert.equal(result.valid, false);
  assert.match(result.targetErrors["category-1"], /0% 到 100%/);
  assert.match(result.deviationError, /0% 到 100%/);
  assert.match(result.concentrationError, /大于 0%/);
});

test("chart rows retain unrounded values while labels are formatted", () => {
  const model = buildPortfolioAlertDisplayModel(view());

  assert.equal(model.pieData[0].value, 600.123456);
  assert.equal(model.rows[0].currentMarketValue, 600.123456);
  assert.equal(model.rows[0].rebalanceAmount, -100.123456);
  assert.equal(model.rows[0].currentPercentLabel, "60.00%");
  assert.equal(model.rows[0].rebalanceAmountLabel, "-100.12 USD");
});

test("incomplete evaluation shows its prior snapshot as stale and describes missing data", () => {
  const model = buildPortfolioAlertDisplayModel(view({
    status: "INCOMPLETE",
    stale: true,
    missingData: [{
      market: "US",
      symbol: "AAPL",
      currency: null,
      reason: "cached quote is unavailable",
    }],
  }), [
    category("growth", "成长", 10),
    category("cash", "现金", 20),
  ]);

  assert.equal(model.stale, true);
  assert.equal(model.statusLabel, "数据不完整");
  assert.deepEqual(model.pieData.map((row) => row.name), ["成长", "现金", "未分类"]);
  assert.match(model.banner ?? "", /等待有效数据/);
  assert.match(model.missingDataDescriptions[0], /US AAPL/);
  assert.equal(model.canAskAi, false);
});

test("empty and invalid configuration states expose distinct guidance", () => {
  const empty = buildPortfolioAlertDisplayModel(view({ status: "EMPTY", snapshot: null }));
  const invalid = buildPortfolioAlertDisplayModel(view({
    status: "INVALID_CONFIG",
    snapshot: snapshot(),
    stale: true,
  }));

  assert.equal(empty.statusLabel, "暂无可评估持仓");
  assert.match(empty.banner ?? "", /暂无可评估持仓/);
  assert.equal(invalid.statusLabel, "配置无效");
  assert.match(invalid.banner ?? "", /重新保存/);
  assert.equal(invalid.canAskAi, false);
});

test("EMPTY never revives a config's cleared last snapshot", () => {
  const model = buildPortfolioAlertDisplayModel(view({
    status: "EMPTY",
    snapshot: null,
    stale: false,
  }, {
    lastSnapshot: snapshot({
      concentrations: [{
        market: "US",
        symbol: "AAPL",
        normalizedSymbol: "AAPL",
        name: "Apple",
        categoryId: "growth",
        marketValue: 600,
        positionPercent: 60,
        thresholdPercent: 20,
      }],
    }),
  }), [
    category("growth", "成长", 10),
    category("cash", "现金", 20),
  ]);

  assert.equal(model.stale, false);
  assert.deepEqual(model.pieData, []);
  assert.deepEqual(model.rows, []);
  assert.deepEqual(model.concentrationRows, []);
});

test("an unconfigured scope is explicit and defaults to no AI action", () => {
  const model = buildPortfolioAlertDisplayModel(undefined);

  assert.equal(model.statusLabel, "未配置");
  assert.match(model.banner ?? "", /设置目标/);
  assert.equal(model.canAskAi, false);
});

test("a disabled configuration marks its retained snapshot as historical", () => {
  const model = buildPortfolioAlertDisplayModel({
    config: config({ isActive: false, lastSnapshot: snapshot() }),
    evaluation: null,
  });

  assert.equal(model.statusLabel, "已停用");
  assert.equal(model.stale, true);
  assert.match(model.banner ?? "", /历史快照/);
});

test("exact threshold equality is normal while larger deviations retain direction", () => {
  const model = buildPortfolioAlertDisplayModel(view({
    snapshot: snapshot({
      categories: [
        allocation({ relativeDeviationPercent: 20, direction: "OVERWEIGHT" }),
        allocation({
          categoryId: "income",
          categoryName: "收益",
          relativeDeviationPercent: 20.000001,
          direction: "UNDERWEIGHT",
        }),
      ],
    }),
  }));

  assert.equal(model.rows[0].status, "NORMAL");
  assert.equal(model.rows[0].statusLabel, "正常");
  assert.equal(model.rows[1].status, "UNDERWEIGHT");
  assert.equal(model.rows[1].statusLabel, "欠配");
});

test("overweight, underweight, and concentration rows expose actionable labels", () => {
  const model = buildPortfolioAlertDisplayModel(view({
    snapshot: snapshot({
      categories: [
        allocation({ relativeDeviationPercent: 20.1, direction: "OVERWEIGHT" }),
        allocation({
          categoryId: "cash",
          categoryName: "现金",
          relativeDeviationPercent: 25,
          direction: "UNDERWEIGHT",
          rebalanceAmount: 100,
        }),
      ],
      concentrations: [{
        market: "US",
        symbol: "AAPL",
        normalizedSymbol: "AAPL",
        name: "Apple",
        categoryId: "growth",
        marketValue: 260.456,
        positionPercent: 26.0456,
        thresholdPercent: 20,
      }],
    }),
  }));

  assert.deepEqual(model.rows.map((row) => row.statusLabel), ["超配", "欠配"]);
  assert.equal(model.concentrationRows[0].marketValue, 260.456);
  assert.match(model.concentrationRows[0].warning, /26.05%/);
  assert.match(model.concentrationRows[0].marketValueLabel, /260.46 USD/);
});

test("AI is enabled only for a ready evaluation with a currently active breach", () => {
  const activeBreach = {
    configId: "config-overall",
    breachKey: "category:growth",
    breachKind: "CATEGORY_DEVIATION",
    direction: "OVERWEIGHT",
    firstTriggeredAt: "2026-09-06T08:00:00Z",
    lastSeenAt: "2026-09-06T08:00:00Z",
  };

  assert.equal(buildPortfolioAlertDisplayModel(view()).canAskAi, false);
  assert.equal(buildPortfolioAlertDisplayModel(view({ activeBreaches: [activeBreach] })).canAskAi, true);
  assert.equal(buildPortfolioAlertDisplayModel(view({
    activeBreaches: [activeBreach],
  }, { isActive: false })).canAskAi, false);
});

test("green ready-normal success is exclusive to active, fresh READY views without breaches", () => {
  const cases = [
    ["ready normal", view(), true],
    ["unconfigured", undefined, false],
    ["inactive", view({}, { isActive: false }), false],
    ["empty", view({ status: "EMPTY", snapshot: null }), false],
    ["incomplete", view({ status: "INCOMPLETE", stale: true }), false],
    ["invalid", view({ status: "INVALID_CONFIG", stale: true }), false],
    ["stale ready", view({ status: "READY", stale: true }), false],
  ];

  for (const [label, candidate, expected] of cases) {
    assert.equal(
      buildPortfolioAlertDisplayModel(candidate).showReadyNormalSuccess,
      expected,
      label,
    );
  }
});

test("command and event breaches can share one notification presentation", () => {
  const presentation = buildPortfolioAlertNotificationPresentation({
    configId: "config-overall",
    breachKey: "security:US:AAPL",
    breachKind: "CONCENTRATION",
    direction: "ABOVE_LIMIT",
    firstTriggeredAt: "2026-09-06T08:00:00Z",
    lastSeenAt: "2026-09-06T08:00:00Z",
  });

  assert.equal(presentation.title, "组合提醒已触发");
  assert.match(presentation.description, /单票集中度/);
  assert.match(presentation.description, /AAPL/);
});

test("fresh categories join display rows in Settings order without making uncategorized editable", () => {
  const categories = [
    category("new", "新类别", 30, "#333333", "🆕"),
    category("cash", "现金", 20, "#ffaa00", "💵"),
    category("growth", "成长", 10, "#00aa00", "🚀"),
  ];
  const model = buildPortfolioAlertDisplayModel(view(), categories);

  assert.deepEqual(model.rows.map((row) => row.name), ["成长", "现金", "新类别", "未分类"]);
  assert.equal(model.rows[2].targetPercent, 0);
  assert.equal(model.rows[2].editable, true);
  assert.equal(model.rows[3].editable, false);
});

test("deleted account fallback is automatic only for a clean draft", () => {
  const options = buildPortfolioAlertScopeOptions([]);

  assert.deepEqual(
    decideDeletedPortfolioAlertScopeTransition(
      accountScope("deleted-account"),
      options,
      false,
      null,
    ),
    {
      action: "FALLBACK",
      fallbackScope: overallScope(),
      transitionKey: "account:deleted-account",
    },
  );
});

test("deleted dirty account requires one confirmation and preserves state after cancellation", () => {
  const selected = accountScope("deleted-account");
  const options = buildPortfolioAlertScopeOptions([]);

  assert.deepEqual(
    decideDeletedPortfolioAlertScopeTransition(selected, options, true, null),
    {
      action: "CONFIRM",
      fallbackScope: overallScope(),
      transitionKey: "account:deleted-account",
    },
  );
  assert.deepEqual(
    decideDeletedPortfolioAlertScopeTransition(
      selected,
      options,
      true,
      "account:deleted-account",
    ),
    {
      action: "PRESERVE",
      transitionKey: "account:deleted-account",
    },
  );
});
