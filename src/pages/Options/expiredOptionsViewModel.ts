import type {
  ExpiredOptionStats,
  OptionContract,
  OptionReviewReport,
  OptionUnderlyingReview,
} from "../../types";

export interface ExpiredUnderlyingSummary {
  underlying: string;
  netPremium: number | null;
  totalRecords: number;
  assignedRecords: number;
  expiredRecords: number;
  putQuantity: number;
  callQuantity: number;
  assignmentRatio: number;
  averageNetPremiumPerRecord: number | null;
  latestCompletedAt: string | null;
}

export function selectAccountOptionContracts(
  contracts: OptionContract[],
  accountId: string,
) {
  return contracts.filter((contract) => contract.account_id === accountId);
}

export function isAllHistoryOptionReview(
  report: OptionReviewReport | null,
  accountId: string,
) {
  return report?.account_id === accountId && report.period_days == null;
}

export function isAllHistoryOptionReviewRequest(
  requestedAccountId: string | null,
  requestedPeriodDays: number | null,
  accountId: string,
) {
  return requestedAccountId === accountId && requestedPeriodDays == null;
}

export function getOptionContractRowKey(record: {
  key?: string;
  id?: string;
}) {
  return record.key ?? record.id ?? "";
}

export function resolveCurrentOptionAccount(
  operationAccountId: string,
  currentAccountId: string,
) {
  return operationAccountId === currentAccountId ? operationAccountId : null;
}

export function buildExpiredOptionStats(
  contracts: OptionContract[],
): ExpiredOptionStats {
  const total = contracts.length;
  const assigned = contracts.filter(
    (contract) => contract.status === "assigned",
  ).length;
  const expired = contracts.filter(
    (contract) => contract.status === "expired",
  ).length;

  return {
    total_contracts: total,
    assigned_contracts: assigned,
    expired_contracts: expired,
    assignment_ratio: total > 0 ? assigned / total : 0,
  };
}

function latestCompletedAt(review: OptionUnderlyingReview | undefined) {
  if (!review) return null;

  const dates = review.campaigns
    .filter((campaign) => campaign.status === "completed" && campaign.ended_at)
    .map((campaign) => campaign.ended_at as string)
    .sort((left, right) => right.localeCompare(left));

  return dates[0] ?? null;
}

export function buildExpiredUnderlyingSummaries(
  contracts: OptionContract[],
  reviews: OptionUnderlyingReview[],
): ExpiredUnderlyingSummary[] {
  const reviewsByUnderlying = new Map(
    reviews.map((review) => [review.underlying, review]),
  );
  const contractsByUnderlying = new Map<string, OptionContract[]>();

  for (const contract of contracts) {
    const items = contractsByUnderlying.get(contract.underlying) ?? [];
    items.push(contract);
    contractsByUnderlying.set(contract.underlying, items);
  }

  return Array.from(contractsByUnderlying, ([underlying, items]) => {
    const review = reviewsByUnderlying.get(underlying);
    const totalRecords = items.length;
    const assignedRecords = items.filter(
      (contract) => contract.status === "assigned",
    ).length;
    const expiredRecords = items.filter(
      (contract) => contract.status === "expired",
    ).length;
    const totalContractQuantity = items.reduce(
      (total, contract) => total + Math.abs(contract.contracts),
      0,
    );
    const quantityByOptionType = (optionType: OptionContract["option_type"]) =>
      items.reduce(
        (total, contract) =>
          contract.option_type === optionType
            ? total + contract.contracts
            : total,
        0,
      );
    const reviewedContracts =
      review?.campaigns
        .filter((campaign) => campaign.status === "completed")
        .reduce((total, campaign) => total + Math.abs(campaign.contracts), 0) ?? 0;
    const hasCompleteReview =
      review != null &&
      Math.abs(reviewedContracts - totalContractQuantity) < 1e-9;
    const netPremium = hasCompleteReview
      ? review.completed_net_premium_pnl
      : null;

    return {
      underlying,
      netPremium,
      totalRecords,
      assignedRecords,
      expiredRecords,
      putQuantity: quantityByOptionType("P"),
      callQuantity: quantityByOptionType("C"),
      assignmentRatio: totalRecords > 0 ? assignedRecords / totalRecords : 0,
      averageNetPremiumPerRecord:
        netPremium != null && totalRecords > 0
          ? netPremium / totalRecords
          : null,
      latestCompletedAt: hasCompleteReview ? latestCompletedAt(review) : null,
    };
  }).sort(
    (left, right) => {
      if (left.netPremium == null && right.netPremium != null) return 1;
      if (left.netPremium != null && right.netPremium == null) return -1;
      return (
        Math.abs(right.netPremium ?? 0) - Math.abs(left.netPremium ?? 0) ||
        left.underlying.localeCompare(right.underlying)
      );
    },
  );
}

export function resolveExpiredUnderlyingSelection(
  rows: Array<Pick<ExpiredUnderlyingSummary, "underlying">>,
  current: string | null,
  preserveCurrent = true,
) {
  if (
    preserveCurrent &&
    current &&
    rows.some((row) => row.underlying === current)
  ) {
    return current;
  }
  return rows[0]?.underlying ?? null;
}
