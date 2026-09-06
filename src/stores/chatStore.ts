import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { message as antdMessage } from "antd";
import type {
  ChatMessageRecord,
  ChatMessageWithMeta,
  ChatUsage,
  ToolCallInfo,
  AiToolContext,
} from "../types";
import {
  consumeAiPrefillToolContext,
  readPersistedAiPrefillContext,
} from "../pages/AiAssistant/prefill.ts";
import {
  buildHistory,
  findLastIndex,
  newId,
  normalizeToolCallEvent,
} from "./chat/protocol.ts";
import { persistMessages } from "./chat/persistence.ts";
import {
  finalizeStreamMessages,
  updateMessageById,
} from "./chat/streamReducer.ts";

interface ChatState {
  messages: ChatMessageWithMeta[];
  sending: boolean;
  error: string | null;
  /** Whether to inject the live portfolio snapshot as extra context. */
  contextEnabled: boolean;
  /**
   * True when a stream is running in the *background* — i.e. the user switched
   * away from the session that owns the in-flight turn, so its tokens are
   * being accumulated into `backgroundStream` instead of `messages`. The UI
   * uses this to avoid showing a streaming indicator on the current (different)
   * session's last message, and to reflect that the send button is tied to a
   * turn the user can't see.
   */
  streamingInBackground: boolean;
  /**
   * The session id that owns the currently in-flight stream (foreground or
   * background), or null when no stream is running. Exposed as reactive state
   * (mirrored from the module-level `streamingSessionId`) so the sidebar can
   * highlight which session is actively generating — both when the user is
   * watching it (foreground) and after they've switched away (background).
   */
  streamingSessionIdState: string | null;
  /**
   * The session id currently shown in the chat panel (null on the welcome
   * screen). Set by `loadSessionMessages` and cleared on switch-to-welcome.
   * Used by event listeners to decide whether a finishing background stream
   * should flush into the live `messages` state (user is viewing it) or just
   * persist silently.
   */
  viewSessionId: string | null;

  /**
   * Ensure the Tauri streaming-event listeners are registered exactly once
   * for the lifetime of the app. Safe to call from every component mount.
   */
  init: () => void;
  sendMessage: (content: string, sessionId: string) => Promise<void>;
  editAndResend: (
    messageId: string,
    newContent: string,
    sessionId: string,
  ) => Promise<void>;
  /**
   * Retry the most recent failed assistant turn in the current session:
   * clear its `error`, reset its content placeholder, and re-invoke
   * `chat_with_ai` with the same history. No-op if there is no failed
   * assistant message. Throws/rejects are surfaced via the same failure
   * path as `sendMessage`.
   */
  retryLastTurn: (sessionId: string) => Promise<void>;
  /**
   * Regenerate a specific assistant turn: drop that assistant message and
   * everything after it, then re-issue `chat_with_ai` using the user turn that
   * preceded it. Unlike `retryLastTurn` (which only retries *failed* turns),
   * this works on a completed assistant answer — the Claude/ZCode "regenerate"
   * affordance. Re-applies any explicit skill selection captured on the
   * original turn. No-op while another stream is running.
   */
  regenerateMessage: (messageId: string, sessionId: string) => Promise<void>;
  /**
   * Remove a failed assistant placeholder from the message list. Used by the
   * "忽略" button on an error card so the user can clean up the conversation
   * without retrying.
   */
  dismissError: (messageId: string) => void;
  stopGeneration: () => Promise<void>;
  /** Load persisted messages for a session into `messages`. */
  loadSessionMessages: (sessionId: string) => Promise<void>;
  /** Abort any in-flight stream and clear local state (used on switch). */
  resetForSessionSwitch: () => Promise<void>;
  clearMessages: (sessionId: string) => Promise<void>;
  setContextEnabled: (enabled: boolean) => void;
  /**
   * Stage explicit skill ids for the next outbound turn (consumed on send).
   * Pass an empty array to fall back to automatic trigger-based activation.
   */
  setActiveSkillsForNextTurn: (skillIds: string[]) => void;
  /** Stage an exact host-approved read-tool scope for the next outbound turn. */
  setToolContextForNextTurn: (context: AiToolContext | null) => void;
  /**
   * Skill ids currently staged for the next send (set via `/` / `@` / quick
   * chip). Read by the Composer to render "待激活" chips with a remove (×)
   * button. Cleared on send (sendMessage / editAndResend).
   */
  pendingActiveSkills: string[];
  pendingToolContext: AiToolContext | null;
}

// Module-scope guards so the streaming listeners are registered at most once.
// Without this, React.StrictMode's mount→unmount→mount cycle (and any
// re-navigation to the page) would stack duplicate listeners and every token
// would be appended N times, producing "作为作为作为…" style output.
let listenersBound = false;

// The id of the assistant message currently being streamed into. Set when
// `sendMessage` pushes the empty placeholder and consumed by the `delta`
// listener to know which message to append to.
let streamingId: string | null = null;

// The session id the current stream belongs to. Captured at send time so the
// `done` listener knows which session to persist into even if the user
// switched sessions mid-stream (the stream is aborted on switch, but this
// guards against the race where the final done event arrives after the switch).
let streamingSessionId: string | null = null;

// When the user switches away from a session that has an in-flight stream,
// we don't abort the stream — we park its state here and let it run to
// completion in the background. The `delta`/`usage` listeners accumulate
// tokens into `backgroundStream.messages` (not the React `messages` state,
// which now holds a *different* session's view). When the stream finishes
// (`done`) we persist from this buffer. If the user switches back to the
// background session before it finishes, `loadSessionMessages` flushes the
// buffer back into the live `messages` state so they can watch it stream.
//
// At most one background stream can exist (backend is single-stream), and it
// is mutually exclusive with foreground streaming: if `backgroundStream` is
// set, the live `messages` belong to a different session and `streamingId`
// points into `backgroundStream.messages`, not `state.messages`.
interface BackgroundStream {
  sessionId: string;
  messages: ChatMessageWithMeta[];
  streamingId: string;
}
let backgroundStream: BackgroundStream | null = null;

// Explicitly activated skill ids for the *next* outbound turn. Kept in the
// store as `pendingActiveSkills` so the UI can render "待激活" chips. Set by
// `setActiveSkillsForNextTurn`; consumed and cleared by `sendMessage` /
// `editAndResend` so the selection only applies to the very next send. When
// empty, the backend falls back to automatic trigger-based activation.

function failStreamingMessage(
  set: (partial: Partial<ChatState> | ((s: ChatState) => Partial<ChatState>)) => void,
  errorMsg: string,
): void {
  const failedId = streamingId;
  streamingId = null;
  streamingSessionId = null;
  set((s) => ({
    sending: false,
    streamingSessionIdState: null,
    messages: failedId
      ? s.messages.map((m) =>
          m.id === failedId ? { ...m, error: errorMsg } : m,
        )
      : s.messages,
  }));
  // Surface the failure as a toast so it's visible even when the error card
  // itself is scrolled out of view. Keep the message short — the full error
  // text lives on the message row.
  const short = errorMsg.length > 120 ? errorMsg.slice(0, 120) + "…" : errorMsg;
  antdMessage.error("AI 回复失败：" + short);
}

/**
 * Apply a streaming update (token delta or usage) to the message identified by
 * `streamingId`. Routes to the background buffer when a background stream is
 * active, otherwise to the live React `messages` state.
 *
 * `apply` returns the new message object for the matched row (or the original
 * if no match — though there should always be a match).
 */
function applyStreamUpdate(
  set: (partial: Partial<ChatState> | ((s: ChatState) => Partial<ChatState>)) => void,
  apply: (m: ChatMessageWithMeta) => ChatMessageWithMeta,
): void {
  if (!streamingId) return;
  // Background stream: accumulate into the buffer, not the live state. The
  // live `messages` belongs to a different session and must not be touched.
  if (backgroundStream) {
    backgroundStream.messages = updateMessageById(
      backgroundStream.messages,
      streamingId,
      apply,
    );
    return;
  }
  set((state) => ({
    messages: updateMessageById(state.messages, streamingId!, apply),
  }));
}

/**
 * Persist the final state of a finished turn and clean up the streaming
 * bookkeeping. Used by the `done` listener for both foreground and background
 * turns — `sourceMessages` is whichever list holds the turn (live state or
 * background buffer).
 *
 * Returns the filtered message list (empty/error turns removed) so the caller
 * can update the live `messages` state if the turn was in the foreground.
 */
function finalizeTurnPersist(
  sessionId: string,
  sourceMessages: ChatMessageWithMeta[],
): ChatMessageWithMeta[] {
  const finalized = finalizeStreamMessages(sourceMessages);
  void persistMessages(sessionId, finalized.persistable);
  return finalized.visible;
}

export const useChatStore = create<ChatState>((set, get) => ({
  messages: [],
  sending: false,
  error: null,
  contextEnabled: true,
  streamingInBackground: false,
  streamingSessionIdState: null,
  viewSessionId: null,
  pendingActiveSkills: [],
  pendingToolContext: null,

  init: () => {
    if (listenersBound) return;
    listenersBound = true;

    listen<string>("ai-chat-delta", (event) => {
      const token = event.payload;
      if (!token || !streamingId) return;
      applyStreamUpdate(set, (m) => ({ ...m, content: m.content + token }));
    }).catch((e) => {
      console.error("[chatStore] failed to bind ai-chat-delta listener", e);
      listenersBound = false;
    });

    listen<ChatUsage>("ai-chat-usage", (event) => {
      if (!streamingId) return;
      const usage = event.payload;
      if (!usage || !usage.totalTokens) return;
      applyStreamUpdate(set, (m) => ({ ...m, usage }));
    }).catch((e) => {
      console.error("[chatStore] failed to bind ai-chat-usage listener", e);
      listenersBound = false;
    });

    listen("ai-chat-done", () => {
      // Persist the final state of the just-finished turn. Two shapes:
      //  1. Foreground: the turn lives in `state.messages` — filter empty/error
      //     rows, update state, persist.
      //  2. Background: the turn was parked in `backgroundStream` because the
      //     user switched away. Persist from the buffer. If the user has since
      //     switched back to that session, the buffer was already flushed to
      //     `state.messages` (see loadSessionMessages) and backgroundStream
      //     cleared — so case 2 only fires when the user is still elsewhere.
      //
      // Race guard: `clearMessages` and `stopGeneration` clear
      // `streamingSessionId` *before* aborting the stream (and the abort is
      // what triggers this `done` event). When we see it already null, the
      // turn's persistence has been taken over by that other path.
      const sessionId = streamingSessionId;
      const bg = backgroundStream;
      streamingId = null;
      streamingSessionId = null;
      backgroundStream = null;
      set({
        sending: false,
        streamingInBackground: false,
        streamingSessionIdState: null,
      });
      if (!sessionId) return;

      if (bg && bg.sessionId === sessionId) {
        // Background turn finished while the user was viewing another session.
        // Persist from the buffer; do NOT touch state.messages (it belongs to
        // a different session). When the user next opens this session,
        // loadSessionMessages will read the now-complete reply from the DB.
        finalizeTurnPersist(sessionId, bg.messages);
        return;
      }

      // Foreground turn: state.messages holds the conversation.
      const snapshot = get().messages;
      if (snapshot.length === 0) return;
      const filtered = finalizeTurnPersist(sessionId, snapshot);
      if (filtered.length !== snapshot.length) {
        set({ messages: filtered });
      }
    }).catch((e) => {
      console.error("[chatStore] failed to bind ai-chat-done listener", e);
      listenersBound = false;
    });

    listen<string>("ai-chat-error", (event) => {
      // Attach the error to the streaming placeholder (rendered as a retryable
      // error card) instead of stomping the global `error` flag. The global
      // flag is reserved for non-message errors (e.g. failed session load).
      //
      // For a background stream: if the user has switched back to that session
      // (viewSessionId matches), flush the buffer into the live state so the
      // error card is visible. Otherwise discard the buffer — error state is
      // not persisted, and the user will see the turn simply not present when
      // they return (the user message was already saved at send time).
      const bg = backgroundStream;
      const errorMsg = event.payload || "未知错误";
      if (bg) {
        backgroundStream = null;
        const marked = bg.messages.map((m) =>
          m.id === streamingId ? { ...m, error: errorMsg } : m,
        );
        streamingId = null;
        streamingSessionId = null;
        set({
          sending: false,
          streamingInBackground: false,
          streamingSessionIdState: null,
        });
        if (bg.sessionId === get().viewSessionId) {
          set({ messages: marked });
        }
        const short = errorMsg.length > 120 ? errorMsg.slice(0, 120) + "…" : errorMsg;
        antdMessage.error("AI 回复失败：" + short);
        return;
      }
      failStreamingMessage(set, errorMsg);
    }).catch((e) => {
      console.error("[chatStore] failed to bind ai-chat-error listener", e);
      listenersBound = false;
    });

    // The backend tells us which skill names it activated for the in-flight
    // turn (explicit selection or auto-matched triggers). Stamp them onto the
    // streaming placeholder so the UI can render a "⚡ 已用技能" badge. Like
    // the other streaming events, this routes through `applyStreamUpdate` so
    // background streams are handled correctly too.
    listen<string[]>("ai-chat-skill", (event) => {
      const names = event.payload;
      if (!Array.isArray(names) || names.length === 0 || !streamingId) return;
      applyStreamUpdate(set, (m) => ({ ...m, activatedSkills: names }));
    }).catch((e) => {
      console.error("[chatStore] failed to bind ai-chat-skill listener", e);
    });

    // The backend tells us which data tools it just executed for the in-flight
    // turn (e.g. get_market_overview, get_stock_quote). Tool calls can happen
    // across multiple rounds within one turn, so we accumulate names (deduped,
    // order-preserving) rather than replacing them — the badge grows to reflect
    // every fetch that informed the final answer.
    listen<string[]>("ai-chat-tool", (event) => {
      const names = event.payload;
      if (!Array.isArray(names) || names.length === 0 || !streamingId) return;
      applyStreamUpdate(set, (m) => {
        const seen = new Set(m.usedTools ?? []);
        const merged = [...(m.usedTools ?? [])];
        for (const n of names) {
          if (!seen.has(n)) {
            seen.add(n);
            merged.push(n);
          }
        }
        return { ...m, usedTools: merged };
      });
    }).catch((e) => {
      console.error("[chatStore] failed to bind ai-chat-tool listener", e);
    });

    // Stream the model's chain-of-thought (reasoning_content) so the UI can
    // render a live, collapsible "思考过程" block. Behaves exactly like
    // ai-chat-delta: append to the in-flight message (foreground or background).
    listen<string>("ai-chat-reasoning", (event) => {
      const token = event.payload;
      if (!token || !streamingId) return;
      applyStreamUpdate(set, (m) => ({
        ...m,
        reasoning: (m.reasoning ?? "") + token,
      }));
    }).catch((e) => {
      console.error("[chatStore] failed to bind ai-chat-reasoning listener", e);
    });

    // Detailed per-tool-call lifecycle: the backend emits a ToolCallEvent with
    // status "running" before execution and "success"/"error" + result/error
    // + durationMs after. We upsert by id so a running card updates in place
    // when its result arrives. Replaces the coarse name-only badge.
    listen<ToolCallInfo>("ai-chat-tool-call", (event) => {
      const incoming = event.payload;
      if (!incoming || !incoming.id || !streamingId) return;
      const tc = normalizeToolCallEvent(incoming);
      applyStreamUpdate(set, (m) => {
        const list = m.toolCalls ? [...m.toolCalls] : [];
        const idx = list.findIndex(
          (c) => c.id === tc.id && c.origin === tc.origin,
        );
        if (idx >= 0) {
          list[idx] = { ...list[idx], ...tc };
        } else {
          list.push(tc);
        }
        return { ...m, toolCalls: list };
      });
    }).catch((e) => {
      console.error("[chatStore] failed to bind ai-chat-tool-call listener", e);
    });
  },

  loadSessionMessages: async (sessionId) => {
    // Record the view session first so listeners (e.g. a background `done`)
    // know whether the finishing turn is currently on screen.
    set({ viewSessionId: sessionId });
    const bg = backgroundStream;
    // Fast path: the user switched back to the session whose stream is still
    // running in the background. The buffer holds a newer view than the DB
    // (the assistant reply is mid-generation, not yet persisted), so use the
    // buffer directly and promote the stream back to the foreground: clear
    // backgroundStream so subsequent deltas route to the live state.
    if (bg && bg.sessionId === sessionId) {
      backgroundStream = null;
      set({
        messages: bg.messages,
        streamingInBackground: false,
        error: null,
      });
      return;
    }
    try {
      const records = await invoke<ChatMessageRecord[]>("get_chat_messages", {
        sessionId,
      });
      set({
        messages: records.map((r) => ({
          id: r.id,
          role: r.role,
          content: r.content,
          createdAt: new Date(r.created_at).getTime(),
          usage:
            r.total_tokens > 0
              ? {
                  promptTokens: r.prompt_tokens,
                  completionTokens: r.completion_tokens,
                  totalTokens: r.total_tokens,
                  cachedTokens: r.cached_tokens,
                }
              : undefined,
          // Restore persisted reasoning + tool calls so they're visible on
          // re-open. tool_calls is a JSON string in the DB → parse to array.
          ...(r.reasoning && r.reasoning.trim().length > 0
            ? { reasoning: r.reasoning }
            : {}),
          ...(r.tool_calls
            ? (() => {
                try {
                  const parsed = JSON.parse(r.tool_calls);
                  if (!Array.isArray(parsed) || parsed.length === 0) return {};
                  const persistedContext = readPersistedAiPrefillContext(parsed);
                  return {
                    toolCalls: parsed,
                    ...(persistedContext
                      ? {
                          explicitToolContext: persistedContext.toolContext,
                          explicitSkillIds: [persistedContext.activeSkill],
                        }
                      : {}),
                  };
                } catch (e) {
                  console.warn(
                    "[chatStore] failed to parse persisted tool_calls",
                    e,
                  );
                  return {};
                }
              })()
            : {}),
        })),
        error: null,
      });
    } catch (err) {
      set({ error: String(err) });
    }
  },

  resetForSessionSwitch: async () => {
    // When switching sessions with an in-flight stream, we DON'T abort the
    // stream — we park it into `backgroundStream` so it can finish in the
    // background. The `delta`/`usage` listeners will accumulate tokens into
    // the buffer; `done` will persist from there. The user can switch back
    // to watch it continue, or ignore it and find the complete reply in the
    // DB when they next open the session.
    //
    // We only park if the stream is currently in the FOREGROUND (live in
    // state.messages). If it's already backgrounded (backgroundStream set),
    // the live messages belong to a different session already and we just
    // clear the view — the background stream keeps running untouched.
    const wasSending = get().sending;
    const inForeground =
      wasSending &&
      streamingId !== null &&
      streamingSessionId !== null &&
      backgroundStream === null;

    if (inForeground) {
      // Snapshot includes the streaming placeholder (being filled). Move it
      // to the buffer; keep streamingId/streamingSessionId set so listeners
      // keep routing to the buffer (see applyStreamUpdate).
      backgroundStream = {
        sessionId: streamingSessionId!,
        messages: [...get().messages],
        streamingId: streamingId!,
      };
      // NOTE: we deliberately do NOT set sending=false. The stream is still
      // running globally; the UI keeps the send button disabled and shows
      // a "stop" affordance. It flips back to false when `done` fires.
      set({
        messages: [],
        streamingInBackground: true,
        error: null,
        viewSessionId: null,
      });
      return;
    }

    // Already backgrounded (or no stream): just clear the view. The caller
    // will load the new session's messages next.
    set({ messages: [], error: null, viewSessionId: null });
  },

  sendMessage: async (content, sessionId) => {
    const trimmed = content.trim();
    if (!trimmed || get().sending) return;

    const now = Date.now();
    const userMsg: ChatMessageWithMeta = {
      id: newId(),
      role: "user",
      content: trimmed,
      createdAt: now,
    };
    // Consume the one-shot explicit skill selection for this turn (set via
    // `/`, `@`, or a quick chip) BEFORE building the placeholder, so it can
    // be stamped onto the assistant row and later re-applied by retryLastTurn.
    const activeSkills = get().pendingActiveSkills;
    const consumedToolContext = consumeAiPrefillToolContext(get().pendingToolContext);
    const toolContext = consumedToolContext.current;
    set({ pendingActiveSkills: [], pendingToolContext: consumedToolContext.next });
    const assistantMsg: ChatMessageWithMeta = {
      id: newId(),
      role: "assistant",
      content: "",
      createdAt: now + 1,
      // Remember the explicit selection so a retry of this turn re-sends the
      // same skills instead of dropping them (see retryLastTurn).
      ...(activeSkills.length > 0 ? { explicitSkillIds: activeSkills } : {}),
      ...(toolContext ? { explicitToolContext: toolContext } : {}),
    };
    streamingId = assistantMsg.id;
    streamingSessionId = sessionId;

    // Capture the pre-turn history once, before any set(), so every consumer
    // below sees the same snapshot. Reading get().messages after the set()
    // would already include userMsg/assistantMsg and lead to duplicated ids
    // when building the persistence payload (UNIQUE constraint violation).
    const priorMessages = get().messages;
    const updatedMessages = [...priorMessages, userMsg, assistantMsg];
    // Build the outgoing request history through buildHistory so empty/error
    // rows and non-alternating roles are scrubbed — otherwise the provider can
    // return an empty reply (the "AI 回复了空气泡" symptom).
    const history = buildHistory([...priorMessages, userMsg]);

    set({
      messages: updatedMessages,
      sending: true,
      error: null,
      streamingSessionIdState: sessionId,
    });

    // Persist the user turn immediately (without the empty assistant
    // placeholder). This guarantees the user's input survives any failure
    // mode that prevents the `ai-chat-done` listener from firing: a mid-stream
    // page refresh, a component unmount that drops local state, an abrupt
    // process exit, or an unhandled rejection in the stream pipeline. The
    // final assistant reply (with usage) is still upserted by the `done`
    // listener when the stream finishes successfully.
    void persistMessages(sessionId, [...priorMessages, userMsg]);

    try {
      // An empty array lets the backend fall back to automatic trigger-based
      // activation.
      await invoke("chat_with_ai", {
        req: {
          messages: history,
          includeContext: get().contextEnabled,
          activeSkills,
          toolContext,
        },
      });
    } catch (err) {
      // invoke() rejects when the backend returns Err — mark the placeholder
      // as failed so the UI shows a retryable error card instead of an empty
      // bubble. (The streaming `ai-chat-error` path handles mid-stream errors.)
      failStreamingMessage(set, String(err));
    }
  },

  editAndResend: async (messageId, newContent, sessionId) => {
    const trimmed = newContent.trim();
    if (!trimmed || get().sending) return;

    const messages = get().messages;
    const targetIdx = messages.findIndex((m) => m.id === messageId);
    if (targetIdx === -1) return;
    const target = messages[targetIdx];
    if (target.role !== "user") return;

    const editedUser: ChatMessageWithMeta = { ...target, content: trimmed };
    // Consume any staged explicit selection here too (a `/`-pick made just
    // before editing should still apply to the edited turn).
    const activeSkills = get().pendingActiveSkills;
    const consumedToolContext = consumeAiPrefillToolContext(get().pendingToolContext);
    const toolContext = consumedToolContext.current;
    set({ pendingActiveSkills: [], pendingToolContext: consumedToolContext.next });
    const assistantMsg: ChatMessageWithMeta = {
      id: newId(),
      role: "assistant",
      content: "",
      createdAt: Date.now() + 1,
      ...(activeSkills.length > 0 ? { explicitSkillIds: activeSkills } : {}),
      ...(toolContext ? { explicitToolContext: toolContext } : {}),
    };
    streamingId = assistantMsg.id;
    streamingSessionId = sessionId;

    const truncated = [...messages.slice(0, targetIdx), editedUser];
    // Normalise history (drop empty/error rows, enforce alternation) before
    // sending — same rationale as sendMessage.
    const history = buildHistory(truncated);

    set({
      messages: [...truncated, assistantMsg],
      sending: true,
      error: null,
      streamingSessionIdState: sessionId,
    });

    // Persist the truncated history (with the edited user turn) immediately —
    // same rationale as `sendMessage`: protects the user's input against any
    // failure that prevents the `done` listener from persisting the final
    // state. The assistant reply is still upserted once the stream finishes.
    void persistMessages(sessionId, truncated);

    try {
      await invoke("chat_with_ai", {
        req: {
          messages: history,
          includeContext: get().contextEnabled,
          activeSkills,
          toolContext,
        },
      });
    } catch (err) {
      failStreamingMessage(set, String(err));
    }
  },

  retryLastTurn: async (sessionId) => {
    if (get().sending) return;
    // Find the most recent failed assistant placeholder.
    const messages = get().messages;
    const failedIdx = findLastIndex(
      messages,
      (m) => m.role === "assistant" && !!m.error,
    );
    if (failedIdx === -1) return;
    const failedMsg = messages[failedIdx];

    // History = everything before the failed placeholder (the user turn that
    // triggered it is the last element). Normalise via buildHistory so any
    // other stale empty/error rows don't cause the retry to fail the same
    // way the original turn did.
    const history = buildHistory(messages.slice(0, failedIdx));

    // Re-apply the explicit skill selection that was staged for the original
    // turn (captured on the placeholder as `explicitSkillIds`). Without this
    // a retry would silently drop a `/`-picked skill and fall back to auto
    // matching. When none was set, an empty array preserves the original
    // behaviour (auto-match from the latest user message).
    const activeSkills = failedMsg.explicitSkillIds ?? [];
    const toolContext = failedMsg.explicitToolContext ?? null;

    // Reset the placeholder: clear error, wipe any partial content, and mark
    // it as the active streaming target so delta/usage/done listeners fill it.
    streamingId = failedMsg.id;
    streamingSessionId = sessionId;
    set((s) => ({
      sending: true,
      error: null,
      streamingSessionIdState: sessionId,
      messages: s.messages.map((m) =>
        m.id === failedMsg.id
          ? { ...m, error: undefined, content: "", stopped: undefined }
          : m,
      ),
    }));

    try {
      await invoke("chat_with_ai", {
        req: {
          messages: history,
          includeContext: get().contextEnabled,
          activeSkills,
          toolContext,
        },
      });
    } catch (err) {
      failStreamingMessage(set, String(err));
    }
  },

  regenerateMessage: async (messageId, sessionId) => {
    if (get().sending) return;
    const messages = get().messages;
    const idx = messages.findIndex((m) => m.id === messageId);
    if (idx === -1) return;
    const target = messages[idx];
    if (target.role !== "assistant") return;

    // Keep everything before this assistant turn (the last element is the user
    // turn that prompted it). Drop the target assistant message and anything
    // after it so a fresh answer replaces the old one.
    const truncated = messages.slice(0, idx);
    const history = buildHistory(truncated);

    // Re-apply the explicit skill selection captured on the original turn so
    // regenerate preserves a `/`-picked skill. Fall back to auto-match otherwise.
    const activeSkills = target.explicitSkillIds ?? [];
    const toolContext = target.explicitToolContext ?? null;

    // New placeholder so React sees a distinct key (the old row is dropped).
    const assistantMsg: ChatMessageWithMeta = {
      id: newId(),
      role: "assistant",
      content: "",
      createdAt: Date.now() + 1,
      ...(activeSkills.length > 0 ? { explicitSkillIds: activeSkills } : {}),
      ...(toolContext ? { explicitToolContext: toolContext } : {}),
    };
    streamingId = assistantMsg.id;
    streamingSessionId = sessionId;

    set({
      messages: [...truncated, assistantMsg],
      sending: true,
      error: null,
      streamingSessionIdState: sessionId,
    });

    // Persist the truncated history immediately so the dropped assistant turns
    // are removed from storage even if this regeneration later fails.
    void persistMessages(sessionId, truncated);

    try {
      await invoke("chat_with_ai", {
        req: {
          messages: history,
          includeContext: get().contextEnabled,
          activeSkills,
          toolContext,
        },
      });
    } catch (err) {
      failStreamingMessage(set, String(err));
    }
  },

  dismissError: (messageId) => {
    set((s) => ({
      messages: s.messages.filter((m) => m.id !== messageId),
    }));
  },

  stopGeneration: async () => {
    if (!get().sending) return;
    const bg = backgroundStream;
    const stoppedId = streamingId;
    // Clear bookkeeping BEFORE aborting so the `done` event the abort
    // triggers won't re-persist (the stop is the user's explicit intent; we
    // persist the partial reply ourselves below).
    streamingId = null;
    streamingSessionId = null;
    backgroundStream = null;
    set({
      sending: false,
      streamingInBackground: false,
      streamingSessionIdState: null,
    });
    try {
      await invoke("stop_ai_chat");
    } catch (err) {
      console.error("[chatStore] stop_ai_chat failed", err);
    }
    // Mark the stopped message and persist the partial turn. If the stream
    // was in the background and the user wasn't viewing it, we still persist
    // (into the background session) so the partial reply is preserved; we
    // just don't touch the live messages (they belong to another session).
    if (!stoppedId) return;
    if (bg) {
      // Background stop: persist the buffer (with stopped marker) into its
      // own session; only update live state if the user is viewing it.
      const marked = bg.messages.map((m) =>
        m.id === stoppedId ? { ...m, stopped: true } : m,
      );
      const filtered = marked.filter(
        (m) => !(m.role === "assistant" && !m.error && m.content.trim().length === 0),
      );
      void persistMessages(bg.sessionId, filtered);
      if (bg.sessionId === get().viewSessionId) {
        set({ messages: marked });
      }
      return;
    }
    // Foreground stop: mark in live state and persist.
    const viewSession = get().viewSessionId;
    set((state) => ({
      messages: state.messages.map((m) =>
        m.id === stoppedId ? { ...m, stopped: true } : m,
      ),
    }));
    if (viewSession) {
      const snapshot = get().messages;
      const filtered = snapshot.filter(
        (m) => !(m.role === "assistant" && !m.error && m.content.trim().length === 0),
      );
      void persistMessages(viewSession, filtered);
    }
  },

  clearMessages: async (sessionId) => {
    // If a stream is in flight, abort it first so the `done` listener doesn't
    // re-persist the cleared messages back into the DB after we wipe them.
    const bg = backgroundStream;
    const wasSending = get().sending;
    if (wasSending) {
      // If the in-flight stream belongs to THIS session, dropping the table
      // would conflict with the stream's eventual persist — abort it. If it
      // belongs to a different (backgrounded) session, we still abort: the
      // user is tearing down conversations and shouldn't have a ghost stream
      // running. Either way, clear bookkeeping so the incoming `done` is inert.
      streamingId = null;
      streamingSessionId = null;
      backgroundStream = null;
      try {
        await invoke("stop_ai_chat");
      } catch (e) {
        console.error("[chatStore] stop_ai_chat during clear failed", e);
      }
    }
    try {
      await invoke("clear_chat_session", { sessionId });
    } catch (err) {
      console.error("[chatStore] clear_chat_session failed", err);
    }
    // Wipe live state only if the cleared session is the one in view. If the
    // cleared session was backgrounded (bg.sessionId === sessionId), the live
    // messages belong to a different session and must be preserved.
    const wipeView = bg?.sessionId !== sessionId;
    set({
      ...(wipeView ? { messages: [] } : {}),
      sending: false,
      streamingInBackground: false,
      streamingSessionIdState: null,
      error: null,
    });
  },

  setContextEnabled: (enabled) => set({ contextEnabled: enabled }),

  setActiveSkillsForNextTurn: (skillIds) => {
    set({ pendingActiveSkills: skillIds.slice() });
  },

  setToolContextForNextTurn: (context) => {
    set({
      pendingToolContext: context
        ? { name: context.name, arguments: { ...context.arguments } }
        : null,
    });
  },
}));

/** Sum the total tokens of every assistant message with usage info. */
export function selectSessionTotalTokens(messages: ChatMessageWithMeta[]): number {
  return messages.reduce((sum, m) => sum + (m.usage?.totalTokens ?? 0), 0);
}
