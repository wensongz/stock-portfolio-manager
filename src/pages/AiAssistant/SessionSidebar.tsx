import { useState } from "react";
import { Button, Input, Popconfirm, Tooltip, message } from "antd";
import {
  CheckOutlined,
  DeleteOutlined,
  EditOutlined,
  LoadingOutlined,
  MenuFoldOutlined,
  MenuUnfoldOutlined,
  MessageOutlined,
  PlusOutlined,
} from "@ant-design/icons";
import type { ChatSession } from "../../types";
import {
  loadAiSidebarCollapsed,
  saveAiSidebarCollapsed,
} from "./sidebarPreference";
import { formatRelativeTime } from "./formatters";

export function SessionSidebar({
  sessions,
  currentSessionId,
  streamingSessionId,
  onSelect,
  onNew,
  onDelete,
  onRename,
}: {
  sessions: ChatSession[];
  currentSessionId: string | null;
  /** Session id whose AI turn is currently generating (foreground or
   * background), or null when idle. Highlighted so the user can tell which
   * session is still replying — especially after switching away. */
  streamingSessionId: string | null;
  onSelect: (id: string) => void;
  onNew: () => void;
  onDelete: (id: string) => Promise<void>;
  onRename: (id: string, name: string) => Promise<void>;
}) {
  // Collapsed by default — users expand it on demand to browse/manage
  // chats. The collapsed rail still shows per-chat icons (with the active
  // one highlighted) so switching is one click away.
  const [collapsed, setCollapsed] = useState(() =>
    loadAiSidebarCollapsed(localStorage),
  );

  const updateCollapsed = (nextCollapsed: boolean) => {
    saveAiSidebarCollapsed(localStorage, nextCollapsed);
    setCollapsed(nextCollapsed);
  };

  if (collapsed) {
    // Collapsed rail: a thin icon column. New-chat + toggle on top, chat
    // icons below (active one highlighted). The "new chat" action just
    // switches to the welcome screen — no session is created until send.
    return (
      <aside className="w-14 flex-shrink-0 border-r flex flex-col items-center py-2 gap-1" style={{ borderColor: "var(--color-border)", backgroundColor: "var(--color-bg-layout)" }}>
        <Tooltip title="新聊天" placement="right">
          <Button
            type="primary"
            shape="circle"
            icon={<PlusOutlined />}
            onClick={onNew}
          />
        </Tooltip>
        <Tooltip title="展开聊天列表" placement="right">
          <Button
            type="text"
            shape="circle"
            icon={<MenuUnfoldOutlined />}
            onClick={() => updateCollapsed(false)}
          />
        </Tooltip>
        <div className="w-full h-px my-1" style={{ backgroundColor: "var(--color-border)" }} />
        <div className="flex-1 overflow-y-auto w-full flex flex-col items-center gap-1 px-1">
          {sessions.map((s) => {
            const isActive = s.id === currentSessionId;
            const isStreaming = s.id === streamingSessionId;
            return (
              <Tooltip
                key={s.id}
                title={
                  isStreaming ? `${s.name}（正在生成…）` : s.name
                }
                placement="right"
              >
                <button
                  onClick={() => onSelect(s.id)}
                  className={`relative w-9 h-9 rounded-full flex items-center justify-center text-sm font-medium transition-colors flex-shrink-0`}
                  style={{
                    backgroundColor: isActive ? "var(--color-info)" : "var(--color-border)",
                    color: isActive ? "white" : "var(--color-text-secondary)",
                  }}
                >
                  {sessionInitial(s.name)}
                  {isStreaming && (
                    <span
                      className="absolute inset-0 rounded-full border-2 border-purple-400 animate-ping"
                      style={{ animationDuration: "1.5s" }}
                    />
                  )}
                </button>
              </Tooltip>
            );
          })}
        </div>
      </aside>
    );
  }

  return (
    <aside className="w-60 flex-shrink-0 border-r flex flex-col" style={{ borderColor: "var(--color-border)", backgroundColor: "var(--color-bg-layout)" }}>
      <div className="p-2 border-b flex items-center gap-2" style={{ borderColor: "var(--color-border)" }}>
        <Button
          type="primary"
          icon={<PlusOutlined />}
          onClick={onNew}
          style={{ flex: 1 }}
        >
          新聊天
        </Button>
        <Tooltip title="收起聊天列表">
          <Button
            type="text"
            icon={<MenuFoldOutlined />}
            onClick={() => updateCollapsed(true)}
          />
        </Tooltip>
      </div>
      <div className="flex-1 overflow-y-auto py-2">
        {sessions.length === 0 ? (
          <div className="px-4 py-6 text-center text-xs" style={{ color: "var(--color-text-tertiary)" }}>
            暂无聊天记录
          </div>
        ) : (
          sessions.map((s) => (
            <SessionItem
              key={s.id}
              session={s}
              active={s.id === currentSessionId}
              streaming={s.id === streamingSessionId}
              onSelect={() => onSelect(s.id)}
              onDelete={() => onDelete(s.id)}
              onRename={(name) => onRename(s.id, name)}
            />
          ))
        )}
      </div>
    </aside>
  );
}
/** Pick a 1-2 char label for a collapsed-rail avatar. Prefers the first
 * meaningful (non-ASCII-prefix) character of the name. */
function sessionInitial(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return "?";
  // For "新聊天 ..." defaults, show the time digits instead of "新".
  const match = trimmed.match(/^新聊天\s+(\d{2}):(\d{2})$/);
  if (match) return match[1];
  return Array.from(trimmed)[0];
}

function SessionItem({
  session,
  active,
  streaming,
  onSelect,
  onDelete,
  onRename,
}: {
  session: { id: string; name: string; updated_at: string };
  active: boolean;
  streaming: boolean;
  onSelect: () => void;
  onDelete: () => Promise<void>;
  onRename: (name: string) => Promise<void>;
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(session.name);

  const submitRename = async () => {
    const trimmed = draft.trim();
    if (trimmed && trimmed !== session.name) {
      await onRename(trimmed);
    }
    setEditing(false);
  };

  if (editing) {
    return (
      <div className="px-2 py-1.5">
        <Input
          size="small"
          autoFocus
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onPressEnter={submitRename}
          onKeyDown={(e) => {
            if (e.key === "Escape") {
              setDraft(session.name);
              setEditing(false);
            }
          }}
          suffix={
            <CheckOutlined
              onClick={submitRename}
              style={{ color: "var(--color-success)", cursor: "pointer" }}
            />
          }
        />
      </div>
    );
  }

  return (
    <div
      className="group flex items-center gap-2 px-2 mx-2 my-0.5 py-2 rounded cursor-pointer transition-colors"
      style={{
        backgroundColor: active ? "color-mix(in srgb, var(--color-info) 15%, transparent)" : "transparent",
        color: "var(--color-text)",
      }}
      onClick={onSelect}
      onMouseEnter={(e) => {
        if (!active) e.currentTarget.style.backgroundColor = "color-mix(in srgb, var(--color-text) 8%, transparent)";
      }}
      onMouseLeave={(e) => {
        if (!active) e.currentTarget.style.backgroundColor = "transparent";
      }}
    >
      {streaming ? (
        <LoadingOutlined
          style={{ fontSize: 14, flexShrink: 0, color: "var(--color-info)" }}
        />
      ) : (
        <MessageOutlined style={{ fontSize: 14, flexShrink: 0 }} />
      )}
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-1.5 min-w-0">
          <div
            className="truncate text-sm"
            title={session.name}
          >
            {session.name}
          </div>
          {streaming && (
            <span
              className="flex-shrink-0 text-xs"
              style={{ fontSize: 11, color: "var(--color-info)" }}
            >
              生成中…
            </span>
          )}
        </div>
        <div className="text-xs" style={{ color: "var(--color-text-tertiary)" }}>
          {streaming ? "AI 正在回复" : formatRelativeTime(session.updated_at)}
        </div>
      </div>
      <div className="flex-shrink-0 opacity-0 group-hover:opacity-100 flex items-center">
        <Button
          type="text"
          size="small"
          icon={<EditOutlined />}
          onClick={(e) => {
            e.stopPropagation();
            setDraft(session.name);
            setEditing(true);
          }}
          style={{ padding: "0 4px" }}
        />
        <Popconfirm
          title="删除该会话？"
          description="会话中的所有对话将一并删除。"
          okText="删除"
          cancelText="取消"
          okButtonProps={{ danger: true }}
          // IMPORTANT: do NOT return the promise. Antd v6's Popconfirm enters a
          // "confirm-button loading" state while awaiting a returned promise,
          // and because deleting the active session re-mounts the chat panel
          // (changing currentSessionId), the Popconfirm can be unmounted
          // mid-flight leaving the button seemingly stuck. Fire-and-forget
          // lets the popover close immediately; the store handles the rest.
          onConfirm={(e) => {
            e?.stopPropagation();
            void onDelete().catch((err) => {
              console.error("[SessionItem] delete failed", err);
              message.error("删除会话失败：" + String(err));
            });
          }}
          onCancel={(e) => e?.stopPropagation()}
        >
          <Button
            type="text"
            size="small"
            danger
            icon={<DeleteOutlined />}
            onClick={(e) => e.stopPropagation()}
            style={{ padding: "0 4px" }}
          />
        </Popconfirm>
      </div>
    </div>
  );
}
