import type {
  Account,
  AllocationDirection,
  Category,
  PortfolioAlertBreach,
  PortfolioAlertCurrency,
  PortfolioAlertScope,
  PortfolioAlertView,
  SavePortfolioAlertConfigInput,
} from "../../types";

export type PortfolioAlertDraft = SavePortfolioAlertConfigInput;

export interface ScopeOption {
  value: string;
  label: string;
  scope: PortfolioAlertScope;
}

export type DeletedPortfolioAlertScopeTransition =
  | { action: "FALLBACK" | "CONFIRM"; fallbackScope: PortfolioAlertScope; transitionKey: string }
  | { action: "PRESERVE"; transitionKey: string }
  | { action: "NONE" };

export interface DraftValidation {
  valid: boolean;
  totalTarget: number;
  totalError: string | null;
  deviationError: string | null;
  concentrationError: string | null;
  targetErrors: Record<string, string>;
  errors: string[];
}

export type PortfolioAlertRowStatus = "NORMAL" | AllocationDirection;

export interface PortfolioAlertDisplayRow {
  key: string;
  categoryId: string | null;
  name: string;
  icon: string;
  color: string;
  editable: boolean;
  targetPercent: number;
  currentPercent: number;
  relativeDeviationPercent: number | null;
  currentMarketValue: number;
  targetMarketValue: number;
  rebalanceAmount: number;
  targetPercentLabel: string;
  currentPercentLabel: string;
  relativeDeviationLabel: string;
  currentMarketValueLabel: string;
  targetMarketValueLabel: string;
  rebalanceAmountLabel: string;
  status: PortfolioAlertRowStatus;
  statusLabel: string;
  statusColor: string;
}

export interface PortfolioAlertTargetEditorRow {
  key: string;
  categoryId: string;
  name: string;
  icon: string;
  color: string;
  targetPercent: number;
}

export interface PortfolioAlertWorkspaceRows {
  evaluatedRows: PortfolioAlertDisplayRow[];
  targetEditorRows: PortfolioAlertTargetEditorRow[];
}

export interface PortfolioAlertConcentrationRow {
  key: string;
  market: string;
  symbol: string;
  name: string;
  marketValue: number;
  positionPercent: number;
  thresholdPercent: number;
  marketValueLabel: string;
  positionPercentLabel: string;
  thresholdPercentLabel: string;
  warning: string;
}

export interface PortfolioAlertDisplayModel {
  configId: string | null;
  currency: PortfolioAlertCurrency;
  statusLabel: string;
  statusColor: string;
  banner: string | null;
  stale: boolean;
  snapshotEvaluatedAt: string | null;
  rows: PortfolioAlertDisplayRow[];
  concentrationRows: PortfolioAlertConcentrationRow[];
  totalTargetPercent: number;
  totalCurrentPercent: number;
  totalTargetLabel: string;
  totalCurrentLabel: string;
  missingDataDescriptions: string[];
  showReadyNormalSuccess: boolean;
  canAskAi: boolean;
  aiDisabledReason: string | null;
}

export interface PortfolioAlertNotificationPresentation {
  title: string;
  description: string;
}

export function overallScope(): PortfolioAlertScope {
  return { kind: "OVERALL", market: null, accountId: null };
}

export function marketScope(
  market: "CN" | "US" | "HK",
): PortfolioAlertScope {
  return { kind: "MARKET", market, accountId: null };
}

export function accountScope(accountId: string): PortfolioAlertScope {
  return { kind: "ACCOUNT", market: null, accountId };
}

export function buildPortfolioAlertScopeOptions(
  accounts: Account[],
): ScopeOption[] {
  const fixed: ScopeOption[] = [
    { value: "overall", label: "整体组合", scope: overallScope() },
    { value: "market:CN", label: "A股组合", scope: marketScope("CN") },
    { value: "market:US", label: "美股组合", scope: marketScope("US") },
    { value: "market:HK", label: "港股组合", scope: marketScope("HK") },
  ];
  return [
    ...fixed,
    ...accounts.map((account) => ({
      value: `account:${account.id}`,
      label: account.name,
      scope: accountScope(account.id),
    })),
  ];
}

export function resolvePortfolioAlertScope(
  scope: PortfolioAlertScope,
  options: ScopeOption[],
): PortfolioAlertScope {
  const key = scope.kind === "OVERALL"
    ? "overall"
    : scope.kind === "MARKET"
      ? `market:${scope.market}`
      : `account:${scope.accountId}`;
  return options.find((option) => option.value === key)?.scope
    ?? overallScope();
}

export function decideDeletedPortfolioAlertScopeTransition(
  selectedScope: PortfolioAlertScope,
  options: ScopeOption[],
  dirty: boolean,
  declinedTransitionKey: string | null,
): DeletedPortfolioAlertScopeTransition {
  if (selectedScope.kind !== "ACCOUNT") return { action: "NONE" };
  const transitionKey = `account:${selectedScope.accountId}`;
  if (options.some((option) => option.value === transitionKey)) {
    return { action: "NONE" };
  }
  if (!dirty) {
    return { action: "FALLBACK", fallbackScope: overallScope(), transitionKey };
  }
  if (declinedTransitionKey === transitionKey) {
    return { action: "PRESERVE", transitionKey };
  }
  return { action: "CONFIRM", fallbackScope: overallScope(), transitionKey };
}

const MARKET_CURRENCIES = {
  CN: "CNY",
  US: "USD",
  HK: "HKD",
} as const;

export function resolvePortfolioAlertCurrency(
  scope: PortfolioAlertScope,
  accounts: Account[],
  overallCurrency: PortfolioAlertCurrency,
): PortfolioAlertCurrency {
  if (scope.kind === "OVERALL") return overallCurrency;
  if (scope.kind === "MARKET") return MARKET_CURRENCIES[scope.market];
  const market = accounts.find((account) => account.id === scope.accountId)?.market;
  return market ? MARKET_CURRENCIES[market] : overallCurrency;
}

function orderedCategories(categories: Category[]): Category[] {
  return [...categories].sort((left, right) =>
    left.sort_order - right.sort_order || left.name.localeCompare(right.name, "zh-CN"),
  );
}

export function mergePortfolioAlertDraftCategories(
  draft: PortfolioAlertDraft,
  categories: Category[],
): PortfolioAlertDraft {
  const targetByCategory = new Map(
    draft.targets.map((target) => [target.categoryId, target.targetPercent]),
  );
  return {
    ...draft,
    targets: orderedCategories(categories).map((category) => ({
      categoryId: category.id,
      targetPercent: targetByCategory.get(category.id) ?? 0,
    })),
  };
}

export function selectPortfolioAlertWorkspaceRows(
  evaluatedRows: PortfolioAlertDisplayRow[],
  draft: PortfolioAlertDraft,
  categories: Category[],
  editing: boolean,
): PortfolioAlertWorkspaceRows {
  const mergedDraft = mergePortfolioAlertDraftCategories(draft, categories);
  const targetByCategory = new Map(
    mergedDraft.targets.map((target) => [target.categoryId, target.targetPercent]),
  );
  return {
    evaluatedRows,
    targetEditorRows: editing
      ? orderedCategories(categories).map((category) => ({
          key: category.id,
          categoryId: category.id,
          name: category.name,
          icon: category.icon,
          color: category.color,
          targetPercent: targetByCategory.get(category.id) ?? 0,
        }))
      : [],
  };
}

const TOTAL_TOLERANCE = 0.01;
const FLOAT_TOLERANCE = 1e-9;
const PERCENT_RANGE_ERROR = "目标占比必须是 0% 到 100% 之间的有限数字";

function validClosedPercentage(value: number): boolean {
  return Number.isFinite(value) && value >= 0 && value <= 100;
}

export function validatePortfolioAlertDraft(
  draft: PortfolioAlertDraft,
): DraftValidation {
  const targetErrors: Record<string, string> = {};
  const seen = new Set<string>();
  for (const target of draft.targets) {
    if (!validClosedPercentage(target.targetPercent)) {
      targetErrors[target.categoryId] = PERCENT_RANGE_ERROR;
    } else if (seen.has(target.categoryId)) {
      targetErrors[target.categoryId] = "每个投资类别只能设置一次目标占比";
    }
    seen.add(target.categoryId);
  }

  const totalTarget = draft.targets.reduce(
    (total, target) => total + target.targetPercent,
    0,
  );
  const totalValid = Number.isFinite(totalTarget)
    && Math.abs(totalTarget - 100) <= TOTAL_TOLERANCE + FLOAT_TOLERANCE;
  const totalError = totalValid
    ? null
    : "目标占比合计必须在 99.99% 到 100.01% 之间";
  const deviationError = validClosedPercentage(draft.deviationThreshold)
    ? null
    : "偏离度阈值必须是 0% 到 100% 之间的有限数字";
  const concentrationError = Number.isFinite(draft.concentrationThreshold)
    && draft.concentrationThreshold > 0
    && draft.concentrationThreshold <= 100
    ? null
    : "单票集中度阈值必须大于 0% 且不超过 100%";
  const errors = [
    ...Object.values(targetErrors),
    totalError,
    deviationError,
    concentrationError,
  ].filter((error): error is string => error !== null);

  return {
    valid: errors.length === 0,
    totalTarget,
    totalError,
    deviationError,
    concentrationError,
    targetErrors,
    errors,
  };
}

export function buildPortfolioAlertNotificationPresentation(
  breach: PortfolioAlertBreach,
): PortfolioAlertNotificationPresentation {
  const breachParts = breach.breachKey.split(":");
  const subject = breach.breachKey.startsWith("category:")
    ? `投资类别 ${breach.breachKey.slice("category:".length)}`
    : `标的 ${breachParts[breachParts.length - 1] ?? breach.breachKey}`;
  const reason = breach.breachKind === "CONCENTRATION"
    ? "单票集中度超过阈值"
    : breach.direction === "UNDERWEIGHT"
      ? "类别配置欠配"
      : "类别配置超配";
  return {
    title: "组合提醒已触发",
    description: `${subject}：${reason}`,
  };
}

function formatNumber(value: number): string {
  return value.toLocaleString("zh-CN", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  });
}

function formatPercent(value: number): string {
  return `${formatNumber(value)}%`;
}

function formatAmount(value: number, currency: PortfolioAlertCurrency): string {
  return `${formatNumber(value)} ${currency}`;
}

function rowStatus(
  targetPercent: number,
  currentPercent: number,
  relativeDeviationPercent: number | null,
  direction: AllocationDirection | null,
  threshold: number,
): PortfolioAlertRowStatus {
  if (targetPercent === 0 && currentPercent > 0) return "OVERWEIGHT";
  if (relativeDeviationPercent === null || relativeDeviationPercent <= threshold) {
    return "NORMAL";
  }
  return direction ?? (currentPercent > targetPercent ? "OVERWEIGHT" : "UNDERWEIGHT");
}

function missingDataDescription(
  item: NonNullable<PortfolioAlertView["evaluation"]>["missingData"][number],
): string {
  const subject = [item.market, item.symbol, item.currency].filter(Boolean).join(" ");
  const reason = item.reason === "cached quote is unavailable"
    ? "缺少缓存行情"
    : item.reason === "cached quote has a negative value"
      ? "缓存行情为负数"
      : item.reason.replace("exchange rate", "汇率").replace("is unavailable", "不可用");
  return subject ? `${subject}：${reason}` : reason;
}

function statusPresentation(view?: PortfolioAlertView): {
  label: string;
  color: string;
  banner: string | null;
} {
  if (!view?.config) {
    return {
      label: "未配置",
      color: "default",
      banner: "当前范围尚未配置组合提醒，请设置目标占比后保存。",
    };
  }
  if (!view.config.isActive) {
    return {
      label: "已停用",
      color: "default",
      banner: view.config.lastSnapshot
        ? "当前范围的组合提醒已停用；下方展示的是历史快照，并非实时结果。"
        : "当前范围的组合提醒已停用。",
    };
  }
  const evaluation = view.evaluation;
  if (!evaluation) {
    return { label: "等待评估", color: "processing", banner: "正在等待首次组合评估。" };
  }
  switch (evaluation.status) {
    case "EMPTY":
      return {
        label: "暂无可评估持仓",
        color: "default",
        banner: "当前范围暂无可评估持仓，不会产生组合提醒。",
      };
    case "INCOMPLETE":
      return {
        label: "数据不完整",
        color: "warning",
        banner: "等待有效数据；当前展示的是最后一次成功评估快照，并非实时结果。",
      };
    case "INVALID_CONFIG":
      return {
        label: "配置无效",
        color: "error",
        banner: "投资类别发生变化后目标合计已失效，请调整目标并重新保存。",
      };
    case "READY":
      return evaluation.activeBreaches.length > 0
        ? { label: "需要调整", color: "error", banner: "当前组合存在需要处理的配置偏离。" }
        : { label: "正常", color: "success", banner: null };
  }
}

export function buildPortfolioAlertDisplayModel(
  view?: PortfolioAlertView,
  categories?: Category[],
): PortfolioAlertDisplayModel {
  const config = view?.config ?? null;
  const evaluation = view?.evaluation ?? null;
  const snapshot = evaluation?.snapshot
    ?? (evaluation?.stale || (config !== null && !config.isActive && evaluation === null)
      ? config?.lastSnapshot ?? null
      : null);
  const currency = snapshot?.baseCurrency ?? config?.baseCurrency ?? "USD";
  const threshold = config?.deviationThreshold ?? 20;
  const sourceRows = snapshot?.categories ?? [];
  const allocationByCategory = new Map(
    sourceRows
      .filter((row) => row.categoryId !== null)
      .map((row) => [row.categoryId as string, row]),
  );
  const targetByCategory = new Map(
    config?.targets.map((target) => [target.categoryId, target.targetPercent]) ?? [],
  );
  const uncategorized = sourceRows.find((row) => row.categoryId === null);
  const showUncategorized = uncategorized
    ? [
      uncategorized.targetPercent,
      uncategorized.currentPercent,
      uncategorized.relativeDeviationPercent,
      uncategorized.currentMarketValue,
      uncategorized.targetMarketValue,
      uncategorized.rebalanceAmount,
    ].some((value) => value !== null && value !== 0)
    : false;
  const mergedRows = evaluation?.status === "EMPTY" || snapshot === null
    ? []
    : categories
      ? [
        ...orderedCategories(categories).map((category) => {
          const existing = allocationByCategory.get(category.id);
          const targetPercent = targetByCategory.get(category.id) ?? existing?.targetPercent ?? 0;
          return existing
            ? {
                ...existing,
                categoryName: category.name,
                categoryColor: category.color,
                categoryIcon: category.icon,
                targetPercent,
              }
            : {
                categoryId: category.id,
                categoryName: category.name,
                categoryColor: category.color,
                categoryIcon: category.icon,
                targetPercent,
                currentPercent: 0,
                relativeDeviationPercent: targetPercent > 0 ? 100 : null,
                currentMarketValue: 0,
                targetMarketValue: snapshot
                  ? snapshot.totalMarketValue * targetPercent / 100
                  : 0,
                rebalanceAmount: snapshot
                  ? snapshot.totalMarketValue * targetPercent / 100
                  : 0,
                direction: targetPercent > 0 ? "UNDERWEIGHT" as const : null,
              };
        }),
        ...(uncategorized && showUncategorized ? [uncategorized] : []),
        ]
      : sourceRows.filter((row) => row.categoryId !== null || showUncategorized);

  const rows: PortfolioAlertDisplayRow[] = mergedRows.map((row) => {
    const status = snapshot
      ? rowStatus(
          row.targetPercent,
          row.currentPercent,
          row.relativeDeviationPercent,
          row.direction,
          threshold,
        )
      : "NORMAL";
    const statusLabels: Record<PortfolioAlertRowStatus, string> = {
      NORMAL: "正常",
      OVERWEIGHT: "超配",
      UNDERWEIGHT: "欠配",
    };
    const statusColors: Record<PortfolioAlertRowStatus, string> = {
      NORMAL: "success",
      OVERWEIGHT: "error",
      UNDERWEIGHT: "warning",
    };
    return {
      key: row.categoryId ?? "uncategorized",
      categoryId: row.categoryId,
      name: row.categoryName,
      icon: row.categoryIcon,
      color: row.categoryColor,
      editable: row.categoryId !== null,
      targetPercent: row.targetPercent,
      currentPercent: row.currentPercent,
      relativeDeviationPercent: row.relativeDeviationPercent,
      currentMarketValue: row.currentMarketValue,
      targetMarketValue: row.targetMarketValue,
      rebalanceAmount: row.rebalanceAmount,
      targetPercentLabel: formatPercent(row.targetPercent),
      currentPercentLabel: formatPercent(row.currentPercent),
      relativeDeviationLabel: row.relativeDeviationPercent === null
        ? "—"
        : formatPercent(row.relativeDeviationPercent),
      currentMarketValueLabel: formatNumber(row.currentMarketValue),
      targetMarketValueLabel: formatNumber(row.targetMarketValue),
      rebalanceAmountLabel: formatNumber(row.rebalanceAmount),
      status,
      statusLabel: statusLabels[status],
      statusColor: statusColors[status],
    };
  });
  const concentrationRows: PortfolioAlertConcentrationRow[] = (snapshot?.concentrations ?? [])
    .map((row) => ({
      key: `${row.market}:${row.normalizedSymbol}`,
      market: row.market,
      symbol: row.symbol,
      name: row.name,
      marketValue: row.marketValue,
      positionPercent: row.positionPercent,
      thresholdPercent: row.thresholdPercent,
      marketValueLabel: formatAmount(row.marketValue, currency),
      positionPercentLabel: formatPercent(row.positionPercent),
      thresholdPercentLabel: formatPercent(row.thresholdPercent),
      warning: `${row.market} ${row.symbol} ${row.name} 当前占比 ${formatPercent(row.positionPercent)}，超过 ${formatPercent(row.thresholdPercent)} 阈值`,
    }));
  const presentation = statusPresentation(view);
  const readyWithBreach = Boolean(
    config?.isActive
      && evaluation?.status === "READY"
      && evaluation.activeBreaches.length > 0,
  );
  const showReadyNormalSuccess = Boolean(
    config?.isActive
      && evaluation?.status === "READY"
      && !evaluation.stale
      && evaluation.activeBreaches.length === 0,
  );
  const aiDisabledReason = readyWithBreach
    ? null
    : !config
      ? "请先保存组合提醒配置"
      : !config.isActive
        ? "请先启用当前配置"
        : evaluation?.status !== "READY"
          ? "只有数据完整且配置有效时才能生成 AI 调仓建议"
          : "当前没有活动违规，无需生成调仓建议";
  const totalTargetPercent = rows
    .filter((row) => row.editable)
    .reduce((total, row) => total + row.targetPercent, 0);
  const totalCurrentPercent = rows.reduce(
    (total, row) => total + row.currentPercent,
    0,
  );

  return {
    configId: config?.id ?? null,
    currency,
    statusLabel: presentation.label,
    statusColor: presentation.color,
    banner: presentation.banner,
    stale: evaluation?.stale
      ?? Boolean(config !== null && !config.isActive && snapshot),
    snapshotEvaluatedAt: snapshot?.evaluatedAt ?? null,
    rows,
    concentrationRows,
    totalTargetPercent,
    totalCurrentPercent,
    totalTargetLabel: formatPercent(totalTargetPercent),
    totalCurrentLabel: formatPercent(totalCurrentPercent),
    missingDataDescriptions: evaluation?.missingData.map(missingDataDescription) ?? [],
    showReadyNormalSuccess,
    canAskAi: readyWithBreach,
    aiDisabledReason,
  };
}
