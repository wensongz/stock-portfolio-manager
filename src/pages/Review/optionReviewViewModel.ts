import type { OptionReviewReport, OptionUnderlyingReview } from "../../types";

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
