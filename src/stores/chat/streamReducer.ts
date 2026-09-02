import type { ChatMessageWithMeta } from "../../types";

export function updateMessageById(
  messages: ChatMessageWithMeta[],
  messageId: string,
  update: (message: ChatMessageWithMeta) => ChatMessageWithMeta,
): ChatMessageWithMeta[] {
  return messages.map((message) =>
    message.id === messageId ? update(message) : message,
  );
}
export function finalizeStreamMessages(
  messages: ChatMessageWithMeta[],
): {
  visible: ChatMessageWithMeta[];
  persistable: ChatMessageWithMeta[];
} {
  const visible = messages.filter(
    (message) =>
      !(
        message.role === "assistant" &&
        !message.error &&
        message.content.trim().length === 0
      ),
  );
  const persistable = visible.filter(
    (message) => !(message.role === "assistant" && message.error),
  );
  return { visible, persistable };
}
