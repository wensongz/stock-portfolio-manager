import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { ChatSession } from "../types";

// In-flight createSession promise. If `createSession` is called again while
// one is already pending (e.g. user double-clicks send, or React re-renders
// mid-flight), we return the same promise instead of firing a second backend
// insert — otherwise two sessions get created and the caller may hold a
// reference to the wrong id, causing "session not found" later.
let pendingCreate: Promise<ChatSession> | null = null;

function requestSessionCreation(): Promise<ChatSession> {
  if (pendingCreate) return pendingCreate;
  const request = invoke<ChatSession>("create_chat_session", { name: null });
  pendingCreate = request;
  const release = () => {
    if (pendingCreate === request) pendingCreate = null;
  };
  void request.then(release, release);
  return request;
}

interface ChatSessionState {
  sessions: ChatSession[];
  /** The currently-active session id, or null before the initial load. */
  currentSessionId: string | null;
  /** Monotonic counter for explicit/claimed session selections. */
  selectionRevision: number;
  loading: boolean;
  error: string | null;

  fetchSessions: () => Promise<void>;
  createSession: () => Promise<ChatSession>;
  /** Create and register a session without changing the user's selection. */
  createDetachedSession: () => Promise<ChatSession>;
  renameSession: (id: string, name: string) => Promise<void>;
  deleteSession: (id: string) => Promise<void>;
  /** Mark a session as the active one (does NOT load messages — the chat
   * store is responsible for loading messages on switch). */
  setCurrentSession: (id: string | null) => void;
  /** Select only if no selection action occurred since expectedRevision. */
  selectSessionIfRevision: (expectedRevision: number, id: string) => boolean;
  /** Bump a session's updated_at on the backend and reorder the local list so
   * the most-recently-used session floats to the top. */
  touchSession: (id: string) => Promise<void>;
  /**
   * If the session still has its default "新会话 …" name, ask the LLM to
   * generate a meaningful title from the user's first question. Falls back to
   * a truncated prefix of the question if AI is unavailable or fails. No-op
   * if the user has already renamed the session themselves.
   */
  autoRenameIfDefault: (sessionId: string, firstMessage: string) => Promise<void>;
}

export const useChatSessionStore = create<ChatSessionState>((set, get) => ({
  sessions: [],
  currentSessionId: null,
  selectionRevision: 0,
  loading: false,
  error: null,

  fetchSessions: async () => {
    set({ loading: true, error: null });
    try {
      const sessions = await invoke<ChatSession[]>("get_chat_sessions");
      set({ sessions, loading: false });
    } catch (err) {
      set({ error: String(err), loading: false });
    }
  },

  createSession: async () => {
    const expectedRevision = get().selectionRevision;
    // Reuse an in-flight creation so concurrent callers (double-click,
    // StrictMode re-render) get the same session instead of creating two.
    const session = await requestSessionCreation();
    set((state) => {
      const sessions = state.sessions.some((row) => row.id === session.id)
        ? state.sessions
        : [session, ...state.sessions];
      if (state.selectionRevision !== expectedRevision) return { sessions };
      return {
        sessions,
        currentSessionId: session.id,
        selectionRevision: state.selectionRevision + 1,
      };
    });
    return session;
  },

  createDetachedSession: async () => {
    const session = await requestSessionCreation();
    set((state) => ({
      sessions: state.sessions.some((row) => row.id === session.id)
        ? state.sessions
        : [session, ...state.sessions],
    }));
    return session;
  },

  renameSession: async (id, name) => {
    const updated = await invoke<ChatSession>("rename_chat_session", { id, name });
    set((state) => ({
      sessions: state.sessions.map((s) => (s.id === id ? updated : s)),
    }));
  },

  deleteSession: async (id) => {
    await invoke("delete_chat_session", { id });
    set((state) => {
      const remaining = state.sessions.filter((s) => s.id !== id);
      // If we deleted the active session, fall back to the most-recently-used
      // remaining one (the list is ordered by updated_at DESC, so index 0 is
      // the latest). If none remain, leave currentSessionId null — the UI
      // renders the welcome hero with a composer in that state, and a session
      // is created lazily on the user's first send.
      const nextCurrent =
        state.currentSessionId === id
          ? remaining.length > 0
            ? remaining[0].id
            : null
          : state.currentSessionId;
      return {
        sessions: remaining,
        currentSessionId: nextCurrent,
        ...(nextCurrent !== state.currentSessionId
          ? { selectionRevision: state.selectionRevision + 1 }
          : {}),
      };
    });
  },

  setCurrentSession: (id) => set((state) => ({
    currentSessionId: id,
    selectionRevision: state.selectionRevision + 1,
  })),

  selectSessionIfRevision: (expectedRevision, id) => {
    let claimed = false;
    set((state) => {
      if (
        state.selectionRevision !== expectedRevision ||
        !state.sessions.some((session) => session.id === id)
      ) {
        return state;
      }
      claimed = true;
      return {
        currentSessionId: id,
        selectionRevision: state.selectionRevision + 1,
      };
    });
    return claimed;
  },

  touchSession: async (id) => {
    await invoke("touch_chat_session", { id }).catch((e) => {
      // Non-critical: a touch failure shouldn't disrupt the chat. Just log it.
      console.error("[chatSessionStore] touch_chat_session failed", e);
    });
    set((state) => {
      const now = new Date().toISOString();
      return {
        sessions: state.sessions
          .map((s) => (s.id === id ? { ...s, updated_at: now } : s))
          // Re-sort so the touched session moves to the top (most recent first).
          .sort((a, b) => b.updated_at.localeCompare(a.updated_at)),
      };
    });
  },

  autoRenameIfDefault: async (sessionId, firstMessage) => {
    try {
      const session = get().sessions.find((s) => s.id === sessionId);
      if (!session) {
        console.warn("[chatSessionStore] autoRename: session not found", sessionId);
        return;
      }
      // Only auto-name sessions that still carry the default placeholder. Once
      // the user has renamed manually we never overwrite their choice.
      if (!session.name.startsWith("新聊天 ")) {
        console.log("[chatSessionStore] autoRename: already named, skip", session.name);
        return;
      }

      const message = firstMessage.trim();
      if (!message) return;

      // Truncation fallback used when AI generation fails or isn't configured.
      const fallback =
        message.length > 20 ? message.slice(0, 20) + "…" : message;

      try {
        const title = await invoke<string>("generate_session_title", {
          userMessage: message,
        });
        if (title && title.trim()) {
          console.log("[chatSessionStore] autoRename: AI generated", title.trim());
          await get().renameSession(sessionId, title.trim());
          return;
        }
      } catch (err) {
        // AI not configured / network error / parse error — fall through to
        // the truncation fallback so the session always gets a useful name.
        console.warn("[chatSessionStore] generate_session_title failed, using fallback", err);
      }
      console.log("[chatSessionStore] autoRename: using fallback", fallback);
      await get().renameSession(sessionId, fallback);
    } catch (err) {
      console.error("[chatSessionStore] autoRenameIfDefault failed", err);
    }
  },
}));
