import { useEffect, useState } from "react";
import { Typography } from "antd";
import { useLocation, useNavigate } from "react-router-dom";
import { useChatStore } from "../../stores/chatStore";
import { useChatSessionStore } from "../../stores/chatSessionStore";
import { useAiStore } from "../../stores/aiStore";
import { useSkillStore } from "../../stores/skillStore";
import {
  readAiPrefill,
  readAiPrefillActiveSkill,
  readAiPrefillToolContext,
  resolveAiPrefillSessionId,
} from "./prefill";
import { ChatPanel } from "./ChatPanel";
import { SessionSidebar } from "./SessionSidebar";

const { Text } = Typography;

export default function AiAssistantPage() {
  const navigate = useNavigate();
  const location = useLocation();
  const {
    sessions,
    currentSessionId,
    fetchSessions,
    deleteSession,
    renameSession,
    setCurrentSession,
  } = useChatSessionStore();
  const [initialPrompt] = useState(() => readAiPrefill(location.state));
  const [initialActiveSkill] = useState(() =>
    readAiPrefillActiveSkill(location.state),
  );
  const [initialToolContext] = useState(() =>
    readAiPrefillToolContext(location.state),
  );
  const [initialSessionId] = useState(() =>
    resolveAiPrefillSessionId(initialPrompt, currentSessionId),
  );
  const { init } = useChatStore();
  const setActiveSkillsForNextTurn = useChatStore(
    (state) => state.setActiveSkillsForNextTurn,
  );
  const setToolContextForNextTurn = useChatStore(
    (state) => state.setToolContextForNextTurn,
  );
  const streamingSessionId = useChatStore(
    (state) => state.streamingSessionIdState,
  );
  const { fetchConfig } = useAiStore();
  const { fetchSkills } = useSkillStore();
  const [bootstrapped, setBootstrapped] = useState(false);

  useEffect(() => {
    if (!initialPrompt) return;
    setCurrentSession(initialSessionId);
    if (initialActiveSkill) {
      setActiveSkillsForNextTurn([initialActiveSkill]);
    }
    if (initialToolContext) {
      setToolContextForNextTurn(initialToolContext);
    }
    navigate(location.pathname, { replace: true, state: null });
  }, [
    initialPrompt,
    initialActiveSkill,
    initialSessionId,
    initialToolContext,
    location.pathname,
    navigate,
    setActiveSkillsForNextTurn,
    setToolContextForNextTurn,
    setCurrentSession,
  ]);

  useEffect(() => {
    init();
    fetchConfig();
    void fetchSkills();
    void fetchSessions().then(() => setBootstrapped(true));
  }, [init, fetchConfig, fetchSkills, fetchSessions]);

  return (
    <div
      className="flex"
      style={{ margin: "-24px", height: "calc(100% + 48px)" }}
    >
      <SessionSidebar
        sessions={sessions}
        currentSessionId={currentSessionId}
        streamingSessionId={streamingSessionId}
        onSelect={setCurrentSession}
        onNew={() => setCurrentSession(null)}
        onDelete={deleteSession}
        onRename={renameSession}
      />
      <div className="flex flex-col h-full flex-1 min-w-0">
        {bootstrapped ? (
          <ChatPanel
            sessionId={currentSessionId}
            navigate={navigate}
            initialPrompt={initialPrompt}
          />
        ) : (
          <div
            className="flex-1 flex items-center justify-center"
            style={{ color: "var(--color-text-tertiary)" }}
          >
            <Text type="secondary">加载中…</Text>
          </div>
        )}
      </div>
    </div>
  );
}
