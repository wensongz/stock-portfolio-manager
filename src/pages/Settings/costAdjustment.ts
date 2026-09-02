import type { QuoteProviderConfig } from "../../types";

export type CostAdjustmentKey =
  | "cn_adjust_sell_pay_cost"
  | "us_adjust_sell_pay_cost"
  | "hk_adjust_sell_pay_cost";

type InvokeCommand = (command: string, args: { config: QuoteProviderConfig }) => Promise<unknown>;

export async function updateCostAdjustmentPolicy(
  invokeCommand: InvokeCommand,
  current: QuoteProviderConfig,
  key: CostAdjustmentKey,
  checked: boolean
): Promise<QuoteProviderConfig> {
  const updated = { ...current, [key]: checked };
  await invokeCommand("update_quote_provider_config", { config: updated });
  return updated;
}
