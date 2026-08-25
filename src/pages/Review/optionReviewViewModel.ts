import type {
  OptionReviewDataQuality,
  OptionReviewReport,
  OptionUnderlyingReview,
} from "../../types";

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

export function getOptionReviewEmptyDescription(
  dataQuality: Pick<OptionReviewDataQuality, "unmatched_records" | "missing_trade_dates">,
) {
  return dataQuality.unmatched_records > 0 || dataQuality.missing_trade_dates > 0
    ? "当前暂无可分析的Campaign，请查看上方数据质量说明"
    : "该账户暂无可复盘的期权记录，请去期权管理导入CSV";
}
