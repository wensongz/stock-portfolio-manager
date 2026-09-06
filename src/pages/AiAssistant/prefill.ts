import type { AiToolContext, ToolCallInfo } from "../../types";

export interface AiPrefillRequest {
  prompt: string;
  activeSkill: string | null;
  autoSend: boolean;
  toolContext: AiToolContext | null;
}

export interface PersistedAiPrefillContext {
  activeSkill: string;
  toolContext: AiToolContext;
}

export type PersistedRebalanceSessionBinding =
  | { kind: "none" }
  | { kind: "invalid" }
  | { kind: "bound"; activeSkill: "portfolio-rebalance"; toolContext: AiToolContext };

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

export function readAiPrefillRequest(state: unknown): AiPrefillRequest | null {
  const prompt = readAiPrefill(state);
  if (!prompt || !state || typeof state !== "object") return null;
  const candidate = state as Record<string, unknown>;

  if (candidate.prefillAutoSend === true) {
    const args = candidate.prefillToolArguments;
    if (
      candidate.prefillActiveSkill !== "portfolio-rebalance" ||
      candidate.prefillToolName !== "get_rebalance_context" ||
      !args ||
      typeof args !== "object" ||
      Array.isArray(args)
    ) {
      return null;
    }
    const argumentRecord = args as Record<string, unknown>;
    const keys = Object.keys(argumentRecord);
    if (
      keys.length !== 1 ||
      keys[0] !== "config_id" ||
      typeof argumentRecord.config_id !== "string" ||
      !argumentRecord.config_id.trim()
    ) {
      return null;
    }
    return {
      prompt,
      activeSkill: "portfolio-rebalance",
      autoSend: true,
      toolContext: {
        name: "get_rebalance_context",
        arguments: { config_id: argumentRecord.config_id },
      },
    };
  }

  return {
    prompt,
    activeSkill: readAiPrefillActiveSkill(state),
    autoSend: false,
    toolContext: readAiPrefillToolContext(state),
  };
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
  return readPersistedAiPrefillContext(toolCalls)?.toolContext ?? null;
}

export function readPersistedAiPrefillContext(
  toolCalls: ToolCallInfo[],
): PersistedAiPrefillContext | null {
  const reserved = toolCalls.filter(
    (call) => call.id === HOST_PREFILLED_TOOL_CALL_ID,
  );
  if (reserved.length !== 1) return null;
  const [persisted] = reserved;
  if (
    persisted.origin !== "host_prefill" ||
    (persisted.name !== "get_stock_review" &&
      persisted.name !== "get_portfolio_overview" &&
      persisted.name !== "get_rebalance_context") ||
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
  const activeSkill = persisted.name === "get_stock_review"
    ? "stock-review"
    : persisted.name === "get_portfolio_overview"
      ? "munger-perspective"
      : "portfolio-rebalance";
  const request = readAiPrefillRequest({
    prefillPrompt: "persisted trusted review context",
    prefillActiveSkill: activeSkill,
    prefillAutoSend: persisted.name === "get_rebalance_context",
    prefillToolName: persisted.name,
    prefillToolArguments: args,
  });
  return request?.activeSkill && request.toolContext
    ? { activeSkill: request.activeSkill, toolContext: request.toolContext }
    : null;
}

/**
 * Recover the single trusted rebalance capability that belongs to a persisted
 * chat session. We only accept the host-reserved prefill card, require a
 * successful invocation, and reject conflicting or malformed records instead
 * of silently widening the next turn back to the full portfolio.
 */
export function readPersistedRebalanceSessionBinding(
  toolCallsByTurn: ToolCallInfo[][],
): PersistedRebalanceSessionBinding {
  const reservedCalls = toolCallsByTurn
    .flat()
    .filter((call) => call.id === HOST_PREFILLED_TOOL_CALL_ID);
  if (reservedCalls.length === 0) return { kind: "none" };

  let configId: string | null = null;
  let sawRebalance = false;
  let sawOrdinaryPrefill = false;
  for (const call of reservedCalls) {
    if (call.origin !== "host_prefill" || call.status !== "success") {
      return { kind: "invalid" };
    }
    const persisted = readPersistedAiPrefillContext([call]);
    if (!persisted) return { kind: "invalid" };
    if (persisted.activeSkill !== "portfolio-rebalance") {
      if (sawRebalance) return { kind: "invalid" };
      sawOrdinaryPrefill = true;
      continue;
    }
    if (sawOrdinaryPrefill) return { kind: "invalid" };
    const currentConfigId = persisted.toolContext.arguments.config_id;
    if (
      typeof currentConfigId !== "string" ||
      !currentConfigId.trim() ||
      (configId !== null && configId !== currentConfigId)
    ) return { kind: "invalid" };
    sawRebalance = true;
    configId = currentConfigId;
  }

  if (!sawRebalance) return { kind: "none" };

  return {
    kind: "bound",
    activeSkill: "portfolio-rebalance",
    toolContext: {
      name: "get_rebalance_context",
      arguments: { config_id: configId! },
    },
  };
}

export function consumeCapturedAiPrefillRequest(
  input: { request: AiPrefillRequest | null; consumed: boolean },
  dependencies: {
    setCurrentSession: (sessionId: string | null) => void;
    clearRouteState: () => void;
  },
): boolean {
  if (!input.request || input.consumed) return input.consumed;
  dependencies.setCurrentSession(null);
  dependencies.clearRouteState();
  return true;
}

export function resolveAiPrefillSessionId(
  prefillPrompt: string | null,
  currentSessionId: string | null,
): string | null {
  return prefillPrompt ? null : currentSessionId;
}
