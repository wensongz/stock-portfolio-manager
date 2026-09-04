import type { AiToolContext, ToolCallInfo } from "../../types";

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
    (candidate.prefillActiveSkill !== "stock-review" &&
      candidate.prefillActiveSkill !== "munger-perspective")
  ) {
    return null;
  }
  return candidate.prefillActiveSkill;
}

export type AiPrefillToolContext = AiToolContext;

export const HOST_PREFILLED_TOOL_CALL_ID = "prefilled-stock-review";

const STOCK_REVIEW_TOOL_ARGUMENT_KEYS = new Set([
  "start_date",
  "end_date",
  "base_currency",
  "account_id",
  "market",
  "symbol",
]);

const PORTFOLIO_OVERVIEW_ARGUMENT_KEYS = new Set(["account_id", "market"]);

export function readAiPrefillToolContext(state: unknown): AiPrefillToolContext | null {
  if (!state || typeof state !== "object") return null;
  const candidate = state as Record<string, unknown>;
  if (
    !candidate.prefillToolArguments ||
    typeof candidate.prefillToolArguments !== "object" ||
    Array.isArray(candidate.prefillToolArguments)
  ) {
    return null;
  }
  const activeSkill = readAiPrefillActiveSkill(state);
  const args = candidate.prefillToolArguments as Record<string, unknown>;
  const keys = Object.keys(args);
  if (activeSkill === "stock-review" && candidate.prefillToolName === "get_stock_review") {
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
  if (
    activeSkill !== "munger-perspective" ||
    candidate.prefillToolName !== "get_portfolio_overview" ||
    keys.length > 1 ||
    keys.some((key) => !PORTFOLIO_OVERVIEW_ARGUMENT_KEYS.has(key)) ||
    keys.some((key) => typeof args[key] !== "string" || !(args[key] as string).trim()) ||
    ("market" in args && !["US", "CN", "HK"].includes(String(args.market)))
  ) {
    return null;
  }
  return {
    name: "get_portfolio_overview",
    arguments: Object.fromEntries(keys.map((key) => [key, String(args[key])] )),
  };
}

/** Atomically model taking the route-provided context for one outbound turn. */
export function consumeAiPrefillToolContext(
  pending: AiPrefillToolContext | null,
): { current: AiPrefillToolContext | null; next: null } {
  return { current: pending, next: null };
}

export function readPersistedAiToolContext(
  toolCalls: ToolCallInfo[],
): AiToolContext | null {
  const reserved = toolCalls.filter(
    (call) => call.id === HOST_PREFILLED_TOOL_CALL_ID,
  );
  if (reserved.length !== 1) return null;
  const [persisted] = reserved;
  if (
    persisted.origin !== "host_prefill" ||
    (persisted.name !== "get_stock_review" &&
      persisted.name !== "get_portfolio_overview") ||
    (persisted.status !== "success" && persisted.status !== "error") ||
    typeof persisted.arguments !== "string"
  ) {
    return null;
  }
  let args: unknown;
  try {
    args = JSON.parse(persisted.arguments);
  } catch {
    return null;
  }
  const isStockReview = persisted.name === "get_stock_review";
  return readAiPrefillToolContext({
    prefillPrompt: "persisted trusted review context",
    prefillActiveSkill: isStockReview ? "stock-review" : "munger-perspective",
    prefillAutoSend: false,
    prefillToolName: persisted.name,
    prefillToolArguments: args,
  });
}

export function resolveAiPrefillSessionId(
  prefillPrompt: string | null,
  currentSessionId: string | null,
): string | null {
  return prefillPrompt ? null : currentSessionId;
}
