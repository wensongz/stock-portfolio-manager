import type { Holding } from "../../types";

export interface AlertHoldingOption {
  value: string;
  label: string;
}

export function buildAlertHoldingOptions(
  holdings: readonly Holding[],
): AlertHoldingOption[] {
  const seen = new Set<string>();
  const options: AlertHoldingOption[] = [];

  for (const holding of holdings) {
    const key = `${holding.market}:${holding.symbol.trim().toUpperCase()}`;
    if (seen.has(key)) continue;

    seen.add(key);
    options.push({
      value: holding.id,
      label: `${holding.symbol} ${holding.name} (${holding.market})`,
    });
  }

  return options;
}
