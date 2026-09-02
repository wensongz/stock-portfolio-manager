import { useState } from "react";
import {
  Alert,
  Button,
  Input,
  Tag,
  Tooltip,
  Typography,
  message as antdMessage,
} from "antd";
import {
  CheckOutlined,
  ClockCircleOutlined,
  CloseOutlined,
  CopyOutlined,
  DatabaseOutlined,
  EditOutlined,
  RedoOutlined,
  RobotOutlined,
  ThunderboltOutlined,
  UserOutlined,
} from "@ant-design/icons";
import { ReasoningBlock } from "../../components/ai/ReasoningBlock";
import { ToolCallList } from "../../components/ai/ToolCallCard";
import { MarkdownRenderer } from "../../components/ai/MarkdownRenderer";
import type { ChatMessageWithMeta, ChatUsage } from "../../types";
import { formatTime, statusPlaceholder, toolLabel } from "./formatters";

const { Text } = Typography;
const { TextArea } = Input;

export function MessageRow({
  message,
  streaming,
  canEdit,
  onEdit,
  onRetry,
  onDismiss,
  onRegenerate,
}: {
  message: ChatMessageWithMeta;
  streaming: boolean;
  canEdit?: boolean;
  onEdit?: (newContent: string) => void;
  /** Retry the failed assistant turn this row represents. */
  onRetry?: () => void;
  /** Remove this failed assistant row from the list. */
  onDismiss?: () => void;
  /** Regenerate this completed assistant turn with a fresh completion. */
  onRegenerate?: () => void;
}) {
  const isUser = message.role === "user";
  const timeLabel = formatTime(message.createdAt);
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(message.content);
  const [copied, setCopied] = useState(false);

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(message.content);
      setCopied(true);
      antdMessage.success("已复制");
      setTimeout(() => setCopied(false), 1500);
    } catch {
      antdMessage.error("复制失败");
    }
  };

  const avatar = (
    <div
      className={`flex-shrink-0 w-8 h-8 rounded-full flex items-center justify-center text-white text-sm ${
        isUser ? "bg-blue-500" : "bg-gradient-to-br from-purple-500 to-indigo-600"
      }`}
    >
      {isUser ? <UserOutlined /> : <RobotOutlined />}
    </div>
  );

  if (isUser) {
    const startEdit = () => {
      setDraft(message.content);
      setEditing(true);
    };
    const cancelEdit = () => setEditing(false);
    const submitEdit = () => {
      const text = draft.trim();
      if (!text || !onEdit) return;
      setEditing(false);
      onEdit(text);
    };

    if (editing) {
      return (
        <div className="flex gap-3 justify-end">
          <div className="max-w-[75%] w-full">
            <div className="rounded-2xl rounded-tr-sm border p-2" style={{ backgroundColor: "var(--color-bg-card)", borderColor: "var(--color-info)" }}>
              <TextArea
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                autoSize={{ minRows: 1, maxRows: 8 }}
                autoFocus
                onPressEnter={(e) => {
                  if (e.ctrlKey || e.metaKey) {
                    e.preventDefault();
                    submitEdit();
                  }
                }}
              />
              <div className="flex justify-end gap-2 mt-2">
                <Button size="small" icon={<CloseOutlined />} onClick={cancelEdit}>
                  取消
                </Button>
                <Button
                  size="small"
                  type="primary"
                  icon={<CheckOutlined />}
                  disabled={!draft.trim() || draft.trim() === message.content}
                  onClick={submitEdit}
                >
                  保存并提交
                </Button>
              </div>
            </div>
          </div>
          {avatar}
        </div>
      );
    }

    return (
      <div className="group flex gap-3 justify-end">
        <div className="max-w-[75%]">
          <div className="rounded-2xl rounded-tr-sm text-white px-4 py-2" style={{ backgroundColor: "var(--color-info)" }}>
            <div className="whitespace-pre-wrap break-words">{message.content}</div>
          </div>
          <div className="flex items-center justify-end gap-2 mt-1 h-5">
            {canEdit && (
              <Button
                type="text"
                size="small"
                className="opacity-0 group-hover:opacity-100 transition-opacity"
                style={{ fontSize: 12, padding: "0 4px", color: "var(--color-text-tertiary)" }}
                icon={<EditOutlined />}
                onClick={startEdit}
              >
                编辑
              </Button>
            )}
            <MessageMeta time={timeLabel} align="right" inline />
          </div>
        </div>
        {avatar}
      </div>
    );
  }

  return (
    <div className="group flex gap-3">
      {avatar}
      <div className="flex-1 min-w-0 pt-0.5">
        {message.activatedSkills && message.activatedSkills.length > 0 && (
          <div className="flex flex-wrap gap-1 mb-1.5">
            {message.activatedSkills.map((name) => (
              <Tag
                key={name}
                icon={<ThunderboltOutlined />}
                color="purple"
                style={{ marginInlineEnd: 0, fontSize: 12 }}
              >
                已用技能：{name}
              </Tag>
            ))}
          </div>
        )}
        {/*
          Chain-of-thought (reasoning_content) from thinking models. Rendered
          above the tool calls so the flow reads: think → query → answer.
          In-memory only; collapsed after streaming finishes.
        */}
        {message.reasoning && message.reasoning.trim().length > 0 && (
          <ReasoningBlock reasoning={message.reasoning} streaming={streaming} />
        )}
        {/*
          Tool calls. Prefer the rich per-call cards (status, args, result)
          when present; fall back to the legacy name-only badges for messages
          loaded from older persisted sessions that only carry `usedTools`.
        */}
        {message.toolCalls && message.toolCalls.length > 0 ? (
          <ToolCallList tools={message.toolCalls} />
        ) : (
          message.usedTools &&
          message.usedTools.length > 0 && (
            <div className="flex flex-wrap gap-1 mb-1.5">
              {message.usedTools.map((name) => (
                <Tag
                  key={name}
                  icon={<DatabaseOutlined />}
                  color="blue"
                  style={{ marginInlineEnd: 0, fontSize: 12 }}
                >
                  已查询：{toolLabel(name)}
                </Tag>
              ))}
            </div>
          )
        )}
        {message.error ? (
          <ErrorCard
            error={message.error}
            time={timeLabel}
            onRetry={onRetry}
            onDismiss={onDismiss}
          />
        ) : message.content ? (
          <div className="ai-chat-md">
            <MarkdownRenderer content={message.content} />
            {streaming && (
              <span className="inline-block w-2 h-4 ml-0.5 bg-purple-500 animate-pulse align-middle" />
            )}
          </div>
        ) : (
          <Text type="secondary" className="ai-chat-md">
            {streaming ? statusPlaceholder(message) : ""}
          </Text>
        )}
        {!streaming && !message.error && (
          <div className="flex items-center gap-1 mt-1 h-5">
            <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
              <Tooltip title="复制">
                <Button
                  type="text"
                  size="small"
                  style={{ fontSize: 12, padding: "0 4px", color: "var(--color-text-tertiary)" }}
                  icon={copied ? <CheckOutlined /> : <CopyOutlined />}
                  onClick={handleCopy}
                />
              </Tooltip>
              {onRegenerate && (
                <Tooltip title="重新生成">
                  <Button
                    type="text"
                    size="small"
                    style={{ fontSize: 12, padding: "0 4px", color: "var(--color-text-tertiary)" }}
                    icon={<RedoOutlined />}
                    onClick={onRegenerate}
                  />
                </Tooltip>
              )}
            </div>
            <MessageMeta time={timeLabel} usage={message.usage} stopped={message.stopped} />
          </div>
        )}
      </div>
    </div>
  );
}
/**
 * Inline error card rendered in place of a failed assistant reply. Shows the
 * error message, a retry button (re-issues the same turn), and a dismiss
 * button (removes the placeholder so the user can move on or re-edit).
 */
function ErrorCard({
  error,
  time,
  onRetry,
  onDismiss,
}: {
  error: string;
  time: string;
  onRetry?: () => void;
  onDismiss?: () => void;
}) {
  return (
    <Alert
      type="error"
      showIcon
      className="rounded-2xl rounded-tl-sm"
      style={{ padding: "8px 12px" }}
      title={
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0 flex-1">
            <div className="font-medium text-sm" style={{ color: "var(--color-error)" }}>AI 回复失败</div>
            <div
              className="text-xs mt-0.5 break-words whitespace-pre-wrap"
              style={{ maxHeight: 120, overflow: "auto", color: "var(--color-text-secondary)" }}
            >
              {error}
            </div>
          </div>
          <div className="flex items-center gap-1 flex-shrink-0">
            {onRetry && (
              <Button
                size="small"
                type="primary"
                icon={<RedoOutlined />}
                onClick={onRetry}
              >
                重试
              </Button>
            )}
            {onDismiss && (
              <Button
                size="small"
                type="text"
                icon={<CloseOutlined />}
                onClick={onDismiss}
              />
            )}
          </div>
        </div>
      }
      description={
        <div className="text-xs mt-1" style={{ color: "var(--color-text-tertiary)" }}>
          <ClockCircleOutlined style={{ fontSize: 11, marginRight: 4 }} />
          {time}
        </div>
      }
    />
  );
}

function MessageMeta({
  time,
  usage,
  stopped,
  align = "left",
  inline = false,
}: {
  time: string;
  usage?: ChatUsage;
  stopped?: boolean;
  align?: "left" | "right";
  inline?: boolean;
}) {
  return (
    <div
      className={`flex items-center gap-2 flex-wrap text-xs ${
        inline ? "" : "mt-1.5 "
      }${align === "right" ? "justify-end" : ""}`}
      style={{ color: "var(--color-text-tertiary)" }}
    >
      <span className="inline-flex items-center gap-1">
        <ClockCircleOutlined style={{ fontSize: 11 }} />
        {time}
      </span>
      {stopped && (
        <Tag color="orange" style={{ margin: 0, fontSize: 11 }}>
          已停止
        </Tag>
      )}
      {usage && usage.totalTokens > 0 && (
        <span>
          输入{" "}
          <Text strong style={{ fontSize: 12 }}>
            {usage.promptTokens.toLocaleString()}
          </Text>
          {usage.cachedTokens && usage.cachedTokens > 0 ? (
            <Text type="success" style={{ fontSize: 11 }}>
              {" "}
              (缓存 {usage.cachedTokens.toLocaleString()})
            </Text>
          ) : null}
          {" · "}
          输出{" "}
          <Text strong style={{ fontSize: 12 }}>
            {usage.completionTokens.toLocaleString()}
          </Text>
          {" · "}
          共{" "}
          <Text strong style={{ fontSize: 12, color: "var(--color-info)" }}>
            {usage.totalTokens.toLocaleString()}
          </Text>{" "}
          tokens
        </span>
      )}
    </div>
  );
}
