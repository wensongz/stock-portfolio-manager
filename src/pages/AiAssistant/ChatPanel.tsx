import { useEffect, useMemo, useRef, useState } from "react";
import type { NavigateFunction } from "react-router-dom";
import {
  Alert,
  Button,
  Popconfirm,
  Space,
  Switch,
  Tag,
  Tooltip,
  Typography,
  message,
} from "antd";
import {
  ClockCircleOutlined,
  DeleteOutlined,
  RobotOutlined,
  SettingOutlined,
  SyncOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";
import { useChatStore, selectSessionTotalTokens } from "../../stores/chatStore";
import { useChatSessionStore } from "../../stores/chatSessionStore";
import { useAiStore } from "../../stores/aiStore";
import { useSkillStore } from "../../stores/skillStore";
import type { Skill } from "../../types";
import { Composer } from "./Composer";
import { MessageRow } from "./MessageRow";
import type { AiPrefillRequest } from "./prefill";
import {
  cancelAiPrefillAutoSendOperation,
  createAiPrefillAutoSendOperation,
  decideAiPrefillAutoSendStart,
  decideAiSessionTransition,
  runAiPrefillAutoSend,
  stageNonAutoAiPrefill,
} from "./aiPrefillAutoSend";
import type { AiPrefillAutoSendOperation } from "./aiPrefillAutoSend";

const { Title, Text } = Typography;


// A large pool of starter prompts shown (6 at a time, randomly) beneath the
// composer in the empty state. Each prompt maps roughly to a tool/category so
// the suggestions showcase the assistant's range. `pickRandom` selects 6 per
// render and the "换一批" button reshuffles.
const SUGGESTION_POOL: string[] = [
  // 行情 / 大盘
  "今天大盘怎么样？主要指数和我的持仓表现如何？",
  "AAPL 现在多少钱？近期走势如何？",
  "帮我查一下腾讯（0700.HK）的实时行情",
  "茅台现在什么价位？最近一个月涨跌多少？",
  // 组合总览
  "我现在的总资产是多少？按市场怎么分布？",
  "分析一下我当前持仓的集中度和风险",
  "我的持仓里哪些占比过高？需要警惕吗？",
  "各账户、各类别的资产分布合理吗？",
  // 绩效 / 收益
  "近一年绩效表现如何？哪些标的贡献最大？",
  "我的收益主要来自哪些股票和市场？",
  "按月看，哪几个月赚了、哪几个月亏了？",
  "最大回撤是多少？发生在什么时候？多久恢复的？",
  "我的夏普比率和波动率说明什么？风险调整后收益好吗？",
  "持仓里哪只股票表现最好？哪只最差？",
  // 交易 / 分红
  "基于近期交易，评估我的操作决策质量",
  "最近一个月我做了哪些买卖？时机好不好？",
  "我收了多少分红？哪些标的贡献的分红最多？",
  // 期权 / 提醒
  "我还有哪些期权没到期？什么时候到期？",
  "我设的价格提醒触发了吗？",
  // 归因 / 深度
  "我的盈亏主要来自哪些股票和市场？帮我做个收益归因",
  "帮我深度诊断一下苹果（AAPL）的行情和走势",
  // 建议
  "给出个性化的投资建议和改进方向",
  "基于当前持仓，我应该如何优化配置？",
];

/// Pick `n` distinct random items from `pool`, seeded by `seed` so the caller
/// can reshuffle by bumping the seed. Deterministic for a given (pool, seed)
/// so React's render stays stable between re-renders unless the seed changes.
function pickRandom<T>(pool: readonly T[], n: number, seed: number): T[] {
  if (pool.length <= n) return [...pool];
  // Simple seeded PRNG (mulberry32) — we don't need crypto-grade randomness,
  // just a stable, reshufflable subset. The seed makes this pure w.r.t. props.
  let s = seed >>> 0;
  const rand = () => {
    s = (s + 0x6d2b79f5) >>> 0;
    let t = s;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
  const indices = pool.map((_, i) => i);
  // Fisher-Yates partial shuffle: move n random picks to the front.
  for (let i = 0; i < n; i++) {
    const j = i + Math.floor(rand() * (indices.length - i));
    [indices[i], indices[j]] = [indices[j], indices[i]];
  }
  return indices.slice(0, n).map((i) => pool[i]);
}

// Friendly Chinese labels for the tool names the backend reports via the
// `ai-chat-tool` event. Raw names like `get_market_overview` would look out


export function ChatPanel({
  sessionId,
  navigate,
  initialRequest,
}: {
  // null means "no active session" (welcome screen). A session is created
  // lazily on the first send.
  sessionId: string | null;
  navigate: NavigateFunction;
  initialRequest: AiPrefillRequest | null;
}) {
  const {
    messages,
    sending,
    error,
    contextEnabled,
    streamingInBackground,
    streamingSessionIdState: streamingSessionId,
    sendMessage,
    editAndResend,
    retryLastTurn,
    regenerateMessage,
    dismissError,
    stopGeneration,
    clearMessages,
    setContextEnabled,
    loadSessionMessages,
    resetForSessionSwitch,
    setActiveSkillsForNextTurn,
    setToolContextForNextTurn,
    stageAiPrefillForNextTurn,
    ownsAiPrefillStaging,
    clearAiPrefillStaging,
    stagingRevision,
  } = useChatStore();
  // Read the staged explicit selection so the Composer can render "待激活"
  // chips. Subscribing via the store keeps the chips reactive as the user
  // adds/removes skills via `/` or the × button.
  const pendingActiveSkillIds = useChatStore((s) => s.pendingActiveSkills);
  const { config, loading: configLoading } = useAiStore();
  const { skills } = useSkillStore();
  // Quick chips and `/` autocomplete only show enabled skills.
  const enabledSkills = useMemo(() => skills.filter((s) => s.enabled), [skills]);
  // Resolve staged ids to full skill objects for chip rendering. Unknown ids
  // (e.g. a staged skill was deleted) are silently filtered out.
  const stagedSkills = useMemo(() => {
    const byId = new Map(skills.map((s) => [s.id, s]));
    return pendingActiveSkillIds
      .map((id) => byId.get(id))
      .filter((s): s is Skill => !!s);
  }, [pendingActiveSkillIds, skills]);
  const touchSession = useChatSessionStore((s) => s.touchSession);
  const autoRenameIfDefault = useChatSessionStore((s) => s.autoRenameIfDefault);
  const createDetachedSession = useChatSessionStore(
    (s) => s.createDetachedSession,
  );
  const selectSessionIfRevision = useChatSessionStore(
    (s) => s.selectSessionIfRevision,
  );
  const selectionRevision = useChatSessionStore((s) => s.selectionRevision);
  // Used by the "background stream" banner's "回到该会话" button to jump
  // directly to the session that's currently generating in the background.
  const setCurrentSession = useChatSessionStore((s) => s.setCurrentSession);

  const [reservedAutoSendOperation] = useState<AiPrefillAutoSendOperation | null>(
    () => {
      if (!initialRequest?.autoSend) return null;
      const sessionState = useChatSessionStore.getState();
      return createAiPrefillAutoSendOperation(
        sessionState.selectionRevision,
        useChatStore.getState().stagingRevision,
        sessionState.currentSessionId,
      );
    },
  );
  const [input, setInput] = useState("");
  const [autoSendPending, setAutoSendPending] = useState(
    reservedAutoSendOperation !== null,
  );
  const initialPrefillConsumedRef = useRef(false);
  // Seed for the random suggestion picker. Bumping it reshuffles which 6 of
  // SUGGESTION_POOL are shown in the empty state ("换一批" button).
  const [suggestionSeed, setSuggestionSeed] = useState(0);
  const suggestions = useMemo(
    () => pickRandom(SUGGESTION_POOL, 6, suggestionSeed),
    [suggestionSeed],
  );
  // Quick skills: show 6 random enabled skills in the empty state rather than
  // all of them (now 10 built-in). Reshuffles together with the "换一批" button
  // via the same seed so one click refreshes both suggestions and skills.
  const quickSkills = useMemo(
    () => pickRandom(enabledSkills, 6, suggestionSeed + 1),
    [enabledSkills, suggestionSeed],
  );
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const loadedSessionRef = useRef<string | null>(null);
  // Exact IDs replace the old broad "expecting some session" flag. A manual
  // selection can therefore never be mistaken for a session created by this
  // panel, even while backend creation is deferred.
  const expectedCreatedSessionIdRef = useRef<string | null>(null);
  const autoSendOperationRef = useRef<AiPrefillAutoSendOperation | null>(
    reservedAutoSendOperation,
  );

  useEffect(() => {
    if (
      !initialRequest ||
      initialRequest.autoSend ||
      initialPrefillConsumedRef.current
    ) {
      return;
    }
    initialPrefillConsumedRef.current = true;
    const prompt = stageNonAutoAiPrefill(initialRequest, {
      stageSkill: (skill) => setActiveSkillsForNextTurn([skill]),
      stageTool: setToolContextForNextTurn,
    });
    setInput((current) =>
      current.trim().length > 0 ? current : (prompt ?? current),
    );
  }, [initialRequest, setActiveSkillsForNextTurn, setToolContextForNextTurn]);

  // Load messages whenever the active session changes.
  //
  // Three cases to be careful about:
  //  1. Switching history sessions (A → B): reload B from DB.
  //  2. Sending from the new-chat welcome screen: `ensureSession` creates a
  //     session and currentSessionId flips null → newId. That change must NOT
  //     trigger a reload — the store is about to hold the user + assistant
  //     placeholder for the in-flight turn, and a reload (DB still empty)
  //     would wipe them, making the conversation vanish.
  //  3. Switching sessions *while* a stream is running on the current one:
  //     we must abort the stream, persist what we already have, and then load
  //     the newly selected session. Doing nothing (as the old sendingRef flag
  //     did) would leave the right panel stuck on the old session's content.
  //
  // Crucially, `messages.length` is NOT in the dependency array — otherwise
  // every token streamed would retrigger this effect and reload over the
  // in-progress reply.
  useEffect(() => {
    const transition = decideAiSessionTransition({
      nextSessionId: sessionId,
      loadedSessionId: loadedSessionRef.current,
      expectedCreatedSessionId: expectedCreatedSessionIdRef.current,
      autoSendOperation: autoSendOperationRef.current,
    });
    if (transition === "CLEAR") {
      // Switching to "new chat" welcome screen: clear the loaded session
      // marker and wipe the in-memory messages so the welcome hero shows
      // instead of the previous conversation. Abort any in-flight stream.
      loadedSessionRef.current = null;
      expectedCreatedSessionIdRef.current = null;
      void resetForSessionSwitch();
      return;
    }
    if (transition === "UNCHANGED") return;
    if (!sessionId) return;
    if (transition === "CANCEL_AUTO_AND_LOAD") {
      const operation = autoSendOperationRef.current;
      if (operation) {
        initialPrefillConsumedRef.current = true;
        cancelAiPrefillAutoSendOperation(operation, {
          clearOwnedContext: clearAiPrefillStaging,
        });
        if (autoSendOperationRef.current === operation) {
          autoSendOperationRef.current = null;
          setAutoSendPending(false);
        }
      }
    }
    loadedSessionRef.current = sessionId;
    // Sending from welcome screen: skip reload, keep in-flight messages.
    if (transition === "KEEP_IN_FLIGHT") {
      if (expectedCreatedSessionIdRef.current === sessionId) {
        expectedCreatedSessionIdRef.current = null;
      }
      return;
    }
    (async () => {
      await resetForSessionSwitch();
      await loadSessionMessages(sessionId);
    })();
    // Intentionally exclude messages.length — see comment above.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [
    clearAiPrefillStaging,
    sessionId,
    resetForSessionSwitch,
    loadSessionMessages,
  ]);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const sessionTotal = useMemo(
    () => selectSessionTotalTokens(messages),
    [messages],
  );

  const providerIsOllama = config?.provider === "ollama";
  const configuredForAutoSend = Boolean(
    !configLoading &&
    config?.model &&
    (providerIsOllama || config.api_key),
  );
  const notConfigured = !configLoading && !configuredForAutoSend;

  useEffect(() => {
    if (!initialRequest?.autoSend) return;
    const operation = autoSendOperationRef.current;
    if (!operation) return;

    const startDecision = decideAiPrefillAutoSendStart({
      operation,
      configured: configuredForAutoSend,
      sending,
      selectionRevision,
      currentSessionId: sessionId,
      stagingRevision,
    });
    if (startDecision === "WAIT" || startDecision === "IGNORE") {
      return;
    }
    if (startDecision === "CANCEL") {
      initialPrefillConsumedRef.current = true;
      cancelAiPrefillAutoSendOperation(operation, {
        clearOwnedContext: clearAiPrefillStaging,
      });
      if (autoSendOperationRef.current === operation) {
        autoSendOperationRef.current = null;
        setAutoSendPending(false);
      }
      return;
    }

    // The reservation prevents earlier rerenders from duplicating work. Mark
    // the route request consumed immediately before starting orchestration.
    initialPrefillConsumedRef.current = true;
    void runAiPrefillAutoSend(initialRequest!, operation, {
      stageOwnedContext: stageAiPrefillForNextTurn,
      ownsStagedContext: ownsAiPrefillStaging,
      clearOwnedContext: clearAiPrefillStaging,
      createSession: async () => (await createDetachedSession()).id,
      getStagingRevision: () => useChatStore.getState().stagingRevision,
      getSelectionState: () => {
        const state = useChatSessionStore.getState();
        return {
          revision: state.selectionRevision,
          currentSessionId: state.currentSessionId,
        };
      },
      claimSession: (expectedRevision, createdSessionId) => {
        expectedCreatedSessionIdRef.current = createdSessionId;
        const claimed = selectSessionIfRevision(
          expectedRevision,
          createdSessionId,
        );
        if (!claimed) expectedCreatedSessionIdRef.current = null;
        return claimed;
      },
      sendMessage,
      touchSession,
      renameSession: autoRenameIfDefault,
    }).then(() => {
      if (autoSendOperationRef.current === operation) {
        autoSendOperationRef.current = null;
        setAutoSendPending(false);
      }
    }).catch((error) => {
      if (autoSendOperationRef.current === operation) {
        autoSendOperationRef.current = null;
        setAutoSendPending(false);
        message.error("自动发送再平衡请求失败：" + String(error));
      }
    });
  }, [
    autoRenameIfDefault,
    clearAiPrefillStaging,
    configuredForAutoSend,
    createDetachedSession,
    initialRequest,
    ownsAiPrefillStaging,
    selectionRevision,
    selectSessionIfRevision,
    sendMessage,
    sending,
    sessionId,
    stagingRevision,
    stageAiPrefillForNextTurn,
    touchSession,
  ]);

  // React StrictMode performs a synthetic cleanup/remount. Deferring cleanup
  // by one microtask lets that remount retain the same owned operation, while
  // a real unmount invalidates it before a late create result can dispatch.
  const mountGenerationRef = useRef(0);
  useEffect(() => {
    const generation = ++mountGenerationRef.current;
    return () => {
      queueMicrotask(() => {
        if (mountGenerationRef.current !== generation) return;
        const operation = autoSendOperationRef.current;
        if (operation) {
          cancelAiPrefillAutoSendOperation(operation, {
            clearOwnedContext: clearAiPrefillStaging,
          });
          autoSendOperationRef.current = null;
        }
      });
    };
  }, [clearAiPrefillStaging]);

  // Resolve the effective session id, creating one on the fly if the user is
  // composing in the no-session welcome state. Returns null if creation
  // failed (so the caller can bail out).
  const ensureSession = async (): Promise<string | null> => {
    if (sessionId) return sessionId;
    const expectedRevision = useChatSessionStore.getState().selectionRevision;
    try {
      const s = await createDetachedSession();
      if (!s.id.trim()) throw new Error("创建会话返回了无效的会话 ID");
      expectedCreatedSessionIdRef.current = s.id;
      if (!selectSessionIfRevision(expectedRevision, s.id)) {
        expectedCreatedSessionIdRef.current = null;
        return null;
      }
      return s.id;
    } catch (err) {
      expectedCreatedSessionIdRef.current = null;
      message.error("创建会话失败：" + String(err));
      return null;
    }
  };

  const blockWhileAutoSendPending = (): boolean => {
    if (!autoSendPending) return false;
    message.warning("正在创建再平衡建议会话，请稍候或先切换会话取消");
    return true;
  };

  const handleSend = async () => {
    if (!input.trim()) return;
    if (blockWhileAutoSendPending()) return;
    if (notConfigured) {
      message.warning("请先在「设置 → AI 配置」中完成配置");
      return;
    }
    // The backend is single-stream: only one AI turn can run at a time. If a
    // turn is in flight (foreground here, or backgrounded after switching),
    // refuse the send with an actionable hint instead of silently dropping it.
    if (sending) {
      message.warning(
        streamingInBackground
          ? "有一条 AI 回复正在后台生成，请等待完成后再发送"
          : "AI 正在回复中，请等待当前回复完成后再发送",
      );
      return;
    }
    const text = input;
    const wasEmpty = messages.length === 0;
    setInput("");
    const sid = await ensureSession();
    if (!sid) return;
    const result = await sendMessage(text, sid);
    if (!result.ok) return;
    await touchSession(sid);
    if (wasEmpty) {
      void autoRenameIfDefault(sid, text);
    }
  };

  const handleSuggestion = async (s: string) => {
    if (notConfigured) return;
    if (blockWhileAutoSendPending()) return;
    if (sending) {
      message.warning(
        streamingInBackground
          ? "有一条 AI 回复正在后台生成，请等待完成后再发送"
          : "AI 正在回复中，请等待当前回复完成后再发送",
      );
      return;
    }
    const wasEmpty = messages.length === 0;
    const sid = await ensureSession();
    if (!sid) return;
    const result = await sendMessage(s, sid);
    if (!result.ok) return;
    await touchSession(sid);
    if (wasEmpty) {
      void autoRenameIfDefault(sid, s);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
      e.preventDefault();
      handleSend();
    }
  };

  // Stage a skill for explicit activation on the next send. We APPEND to the
  // existing staged list (deduped) so the user can layer multiple skills via
  // repeated `/` picks; the staged chips above the composer let them review
  // and remove individual picks before sending.
  const handlePickSkill = (skill: Skill) => {
    if (blockWhileAutoSendPending()) return;
    const current = pendingActiveSkillIds.filter((id) => id !== skill.id);
    setActiveSkillsForNextTurn([...current, skill.id]);
    message.success(`已激活技能：${skill.name}（发送时生效）`);
  };

  // Remove a single staged skill via the × on its chip.
  const handleRemoveStagedSkill = (skillId: string) => {
    if (blockWhileAutoSendPending()) return;
    setActiveSkillsForNextTurn(pendingActiveSkillIds.filter((id) => id !== skillId));
  };

  // Quick chip on the welcome hero: stage the skill AND immediately send a
  // canned opening prompt so the user sees an actionable result. The prompt
  // is derived from the skill's trigger / description so it's relevant.
  const handleQuickSkill = async (skill: Skill) => {
    if (notConfigured) {
      message.warning("请先在「设置 → AI 配置」中完成配置");
      return;
    }
    if (blockWhileAutoSendPending()) return;
    if (sending) {
      message.warning("AI 正在回复中，请等待当前回复完成");
      return;
    }
    setActiveSkillsForNextTurn([skill.id]);
    const prompt = `请使用「${skill.name}」技能帮我分析当前的投资组合。`;
    const wasEmpty = messages.length === 0;
    const sid = await ensureSession();
    if (!sid) return;
    const result = await sendMessage(prompt, sid);
    if (!result.ok) return;
    await touchSession(sid);
    if (wasEmpty) {
      void autoRenameIfDefault(sid, prompt);
    }
  };

  const handleClear = async () => {
    if (!sessionId) return;
    await clearMessages(sessionId);
  };

  const hasMessages = messages.length > 0;
  const interactionBlocked = sending || autoSendPending;

  return (
    <div className="flex flex-col h-full">
      {/* Top bar — only shown once the conversation has started. In the empty
          state the title lives in the centered hero block instead. */}
      {hasMessages && (
        <div className="flex items-center justify-between flex-wrap gap-3 mb-3 px-6">
          <Title level={3} style={{ margin: 0 }}>
            <RobotOutlined /> AI 助手
          </Title>
          <Space>
            {sessionTotal > 0 && (
              <Tag icon={<ThunderboltOutlined />} color="purple">
                本会话 {sessionTotal.toLocaleString()} tokens
              </Tag>
            )}
            <Tooltip title="开启后，AI 会参考你的实时持仓与绩效回答">
              <Space size="small">
                <Switch
                  checked={contextEnabled}
                  onChange={setContextEnabled}
                  size="small"
                />
                <Text type="secondary" style={{ fontSize: 13 }}>
                  注入数据
                </Text>
              </Space>
            </Tooltip>
            <Popconfirm
              title="清空当前会话的所有消息？"
              description="该操作不可撤销，会话本身会保留。"
              okText="清空"
              cancelText="取消"
              okButtonProps={{ danger: true }}
              onConfirm={handleClear}
            >
              <Button
                size="small"
                icon={<DeleteOutlined />}
                disabled={interactionBlocked || messages.length === 0}
              >
                清空对话
              </Button>
            </Popconfirm>
          </Space>
        </div>
      )}

      {error && (
        <Alert
          type="error"
          showIcon
          title={error}
          closable
          className="mb-3"
        />
      )}

      {streamingInBackground && (
        <Alert
          type="info"
          showIcon
          icon={<ClockCircleOutlined />}
          title="另一会话的 AI 回复正在后台生成中，完成前暂时无法发送新消息"
          className="mb-3"
          style={{ paddingBlock: 6, paddingInline: 12 }}
          action={
            streamingSessionId ? (
              <Button
                size="small"
                type="primary"
                onClick={() => setCurrentSession(streamingSessionId)}
              >
                回到该会话
              </Button>
            ) : undefined
          }
        />
      )}

      {hasMessages ? (
        <>
          <div className="flex-1 overflow-y-auto rounded-lg border p-6" style={{ backgroundColor: "var(--color-bg-card)", borderColor: "var(--color-border)" }}>
            <div className="space-y-6">
              {messages.map((m, i) => (
                <MessageRow
                  key={m.id}
                  message={m}
                  // Only show the streaming indicator when the in-flight turn
                  // is actually on screen (foreground). When it's backgrounded
                  // in another session, the current view's last message is NOT
                  // being streamed into and must not show a pulsing cursor.
                  streaming={
                    sending && !streamingInBackground && i === messages.length - 1
                  }
                  // Editing resends through the single backend stream, so it's
                  // disabled whenever ANY stream is in flight (foreground or
                  // backgrounded). Allowing edit-while-backgrounding would let
                  // the user submit, only for editAndResend to silently no-op.
                  canEdit={!interactionBlocked}
                  onEdit={(text) => {
                    if (sessionId) editAndResend(m.id, text, sessionId);
                  }}
                  onRetry={
                    m.error && sessionId && !autoSendPending
                      ? () => void retryLastTurn(sessionId)
                      : undefined
                  }
                  onDismiss={m.error ? () => dismissError(m.id) : undefined}
                  // Regenerate is available on any completed (non-error)
                  // assistant answer that isn't the in-flight streaming row.
                  // Disabled entirely while a stream is running.
                  onRegenerate={
                    sessionId && !m.error && !interactionBlocked
                      ? () => void regenerateMessage(m.id, sessionId)
                      : undefined
                  }
                />
              ))}
              <div ref={messagesEndRef} />
            </div>
          </div>
          <div className="mt-3">
            <Composer
              input={input}
              setInput={setInput}
              handleKeyDown={handleKeyDown}
              handleSend={handleSend}
              stopGeneration={stopGeneration}
              sending={sending}
              pending={autoSendPending}
              notConfigured={!!notConfigured}
              skills={enabledSkills}
              onPickSkill={handlePickSkill}
              stagedSkills={stagedSkills}
              onRemoveStagedSkill={handleRemoveStagedSkill}
            />
          </div>
        </>
      ) : (
        <div className="flex-1 overflow-y-auto flex items-center justify-center">
          <div className="w-full max-w-3xl px-4">
            <div className="text-center mb-8">
              <div
                className="inline-flex items-center justify-center w-16 h-16 rounded-full text-white text-2xl mb-4"
                style={{ background: "linear-gradient(135deg, #7c3aed 0%, #4f46e5 100%)" }}
              >
                <RobotOutlined />
              </div>
              <Title level={2} style={{ marginBottom: 8 }}>
                今天能帮你分析什么？
              </Title>
              <Text type="secondary">
                {contextEnabled
                  ? "已开启组合数据注入，AI 会参考你的实时持仓与绩效"
                  : "组合数据注入已关闭"}
              </Text>
            </div>

            {notConfigured ? (
              <Alert
                type="warning"
                showIcon
                title="尚未完成 AI 配置"
                description={
                  <Space>
                    <span>需要先配置服务商、API Key 与模型后才能开始对话。</span>
                    <Button
                      size="small"
                      type="link"
                      icon={<SettingOutlined />}
                      onClick={() => navigate("/settings")}
                    >
                      去配置
                    </Button>
                  </Space>
                }
                className="mb-6"
              />
            ) : (
              <Composer
                input={input}
                setInput={setInput}
                handleKeyDown={handleKeyDown}
                handleSend={handleSend}
                stopGeneration={stopGeneration}
                sending={sending}
                pending={autoSendPending}
                notConfigured={!!notConfigured}
                size="large"
                skills={enabledSkills}
                onPickSkill={handlePickSkill}
                stagedSkills={stagedSkills}
                onRemoveStagedSkill={handleRemoveStagedSkill}
              />
            )}

            {quickSkills.length > 0 && (
              <div className="flex flex-wrap items-center gap-2 mt-4">
                <Text type="secondary" style={{ fontSize: 13 }}>
                  <ThunderboltOutlined /> 快捷技能：
                </Text>
                {quickSkills.map((s) => (
                  <Tooltip
                    key={s.id}
                    title={s.description || `使用「${s.name}」技能开始分析`}
                  >
                    <Tag
                      color="purple"
                      style={{
                        cursor: autoSendPending ? "not-allowed" : "pointer",
                        marginInlineEnd: 0,
                        opacity: autoSendPending ? 0.5 : 1,
                      }}
                      onClick={() => handleQuickSkill(s)}
                    >
                      {s.name}
                    </Tag>
                  </Tooltip>
                ))}
              </div>
            )}

            <div className="flex items-center justify-between mt-4">
              <Text type="secondary" style={{ fontSize: 13 }}>
                试试问我：
              </Text>
              <Button
                type="text"
                size="small"
                icon={<SyncOutlined />}
                onClick={() => setSuggestionSeed((s) => s + 1)}
                style={{ color: "var(--color-info)", fontSize: 12, padding: "0 4px" }}
              >
                换一批
              </Button>
            </div>
            <div className="grid grid-cols-1 md:grid-cols-2 gap-2 mt-2">
              {suggestions.map((s) => (
                <Button
                  key={s}
                  disabled={!!notConfigured || autoSendPending}
                  onClick={() => handleSuggestion(s)}
                  style={{ textAlign: "left", whiteSpace: "normal", height: "auto", padding: "10px 14px" }}
                >
                  {s}
                </Button>
              ))}
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
