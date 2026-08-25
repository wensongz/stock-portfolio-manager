import type {
  OptionReviewDataQuality,
  OptionReviewReport,
  OptionUnderlyingReview,
} from "../../types";

export const OPTION_REVIEW_ANNUALIZED_YIELD_LABEL = "年化收益率（担保名义资本口径）";
export const OPTION_REVIEW_NET_PREMIUM_LABEL = "累计净权利金（含进行中）";

interface OptionReviewPromptInput {
  accountId: string;
  accountName: string;
  symbol: string;
  periodDays: number | null;
}

export function buildOptionReviewPrompt({
  accountId,
  accountName,
  symbol,
  periodDays,
}: OptionReviewPromptInput) {
  const allHistory = periodDays == null;
  const toolArguments = allHistory
    ? { accountId, symbol, allHistory: true }
    : { accountId, symbol, periodDays, allHistory: false };
  const period = allHistory ? "全部历史" : `最近 ${periodDays} 天`;
  return `请复盘账户 ${accountName}（accountId: ${accountId}）在${period}的 ${symbol} 期权交易。请调用 get_option_review，工具参数为 ${JSON.stringify(toolArguments)}。分别说明做得好的、做得不好的和最值得改进的地方；请使用确定性期权复盘数据并说明样本限制。`;
}

export function sortUnderlyingReviews(items: OptionUnderlyingReview[]) {
  return [...items].sort(
    (a, b) =>
      Math.abs(b.net_premium_pnl) - Math.abs(a.net_premium_pnl) ||
      a.underlying.localeCompare(b.underlying),
  );
}

export function selectDefaultUnderlying(report: OptionReviewReport | null) {
  return report ? sortUnderlyingReviews(report.underlyings)[0]?.underlying ?? null : null;
}

export function formatReviewPercent(value: number | null) {
  return value == null ? "—" : `${(value * 100).toFixed(1)}%`;
}

export function shouldShowNetPremium(
  summary: Pick<OptionReviewReport["summary"], "completed_campaigns" | "active_campaigns">,
) {
  return summary.completed_campaigns + summary.active_campaigns > 0;
}

export function getOptionReviewEmptyDescription(
  dataQuality: Pick<OptionReviewDataQuality, "unmatched_records" | "missing_trade_dates">,
) {
  return dataQuality.unmatched_records > 0 || dataQuality.missing_trade_dates > 0
    ? "当前暂无可分析的Campaign，请查看上方数据质量说明"
    : "该账户暂无可复盘的期权记录，请去期权管理导入CSV";
}
