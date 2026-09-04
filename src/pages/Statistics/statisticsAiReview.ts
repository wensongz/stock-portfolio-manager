import type { Currency, Market } from "../../types";

export type StatisticsAiReviewScope =
  | { kind: "overview"; baseCurrency: Currency }
  | { kind: "market"; market: Market }
  | { kind: "account"; accountId: string; accountName: string };

const marketNames: Record<Market, string> = {
  US: "美股",
  CN: "A股",
  HK: "港股",
};

const reviewRequirements =
  "请从芒格视角检查持仓集中度、能力圈、护城河、估值纪律、认知偏误与永久损失风险；先指出最可能导致失败的地方，再给出有优先级的调仓建议、建议目标仓位和执行条件。不要只根据当前浮盈亏判断投资质量；事实不足时请明确说明，并按需查询最新数据。";

export function buildStatisticsAiReviewPrefill(
  scope: StatisticsAiReviewScope,
) {
  let target: string;
  let toolArguments: Record<string, string>;
  switch (scope.kind) {
    case "overview":
      target = `复盘整个投资组合，并以 ${scope.baseCurrency} 为基准货币比较仓位`;
      toolArguments = {};
      break;
    case "market":
      target = `仅复盘${marketNames[scope.market]}（${scope.market}）范围内的持仓，忽略其他市场`;
      toolArguments = { market: scope.market };
      break;
    case "account":
      target = `仅复盘账户「${scope.accountName}」（账户 ID：${scope.accountId}）内的持仓，忽略其他账户`;
      toolArguments = { account_id: scope.accountId };
      break;
  }

  return {
    activeSkill: "munger-perspective" as const,
    prompt: `${target}。${reviewRequirements}`,
    toolName: "get_portfolio_overview" as const,
    toolArguments,
    autoSend: false as const,
  };
}
