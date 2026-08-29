import type { AiToolContext } from "../../types";

export function readAiPrefill(state: unknown): string | null {
  if (!state || typeof state !== "object" || !("prefillPrompt" in state)) return null;
  const value = (state as { prefillPrompt?: unknown }).prefillPrompt;
  if (typeof value !== "string") return null;
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

export function readAiPrefillActiveSkill(state: unknown): string | null {
  if (!state || typeof state !== "object") return null;
  const candidate = state as {
    prefillPrompt?: unknown;
    prefillActiveSkill?: unknown;
    prefillAutoSend?: unknown;
  };
  if (
    readAiPrefill(state) == null ||
    candidate.prefillAutoSend !== false ||
    candidate.prefillActiveSkill !== "stock-review"
  ) {
    return null;
  }
  return "stock-review";
}

export type AiPrefillToolContext = AiToolContext;

const STOCK_REVIEW_TOOL_ARGUMENT_KEYS = new Set([
  "start_date",
  "end_date",
  "base_currency",
  "account_id",
  "market",
  "benchmark_symbol",
  "symbol",
  "campaign_id",
]);

export function readAiPrefillToolContext(state: unknown): AiPrefillToolContext | null {
  if (!state || typeof state !== "object") return null;
  const candidate = state as Record<string, unknown>;
  if (
    readAiPrefillActiveSkill(state) !== "stock-review" ||
    candidate.prefillToolName !== "get_stock_review" ||
    !candidate.prefillToolArguments ||
    typeof candidate.prefillToolArguments !== "object" ||
    Array.isArray(candidate.prefillToolArguments)
  ) {
    return null;
  }
  const args = candidate.prefillToolArguments as Record<string, unknown>;
  const keys = Object.keys(args);
  if (
    keys.some((key) => !STOCK_REVIEW_TOOL_ARGUMENT_KEYS.has(key)) ||
    keys.some((key) => typeof args[key] !== "string" || !(args[key] as string).trim()) ||
    typeof args.start_date !== "string" ||
    typeof args.end_date !== "string" ||
    !["USD", "CNY", "HKD"].includes(String(args.base_currency))
  ) {
    return null;
  }
  return {
    name: "get_stock_review",
    arguments: Object.fromEntries(keys.map((key) => [key, String(args[key])] )),
  };
}

/** Atomically model taking the route-provided context for one outbound turn. */
export function consumeAiPrefillToolContext(
  pending: AiPrefillToolContext | null,
): { current: AiPrefillToolContext | null; next: null } {
  return { current: pending, next: null };
}

export function resolveAiPrefillSessionId(
  prefillPrompt: string | null,
  currentSessionId: string | null,
): string | null {
  return prefillPrompt ? null : currentSessionId;
}
