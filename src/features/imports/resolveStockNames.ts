export type InvokeFunction = <T>(command: string, args?: Record<string, unknown>) => Promise<T>;

interface NamedHolding {
  symbol: string;
  name: string;
}

/** Resolve each symbol once: local holdings first, remote provider second, symbol fallback last. */
export async function resolveStockNames(
  symbols: string[],
  invoke: InvokeFunction,
): Promise<Map<string, string>> {
  const uniqueSymbols = [...new Set(symbols.map((symbol) => symbol.trim().toUpperCase()).filter(Boolean))];
  const names = new Map<string, string>();

  try {
    const holdings = await invoke<NamedHolding[]>("get_holdings", { accountId: null });
    const wanted = new Set(uniqueSymbols);
    for (const holding of holdings) {
      const symbol = holding.symbol.toUpperCase();
      if (wanted.has(symbol) && holding.name) names.set(symbol, holding.name);
    }
  } catch {
    // Remote lookups below remain available when the local cache cannot be read.
  }

  const unresolved = uniqueSymbols.filter((symbol) => !names.has(symbol));
  await Promise.all(unresolved.map(async (symbol) => {
    try {
      const name = await invoke<string | null>("lookup_stock_name_by_symbol", { symbol });
      names.set(symbol, name || symbol);
    } catch {
      names.set(symbol, symbol);
    }
  }));

  return names;
}
