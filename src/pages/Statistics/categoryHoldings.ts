export interface CategoryHoldingLike {
  symbol: string;
  category_id: string | null;
  shares: number;
}

export interface ActiveHoldingLike {
  symbol: string;
  shares: number;
}

function isActiveHolding(holding: ActiveHoldingLike): boolean {
  // Negative cash is an overdraft and must reduce net assets.
  return holding.shares > 0 ||
    (holding.shares < 0 && holding.symbol.startsWith("$CASH-"));
}

export function filterActiveOverviewHoldings<T extends ActiveHoldingLike>(
  holdings: readonly T[],
): T[] {
  return holdings.filter(isActiveHolding);
}

export function filterActiveStockHoldings<T extends ActiveHoldingLike>(
  holdings: readonly T[],
  isCashCategory = false,
): T[] {
  return holdings.filter(
    (holding) =>
      isActiveHolding(holding) &&
      (isCashCategory || !holding.symbol.startsWith("$CASH-")),
  );
}

export function filterCategoryHoldings<T extends CategoryHoldingLike>(
  holdings: readonly T[],
  categoryId: string,
): T[] {
  return filterActiveStockHoldings(holdings).filter(
    (holding) => holding.category_id === categoryId,
  );
}
