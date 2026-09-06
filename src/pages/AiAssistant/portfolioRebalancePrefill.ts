export interface PortfolioRebalanceNavigation {
  path: "/ai-assistant";
  sessionId: null;
  state: {
    prefillPrompt: string;
    prefillActiveSkill: "portfolio-rebalance";
    prefillAutoSend: true;
    prefillToolName: "get_rebalance_context";
    prefillToolArguments: { config_id: string };
  };
}

const PORTFOLIO_REBALANCE_PROMPT =
  "请基于可信的当前组合再平衡上下文生成一份详细、可执行且不追加资金的调整方案：逐项说明需要增配或减配的类别、建议买卖的具体标的与约计金额，并汇总调整后的类别和标的配置；不要自动交易，并说明数据限制和执行风险。";

export function buildPortfolioRebalanceNavigation(
  configId: string,
): PortfolioRebalanceNavigation {
  if (!configId.trim()) throw new Error("再平衡配置 ID 不能为空");
  return {
    path: "/ai-assistant",
    sessionId: null,
    state: {
      prefillPrompt: PORTFOLIO_REBALANCE_PROMPT,
      prefillActiveSkill: "portfolio-rebalance",
      prefillAutoSend: true,
      prefillToolName: "get_rebalance_context",
      prefillToolArguments: { config_id: configId },
    },
  };
}

export function navigateToPortfolioRebalance(
  configId: string,
  dependencies: {
    setCurrentSession: (sessionId: string | null) => void;
    navigate: (path: string, options: { state: PortfolioRebalanceNavigation["state"] }) => void;
  },
): void {
  const action = buildPortfolioRebalanceNavigation(configId);
  dependencies.setCurrentSession(action.sessionId);
  dependencies.navigate(action.path, { state: action.state });
}
