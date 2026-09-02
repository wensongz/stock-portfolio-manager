import type {
  ChatMessage,
  ChatMessageWithMeta,
  ToolCallInfo,
} from "../../types";
import { HOST_PREFILLED_TOOL_CALL_ID } from "../../pages/AiAssistant/prefill.ts";

export const newId = () =>
  typeof crypto !== "undefined" && "randomUUID" in crypto
    ? crypto.randomUUID()
    : `msg_${Date.now()}_${Math.random().toString(36).slice(2)}`;

/** Like Array.prototype.findIndex but scanning from the end. ES2023 adds this
 * on Array.prototype, but we target older runtimes so define our own. */
export function findLastIndex<T>(arr: T[], predicate: (item: T, index: number) => boolean): number {
  for (let i = arr.length - 1; i >= 0; i--) {
    if (predicate(arr[i], i)) return i;
  }
  return -1;
}

/**
 * Treat every lifecycle event as model-origin unless the Rust host explicitly
 * marks it as its prefill execution. Model ids are defensively moved out of
 * the single reserved host id even though current backends already namespace
 * them before emitting the event.
 */
export function normalizeToolCallEvent(toolCall: ToolCallInfo): ToolCallInfo {
  const origin = toolCall.origin === "host_prefill" ? "host_prefill" : "model";
  const id =
    origin === "model" && toolCall.id === HOST_PREFILLED_TOOL_CALL_ID
      ? `model:${toolCall.id}`
      : toolCall.id;
  return { ...toolCall, id, origin };
}

/**
 * Normalise the in-memory message list into a clean conversation history to
 * send to the LLM. The display list (`messages` state) can contain rows that
 * are illegal to send to an OpenAI-compatible endpoint and would cause the
 * provider to return an empty reply (HTTP 200 with no delta, then `[DONE]`),
 * which the user sees as "AI replied with an empty bubble".
 *
 * Three problems this fixes:
 *
 *  1. Empty-content rows — e.g. an assistant placeholder left behind when a
 *     stream was interrupted, or a degenerate empty reply that slipped past
 *     the `done` filter. Sending `{role:"assistant", content:""}` makes many
 *     providers reject the request silently.
 *  2. Failed (`error`) rows — a failed turn has empty content plus an error
 *     marker; it must never be replayed to the model.
 *  3. Non-alternating roles — OpenAI requires strict user/assistant
 *     alternation. Consecutive same-role rows (e.g. two user messages back to
 *     back, or user → empty-assistant → user) break this and trigger empty
 *     replies on several providers.
 *
 * The function ONLY shapes the outgoing request — the live `messages` state
 * is untouched (the user still sees their failed cards / placeholders).
 */
export function buildHistory(messages: ChatMessageWithMeta[]): ChatMessage[] {
  // Step 1: keep only rows safe to send — non-empty content, no error marker,
  // and a role the chat endpoint understands.
  const cleaned = messages.filter(
    (m) =>
      (m.role === "user" || m.role === "assistant") &&
      !m.error &&
      m.content.trim().length > 0,
  );

  // Step 2: enforce strict role alternation by collapsing runs of the same
  // role. For consecutive user turns we merge (so no user input is lost); for
  // consecutive assistant turns we keep only the last (earlier ones are
  // treated as superseded/partial). system rows don't appear here (filtered
  // above) so we only deal with user/assistant.
  const collapsed: ChatMessage[] = [];
  for (const m of cleaned) {
    const last = collapsed[collapsed.length - 1];
    if (last && last.role === m.role) {
      if (m.role === "user") {
        // Preserve both inputs — join with a blank line for readability.
        last.content = `${last.content}\n\n${m.content}`;
      } else {
        // Assistant: the newer reply supersedes the older one.
        last.content = m.content;
      }
    } else {
      collapsed.push({ role: m.role, content: m.content });
    }
  }

  // Step 3: the conversation the model continues MUST end on a user turn
  // (we're asking it to reply). If a stray assistant row is left at the tail,
  // drop it — there's nothing to respond to.
  while (collapsed.length > 0 && collapsed[collapsed.length - 1].role !== "user") {
    collapsed.pop();
  }

  return collapsed;
}
