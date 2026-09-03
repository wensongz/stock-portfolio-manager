import type { OptionContract, OptionUnderlyingReview } from "../../types";

export interface ActiveUnderlyingSummary {
  underlying: string;
  netPremium: number | null;
  totalRecords: number;
  putContracts: number;
  callContracts: number;
  averageNetPremiumPerRecord: number | null;
  nextExpiryDate: string | null;
  expiringWithin30Days: number;
}

const monthNumbers: Record<string, string> = {
  JAN: "01",
  FEB: "02",
  MAR: "03",
  APR: "04",
  MAY: "05",
  JUN: "06",
  JUL: "07",
  AUG: "08",
  SEP: "09",
  OCT: "10",
  NOV: "11",
  DEC: "12",
};

function normalizeExpiryDate(value: string): string | null {
  const trimmed = value.trim();
  const separated =
    /^(\d{4})-(\d{2})-(\d{2})$/.exec(trimmed) ??
    /^(\d{4})\/(\d{2})\/(\d{2})$/.exec(trimmed);
  const compact = /^(\d{2})([A-Za-z]{3})(\d{2})$/.exec(trimmed);
  const parts = separated
    ? {
        year: Number(separated[1]),
        month: Number(separated[2]),
        day: Number(separated[3]),
      }
    : compact && monthNumbers[compact[2].toUpperCase()]
      ? {
          year: 2000 + Number(compact[3]),
          month: Number(monthNumbers[compact[2].toUpperCase()]),
          day: Number(compact[1]),
        }
      : null;
  if (!parts) return null;

  const parsed = new Date(Date.UTC(parts.year, parts.month - 1, parts.day));
  if (
    parsed.getUTCFullYear() !== parts.year ||
    parsed.getUTCMonth() + 1 !== parts.month ||
    parsed.getUTCDate() !== parts.day
  ) {
    return null;
  }
  return [
    parts.year.toString().padStart(4, "0"),
    parts.month.toString().padStart(2, "0"),
    parts.day.toString().padStart(2, "0"),
  ].join("-");
}

function daysBetween(left: string, right: string) {
  const millisecondsPerDay = 24 * 60 * 60 * 1000;
  return (
    (Date.parse(`${right}T00:00:00Z`) - Date.parse(`${left}T00:00:00Z`)) /
    millisecondsPerDay
  );
}

export function buildActiveUnderlyingSummaries(
  contracts: OptionContract[],
  reviews: OptionUnderlyingReview[],
  today: string,
): ActiveUnderlyingSummary[] {
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
    const activeCampaigns =
      reviewsByUnderlying
        .get(underlying)
        ?.campaigns.filter((campaign) => campaign.status === "active") ?? [];
    const activeCampaignsById = new Map(
      activeCampaigns.map((campaign) => [campaign.id, campaign]),
    );
    const matchedCampaigns = items.map((contract) =>
      activeCampaignsById.get(
        `option-review:${contract.account_id}:${underlying}:${contract.id}`,
      ),
    );
    const hasCompleteReview =
      activeCampaigns.length === items.length &&
      matchedCampaigns.every(
        (campaign) => campaign?.net_premium_pnl != null,
      );
    const netPremium = hasCompleteReview
      ? matchedCampaigns.reduce(
          (total, campaign) => total + (campaign?.net_premium_pnl ?? 0),
          0,
        )
      : null;
    const expiryDates = items
      .map((contract) => normalizeExpiryDate(contract.expiry_date))
      .filter((date): date is string => date != null)
      .sort();
    const totalRecords = items.length;

    return {
      underlying,
      netPremium,
      totalRecords,
      putContracts: items.reduce(
        (total, contract) =>
          contract.option_type === "P" ? total + contract.contracts : total,
        0,
      ),
      callContracts: items.reduce(
        (total, contract) =>
          contract.option_type === "C" ? total + contract.contracts : total,
        0,
      ),
      averageNetPremiumPerRecord:
        netPremium != null && totalRecords > 0
          ? netPremium / totalRecords
          : null,
      nextExpiryDate: expiryDates[0] ?? null,
      expiringWithin30Days: expiryDates.filter((date) => {
        const days = daysBetween(today, date);
        return days >= 0 && days <= 30;
      }).length,
    };
  }).sort((left, right) => {
    if (left.netPremium == null && right.netPremium != null) return 1;
    if (left.netPremium != null && right.netPremium == null) return -1;
    return (
      (right.netPremium ?? 0) - (left.netPremium ?? 0) ||
      left.underlying.localeCompare(right.underlying)
    );
  });
}

export function resolveActiveUnderlyingSelection(
  rows: Array<Pick<ActiveUnderlyingSummary, "underlying">>,
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
