import { invoke } from "@tauri-apps/api/core";
import type { ChatMessageRecord, ChatMessageWithMeta } from "../../types";

export function toRecords(
  sessionId: string,
  msgs: ChatMessageWithMeta[],
): ChatMessageRecord[] {
  return msgs.map((m) => ({
    id: m.id,
    session_id: sessionId,
    role: m.role,
    content: m.content,
    prompt_tokens: m.usage?.promptTokens ?? 0,
    completion_tokens: m.usage?.completionTokens ?? 0,
    total_tokens: m.usage?.totalTokens ?? 0,
    cached_tokens: m.usage?.cachedTokens ?? 0,
    // Persist reasoning (chain-of-thought) and tool-call details so they
    // survive a reload / session switch — assistant turns only. Tool calls are
    // serialised to a JSON string (the column is TEXT). Empty values are
    // omitted (undefined → NULL on the backend, skipped by serde).
    ...(m.reasoning && m.reasoning.trim().length > 0
      ? { reasoning: m.reasoning }
      : {}),
    ...(m.toolCalls && m.toolCalls.length > 0
      ? { tool_calls: JSON.stringify(m.toolCalls) }
      : {}),
    // Persist as RFC3339 so backend `ORDER BY created_at ASC` sorts correctly.
    created_at: new Date(m.createdAt).toISOString(),
  }));
}

/**
 * Persist a snapshot of messages for a session (delete + insert).
 *
 * IMPORTANT: always pass an explicit snapshot (`msgs`) captured at the call
 * site — never read `get().messages` here. The persistence `await` can straddle
 * a session switch or `resetForSessionSwitch` call; if we re-read state after
 * the await we'd persist an empty array and overwrite the data we intended to
 * save. The caller owns the snapshot.
 */
export async function persistMessages(sessionId: string, msgs: ChatMessageWithMeta[]) {
  if (msgs.length === 0) return;
  try {
    await invoke("save_chat_messages", {
      sessionId,
      messages: toRecords(sessionId, msgs),
    });
    await invoke("touch_chat_session", { id: sessionId });
  } catch (err) {
    // Persistence is best-effort: a failure shouldn't crash the chat UI.
    console.error(`[chatStore] failed to persist messages for ${sessionId}`, err);
  }
}
