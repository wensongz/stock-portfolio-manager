import type { ChatMessageWithMeta } from "../../types";

const TOOL_LABELS: Record<string, string> = {
  get_market_overview: "大盘总览",
  get_stock_quote: "实时行情",
  get_price_history: "价格历史",
  search_stock: "代码查询",
  get_stock_fundamentals: "估值基本面",
  get_technical_indicators: "技术指标",
  get_financial_statements: "财务报表",
  get_portfolio_overview: "组合总览",
  get_holdings_detail: "持仓明细",
  get_dashboard_summary: "资产总览",
  get_transactions: "交易记录",
  get_performance_metrics: "绩效指标",
  get_return_attribution: "收益归因",
  get_monthly_returns: "月度收益",
  get_drawdown_analysis: "回撤分析",
  get_risk_metrics: "风险指标",
  get_holding_ranking: "持仓排名",
  get_dividend_income: "分红收入",
  check_price_alerts: "价格提醒",
  get_option_positions: "期权持仓",
  get_option_review: "期权操作复盘",
  get_stock_review: "股票操作复盘",
};

export function toolLabel(name: string): string {
  return TOOL_LABELS[name] ?? name;
}

export function statusPlaceholder(
  message: Pick<ChatMessageWithMeta, "toolCalls" | "reasoning">,
): string {
  const toolsRunning = message.toolCalls?.some((tool) => tool.status === "running");
  if (toolsRunning) return "正在查询数据…";
  if (message.reasoning && message.reasoning.length > 0) return "正在思考…";
  return "思考中…";
}

export function formatTime(epochMs: number): string {
  const date = new Date(epochMs);
  const hours = String(date.getHours()).padStart(2, "0");
  const minutes = String(date.getMinutes()).padStart(2, "0");
  return `${hours}:${minutes}`;
}

/** Render an RFC3339 timestamp as a short Chinese relative label. */
export function formatRelativeTime(
  iso: string,
  now: number = Date.now(),
): string {
  const timestamp = new Date(iso).getTime();
  if (Number.isNaN(timestamp)) return "";

  const seconds = Math.floor((now - timestamp) / 1_000);
  const minutes = Math.floor(seconds / 60);
  const hours = Math.floor(minutes / 60);
  const days = Math.floor(hours / 24);

  if (seconds < 60) return "刚刚";
  if (minutes < 60) return `${minutes} 分钟前`;
  if (hours < 24) return `${hours} 小时前`;
  if (days === 1) return "昨天";
  if (days < 7) return `${days} 天前`;

  const date = new Date(iso);
  const year = date.getFullYear();
  const month = String(date.getMonth() + 1).padStart(2, "0");
  const day = String(date.getDate()).padStart(2, "0");
  const hoursLabel = String(date.getHours()).padStart(2, "0");
  const minutesLabel = String(date.getMinutes()).padStart(2, "0");
  return `${year}-${month}-${day} ${hoursLabel}:${minutesLabel}`;
}
