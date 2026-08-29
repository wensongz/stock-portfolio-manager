import { useEffect, useMemo, useRef, useState } from "react";
import { Alert, Button, Input, Popconfirm, Popover, Select, Space, Switch, Tag, Tooltip, Typography, message, message as antdMessage } from "antd";
import {
  RobotOutlined,
  SendOutlined,
  DeleteOutlined,
  SettingOutlined,
  UserOutlined,
  StopOutlined,
  ClockCircleOutlined,
  ThunderboltOutlined,
  EditOutlined,
  CheckOutlined,
  CloseOutlined,
  PlusOutlined,
  MessageOutlined,
  MenuFoldOutlined,
  MenuUnfoldOutlined,
  RedoOutlined,
  LoadingOutlined,
  DatabaseOutlined,
  SyncOutlined,
  CopyOutlined,
} from "@ant-design/icons";
import { useLocation, useNavigate } from "react-router-dom";
import { useChatStore, selectSessionTotalTokens } from "../../stores/chatStore";
import { useChatSessionStore } from "../../stores/chatSessionStore";
import { useAiStore } from "../../stores/aiStore";
import { useSkillStore } from "../../stores/skillStore";
import { ReasoningBlock } from "../../components/ai/ReasoningBlock";
import { ToolCallList } from "../../components/ai/ToolCallCard";
import { MarkdownRenderer } from "../../components/ai/MarkdownRenderer";
import type { AiModelInfo, ChatMessageWithMeta, ChatSession, ChatUsage, Skill } from "../../types";
import {
  readAiPrefill,
  readAiPrefillActiveSkill,
  readAiPrefillToolContext,
  resolveAiPrefillSessionId,
} from "./prefill";
import {
  loadAiSidebarCollapsed,
  saveAiSidebarCollapsed,
} from "./sidebarPreference";

const { Title, Text } = Typography;
const { TextArea } = Input;

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
// of place in a badge, so we map them to what the user actually asked for.
const TOOL_LABELS: Record<string, string> = {
  get_market_overview: "大盘总览",
  get_stock_quote: "实时行情",
  get_price_history: "价格历史",
  search_stock: "代码查询",
  get_stock_fundamentals: "估值基本面",
  get_technical_indicators: "技术指标",
  get_financial_statements: "财务报表",
  get_portfolio_overview: "组合总览",
  get_holdings_detail: "持仓明细",
  get_dashboard_summary: "资产总览",
  get_transactions: "交易记录",
  get_performance_metrics: "绩效指标",
  get_return_attribution: "收益归因",
  get_monthly_returns: "月度收益",
  get_drawdown_analysis: "回撤分析",
  get_risk_metrics: "风险指标",
  get_holding_ranking: "持仓排名",
  get_dividend_income: "分红收入",
  check_price_alerts: "价格提醒",
  get_option_positions: "期权持仓",
  get_option_review: "期权操作复盘",
  get_stock_review: "股票操作复盘",
  save_stock_review_annotation: "保存股票复盘注释",
};

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
  // Subscribe to the streaming session id so the sidebar can highlight which
  // session is actively generating (foreground or background). Selector form
  // avoids re-rendering the whole page on every unrelated chatStore change.
  const streamingSessionId = useChatStore((s) => s.streamingSessionIdState);
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

  // One-time bootstrap: bind streaming listeners, load AI config, load the
  // session list. We deliberately leave currentSessionId null so the page
  // opens on the "new chat" welcome screen (ChatGPT-style) — the user picks
  // a history item from the sidebar or starts typing to begin a new chat,
  // at which point a session is created lazily.
  useEffect(() => {
    init();
    fetchConfig();
    void fetchSkills();
    (async () => {
      await fetchSessions();
      setBootstrapped(true);
    })();
  }, [init, fetchConfig, fetchSessions]);

  // "New chat" button: switch to the welcome screen without creating a
  // session. A session is created lazily when the user actually sends.
  const handleNewChat = () => {
    setCurrentSession(null);
  };

  const handleDeleteSession = async (id: string) => {
    await deleteSession(id);
  };

  return (
    // The global MainLayout wraps every page in a `<Content className="p-6">`
    // that adds 24px of padding on all sides. For most pages that whitespace
    // is desirable, but the AI assistant has its own full-height sidebar
    // that should dock flush against the left edge (next to the nav menu).
    // We cancel the parent's padding with a negative margin and grow to fill
    // the resulting expanded box.
    <div
      className="flex"
      style={{ margin: "-24px", height: "calc(100% + 48px)" }}
    >
      <SessionSidebar
        sessions={sessions}
        currentSessionId={currentSessionId}
        streamingSessionId={streamingSessionId}
        onSelect={setCurrentSession}
        onNew={handleNewChat}
        onDelete={handleDeleteSession}
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
          <div className="flex-1 flex items-center justify-center" style={{ color: "var(--color-text-tertiary)" }}>
            <Text type="secondary">加载中…</Text>
          </div>
        )}
      </div>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Session sidebar
// ─────────────────────────────────────────────────────────────────────────────

function SessionSidebar({
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

// ─────────────────────────────────────────────────────────────────────────────
// Model switcher (compact dropdown in the chat header)
// ─────────────────────────────────────────────────────────────────────────────

/// Compact model selector that lets the user switch the active model without
/// leaving the chat page. Fetches the provider's model list on mount and on
/// provider change; falls back to the current model id when the list is empty
/// or the fetch fails (no key, offline, provider unsupported).
///
/// The backend's `update_ai_config` is a full-replace, so we spread the
/// existing config and only override `model` — preserving provider/key/base_url.
function ModelSwitcher() {
  const { config, fetchModels, updateConfig } = useAiStore();
  const [models, setModels] = useState<AiModelInfo[]>([]);
  const [loading, setLoading] = useState(false);

  // Fetch the model list whenever the provider / key / base_url changes.
  // Best-effort: a failure (no key, offline) just leaves the list empty and
  // the Select renders the current model as a free-text option.
  useEffect(() => {
    if (!config) return;
    let cancelled = false;
    setLoading(true);
    fetchModels({
      provider: config.provider,
      api_key: config.api_key,
      base_url: config.base_url ?? undefined,
    })
      .then((list) => {
        if (!cancelled) setModels(list);
      })
      .catch(() => {
        // Silent: the switcher still works with just the current model.
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [config?.provider, config?.api_key, config?.base_url]);

  // Build options from the fetched list. Always include the current model so
  // the Select shows a valid value even when the list hasn't loaded yet or the
  // current model isn't in the provider's catalog.
  const options = useMemo(() => {
    const map = new Map<string, string>();
    for (const m of models) {
      map.set(m.id, m.name ? `${m.name}（${m.id}）` : m.id);
    }
    if (config?.model && !map.has(config.model)) {
      map.set(config.model, config.model);
    }
    return Array.from(map, ([value, label]) => ({ value, label }));
  }, [models, config?.model]);

  const handleChange = async (id: string) => {
    if (!config) return;
    await updateConfig({ ...config, model: id });
    message.success(`已切换到 ${id}`);
  };

  return (
    <Select
      size="small"
      showSearch
      style={{ minWidth: 160, maxWidth: 240 }}
      value={config?.model}
      options={options}
      loading={loading}
      onChange={handleChange}
      notFoundContent={loading ? "加载中..." : "暂无模型列表"}
      placeholder="选择模型"
    />
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Chat panel (right side)
// ─────────────────────────────────────────────────────────────────────────────

function ChatPanel({
  sessionId,
  navigate,
  initialPrompt,
}: {
  // null means "no active session" (welcome screen). A session is created
  // lazily on the first send.
  sessionId: string | null;
  navigate: ReturnType<typeof useNavigate>;
  initialPrompt: string | null;
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
  } = useChatStore();
  // Read the staged explicit selection so the Composer can render "待激活"
  // chips. Subscribing via the store keeps the chips reactive as the user
  // adds/removes skills via `/` or the × button.
  const pendingActiveSkillIds = useChatStore((s) => s.pendingActiveSkills);
  const { config } = useAiStore();
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
  const createSession = useChatSessionStore((s) => s.createSession);
  // Used by the "background stream" banner's "回到该会话" button to jump
  // directly to the session that's currently generating in the background.
  const setCurrentSession = useChatSessionStore((s) => s.setCurrentSession);

  const [input, setInput] = useState("");
  const seededPromptRef = useRef<string | null>(null);
  useEffect(() => {
    if (!initialPrompt || seededPromptRef.current === initialPrompt) return;
    seededPromptRef.current = initialPrompt;
    setInput((current) => current.trim().length > 0 ? current : initialPrompt);
  }, [initialPrompt]);
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
  // Set BEFORE calling ensureSession() when sending from the welcome screen.
  // It tells the session-change effect that the *next* sessionId change
  // (null → newly created id) is the send-from-welcome flow and must NOT
  // reload — the in-memory user + assistant placeholder is about to be
  // pushed and a reload (DB still empty) would wipe it, bouncing the UI
  // back to the welcome screen.
  //
  // This flag must be set *before* the await on ensureSession, not after.
  // createSession() synchronously commits currentSessionId to the store,
  // which schedules a React re-render; that re-render (and this effect) can
  // run during the await, before any code after ensureSession gets a chance
  // to run. Setting the flag post-await loses the race.
  //
  // One-shot: consumed and cleared by the effect on the first sessionId
  // change after being set, so it can't suppress a later genuine switch.
  const expectingSessionCreation = useRef(false);

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
    if (sessionId === null) {
      // Switching to "new chat" welcome screen: clear the loaded session
      // marker and wipe the in-memory messages so the welcome hero shows
      // instead of the previous conversation. Abort any in-flight stream.
      loadedSessionRef.current = null;
      expectingSessionCreation.current = false;
      void resetForSessionSwitch();
      return;
    }
    if (loadedSessionRef.current === sessionId) return;
    loadedSessionRef.current = sessionId;
    // Sending from welcome screen: skip reload, keep in-flight messages.
    if (expectingSessionCreation.current) {
      expectingSessionCreation.current = false;
      return;
    }
    (async () => {
      await resetForSessionSwitch();
      await loadSessionMessages(sessionId);
    })();
    // Intentionally exclude messages.length — see comment above.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [sessionId, resetForSessionSwitch, loadSessionMessages]);

  useEffect(() => {
    messagesEndRef.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages]);

  const sessionTotal = useMemo(
    () => selectSessionTotalTokens(messages),
    [messages],
  );

  const providerIsOllama = config?.provider === "ollama";
  const notConfigured =
    config && (!config.model || (!providerIsOllama && !config.api_key));

  // Resolve the effective session id, creating one on the fly if the user is
  // composing in the no-session welcome state. Returns null if creation
  // failed (so the caller can bail out).
  const ensureSession = async (): Promise<string | null> => {
    if (sessionId) return sessionId;
    try {
      const s = await createSession();
      return s.id;
    } catch (err) {
      message.error("创建会话失败：" + String(err));
      return null;
    }
  };

  const handleSend = async () => {
    if (!input.trim()) return;
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
    // Sending from the welcome screen: ensureSession will create a session
    // and currentSessionId flips null → newId. Set the expectation flag
    // BEFORE the await so the session-change effect (which can run during
    // the await) skips its reload — otherwise resetForSessionSwitch wipes
    // the in-memory placeholder and the UI bounces back to the welcome hero.
    const wasNewChat = !sessionId;
    if (wasNewChat) expectingSessionCreation.current = true;
    const sid = await ensureSession();
    if (!sid) {
      expectingSessionCreation.current = false;
      return;
    }
    await sendMessage(text, sid);
    await touchSession(sid);
    if (wasEmpty) {
      void autoRenameIfDefault(sid, text);
    }
  };

  const handleSuggestion = async (s: string) => {
    if (notConfigured) return;
    if (sending) {
      message.warning(
        streamingInBackground
          ? "有一条 AI 回复正在后台生成，请等待完成后再发送"
          : "AI 正在回复中，请等待当前回复完成后再发送",
      );
      return;
    }
    const wasEmpty = messages.length === 0;
    const wasNewChat = !sessionId;
    if (wasNewChat) expectingSessionCreation.current = true;
    const sid = await ensureSession();
    if (!sid) {
      expectingSessionCreation.current = false;
      return;
    }
    await sendMessage(s, sid);
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
    const current = pendingActiveSkillIds.filter((id) => id !== skill.id);
    setActiveSkillsForNextTurn([...current, skill.id]);
    message.success(`已激活技能：${skill.name}（发送时生效）`);
  };

  // Remove a single staged skill via the × on its chip.
  const handleRemoveStagedSkill = (skillId: string) => {
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
    if (sending) {
      message.warning("AI 正在回复中，请等待当前回复完成");
      return;
    }
    setActiveSkillsForNextTurn([skill.id]);
    const prompt = `请使用「${skill.name}」技能帮我分析当前的投资组合。`;
    const wasEmpty = messages.length === 0;
    const wasNewChat = !sessionId;
    if (wasNewChat) expectingSessionCreation.current = true;
    const sid = await ensureSession();
    if (!sid) {
      expectingSessionCreation.current = false;
      return;
    }
    await sendMessage(prompt, sid);
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
                disabled={sending || messages.length === 0}
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
                  canEdit={!sending}
                  onEdit={(text) => {
                    if (sessionId) editAndResend(m.id, text, sessionId);
                  }}
                  onRetry={
                    m.error && sessionId
                      ? () => void retryLastTurn(sessionId)
                      : undefined
                  }
                  onDismiss={m.error ? () => dismissError(m.id) : undefined}
                  // Regenerate is available on any completed (non-error)
                  // assistant answer that isn't the in-flight streaming row.
                  // Disabled entirely while a stream is running.
                  onRegenerate={
                    sessionId && !m.error && !sending
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
                      style={{ cursor: "pointer", marginInlineEnd: 0 }}
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
                  disabled={!!notConfigured}
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

function Composer({
  input,
  setInput,
  handleKeyDown,
  handleSend,
  stopGeneration,
  sending,
  notConfigured,
  size = "default",
  skills,
  onPickSkill,
  stagedSkills,
  onRemoveStagedSkill,
}: {
  input: string;
  setInput: (v: string) => void;
  handleKeyDown: (e: React.KeyboardEvent<HTMLTextAreaElement>) => void;
  handleSend: () => void;
  stopGeneration: () => void;
  sending: boolean;
  notConfigured: boolean;
  size?: "default" | "large";
  /** Skills available for `/` autocomplete. */
  skills: Skill[];
  /** Called when the user picks a skill from the `/` popover. */
  onPickSkill: (skill: Skill) => void;
  /** Skills currently staged for the next send (rendered as chips). */
  stagedSkills: Skill[];
  /** Remove a staged skill (the × button on its chip). */
  onRemoveStagedSkill: (skillId: string) => void;
}) {
  const minRows = size === "large" ? 2 : 1;
  const canSend = input.trim().length > 0 && !notConfigured;

  // `/` autocomplete: when the text ends with `/` (optionally followed by a
  // filter prefix with no intervening whitespace), show a filtered skill list.
  // Picking one stages the skill for explicit activation and removes the `/…`
  // token from the input.
  const slashMatch = input.match(/(^|\s)\/([^\s/]*)$/);
  const slashOpen = !!slashMatch && skills.length > 0;
  const slashFilter = slashMatch ? slashMatch[2].toLowerCase() : "";
  const filteredSkills = useMemo(() => {
    if (!slashOpen) return [];
    return skills.filter(
      (s) =>
        s.name.toLowerCase().includes(slashFilter) ||
        s.id.toLowerCase().includes(slashFilter),
    );
  }, [slashOpen, slashFilter, skills]);

  // Keyboard navigation within the `/` popover. `activeIdx` is the focused
  // row; ArrowUp/ArrowDown move it (with wrap-around), Enter picks, Escape
  // dismisses by clearing the `/…` token. Reset whenever the filter or open
  // state changes so the highlight doesn't point at a stale row.
  const [activeIdx, setActiveIdx] = useState(0);
  useEffect(() => {
    setActiveIdx(0);
  }, [slashOpen, slashFilter]);

  const pickSkill = (skill: Skill) => {
    // Strip the trailing `/…` token (the match group spans from the leading
    // whitespace-or-start through the end of input).
    if (slashMatch) {
      const stripped = input.slice(0, input.length - slashMatch[0].length);
      setInput(stripped);
    }
    onPickSkill(skill);
  };

  const dismissSlash = () => {
    // Remove the trailing `/…` token — closing the menu by editing rather
    // than by an external flag keeps the open state a pure function of input.
    if (slashMatch) {
      const stripped = input.slice(0, input.length - slashMatch[0].length);
      setInput(stripped);
    }
  };

  const onTextareaKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (slashOpen && filteredSkills.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setActiveIdx((i) => (i + 1) % filteredSkills.length);
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setActiveIdx((i) => (i - 1 + filteredSkills.length) % filteredSkills.length);
        return;
      }
      if (e.key === "Enter" && !e.ctrlKey && !e.metaKey && !e.shiftKey) {
        // Plain Enter selects the highlighted skill; Cmd/Ctrl+Enter still
        // sends (handled by handleKeyDown below).
        e.preventDefault();
        pickSkill(filteredSkills[activeIdx]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        dismissSlash();
        return;
      }
    }
    handleKeyDown(e);
  };

  // The send/stop button sits in the bottom-right corner of the textarea
  // wrapper. To keep the button inside the box at every textarea height we
  // fix the vertical padding (`py`) and place the button flush with that
  // padding (`bottom: 7`), and bump the small-size min height so there is
  // always room for the 34px button plus breathing space.
  return (
    <div>
      {/* Staged-skill chips above the textarea so the user can see — and
          remove — the explicit selection that will apply to the next send. */}
      {stagedSkills.length > 0 && (
        <div className="flex flex-wrap items-center gap-1 mb-2">
          <Text type="secondary" style={{ fontSize: 12 }}>
            待激活：
          </Text>
          {stagedSkills.map((s) => (
            <Tag
              key={s.id}
              color="purple"
              icon={<ThunderboltOutlined />}
              closable
              onClose={() => onRemoveStagedSkill(s.id)}
              style={{ marginInlineEnd: 0 }}
            >
              {s.name}
            </Tag>
          ))}
        </div>
      )}
      <div
        className="rounded-lg border"
        style={{ borderColor: "var(--color-border)", backgroundColor: "var(--color-bg-elevated)" }}
      >
      <Popover
        open={slashOpen}
        placement="topLeft"
        trigger={[]}
        showArrow={false}
        overlayStyle={{ minWidth: 280 }}
        content={
          <div
            className="overflow-auto"
            style={{ maxHeight: 264 }}
            // Clicking outside the popover closes it via input mutation
            // (handled by AntD's onOpenChange → we re-derive from input).
            onMouseDown={(e) => e.preventDefault()}
          >
            {filteredSkills.length === 0 ? (
              <div style={{ padding: "8px 12px" }}>
                <Text type="secondary" style={{ fontSize: 12 }}>
                  没有匹配的技能
                </Text>
              </div>
            ) : (
              filteredSkills.map((s, idx) => (
                <button
                  key={s.id}
                  type="button"
                  onClick={() => pickSkill(s)}
                  onMouseEnter={() => setActiveIdx(idx)}
                  className="block w-full text-left rounded transition-colors"
                  style={{
                    border: "none",
                    background:
                      idx === activeIdx ? "color-mix(in srgb, var(--color-info) 15%, transparent)" : "transparent",
                    padding: "6px 10px",
                    cursor: "pointer",
                  }}
                >
                  <div className="flex items-center gap-2">
                    <ThunderboltOutlined style={{ color: "var(--color-info)" }} />
                    <span style={{ fontWeight: 500 }}>{s.name}</span>
                    {s.source === "builtin" && (
                      <Tag style={{ marginInlineEnd: 0, fontSize: 11 }}>内置</Tag>
                    )}
                  </div>
                  {s.description && (
                    <div
                      style={{ fontSize: 12, marginTop: 2, paddingLeft: 20, color: "var(--color-text-secondary)" }}
                    >
                      {s.description}
                    </div>
                  )}
                </button>
              ))
            )}
          </div>
        }
      >
        {/* Anchor: the textarea itself. Popover attaches to its top-left,
            which is reliably near where `/` was typed on a fresh composer
            and avoids the old zero-size corner div. */}
        <TextArea
          value={input}
          onChange={(e) => setInput(e.target.value)}
          onKeyDown={onTextareaKeyDown}
          placeholder={
            notConfigured
              ? "请先完成 AI 配置…"
              : "输入问题，Ctrl/⌘+Enter 发送。输入 / 选择技能"
          }
          autoSize={{ minRows, maxRows: 8 }}
          disabled={notConfigured}
          // Borderless: the outer wrapper provides the border so the bottom
          // toolbar (model switcher + send button) sits flush inside it.
          variant="borderless"
          style={{
            padding: "12px 14px 8px",
            minHeight: size === "large" ? 72 : 60,
            ...(size === "large" ? { fontSize: 15 } : {}),
          }}
        />
      </Popover>
      {/* Bottom toolbar inside the input box: model switcher on the left,
          send/stop button on the right — like the reference screenshot. */}
      <div className="flex items-center justify-between px-2 pb-2 pt-1">
        <ModelSwitcher />
        <button
          type="button"
          onClick={sending ? stopGeneration : handleSend}
          disabled={!sending && !canSend}
          aria-label={sending ? "停止生成" : "发送"}
          className="flex items-center justify-center text-white transition-opacity disabled:cursor-not-allowed disabled:opacity-40"
          style={{
            width: 34,
            height: 34,
            borderRadius: 9999,
            border: "none",
            cursor: "pointer",
            background: sending
              ? "linear-gradient(135deg, var(--color-error) 0%, #dc2626 100%)"
              : "linear-gradient(135deg, var(--color-info) 0%, #4f46e5 100%)",
            boxShadow: "0 2px 6px color-mix(in srgb, var(--color-text) 15%, transparent)",
          }}
        >
          {sending ? <StopOutlined style={{ fontSize: 16 }} /> : <SendOutlined style={{ fontSize: 16 }} />}
        </button>
      </div>
      </div>
    </div>
  );
}

function MessageRow({
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
                  已查询：{TOOL_LABELS[name] ?? name}
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
 * Choose the empty-state placeholder text while streaming based on what the
 * model is currently doing, so the user has a clearer signal than a generic
 * "思考中…". Priority: actively running tools → reasoning in progress →
 * waiting for the first token.
 */
function statusPlaceholder(message: ChatMessageWithMeta): string {
  const toolsRunning = message.toolCalls?.some((t) => t.status === "running");
  if (toolsRunning) return "正在查询数据…";
  if (message.reasoning && message.reasoning.length > 0) return "正在思考…";
  return "思考中…";
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

function formatTime(epochMs: number): string {
  const d = new Date(epochMs);
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${hh}:${mm}`;
}

/** Render an RFC3339 timestamp as a short Chinese relative label. */
function formatRelativeTime(iso: string): string {
  const t = new Date(iso).getTime();
  if (isNaN(t)) return "";
  const diffMs = Date.now() - t;
  const sec = Math.floor(diffMs / 1000);
  const min = Math.floor(sec / 60);
  const hr = Math.floor(min / 60);
  const day = Math.floor(hr / 24);

  if (sec < 60) return "刚刚";
  if (min < 60) return `${min} 分钟前`;
  if (hr < 24) return `${hr} 小时前`;
  if (day === 1) return "昨天";
  if (day < 7) return `${day} 天前`;

  const d = new Date(iso);
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const dd = String(d.getDate()).padStart(2, "0");
  const hh = String(d.getHours()).padStart(2, "0");
  const mm = String(d.getMinutes()).padStart(2, "0");
  return `${y}-${m}-${dd} ${hh}:${mm}`;
}
