export interface CategoryHoldingLike {
  symbol: string;
  category_id: string | null;
  shares: number;
}

export interface ActiveHoldingLike {
  symbol: string;
  shares: number;
}

export function filterActiveOverviewHoldings<T extends ActiveHoldingLike>(
  holdings: readonly T[],
): T[] {
  return holdings.filter((holding) => holding.shares > 0);
}

export function filterActiveStockHoldings<T extends ActiveHoldingLike>(
  holdings: readonly T[],
  isCashCategory = false,
): T[] {
  return holdings.filter(
    (holding) =>
      holding.shares > 0 &&
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
