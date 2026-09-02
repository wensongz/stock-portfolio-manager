import { useEffect, useMemo, useState } from "react";
import type { KeyboardEvent } from "react";
import { Input, Popover, Select, Tag, Typography, message } from "antd";
import {
  SendOutlined,
  StopOutlined,
  ThunderboltOutlined,
} from "@ant-design/icons";
import { useAiStore } from "../../stores/aiStore";
import type { AiModelInfo, Skill } from "../../types";

const { Text } = Typography;
const { TextArea } = Input;

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

export function Composer({
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
  handleKeyDown: (e: KeyboardEvent<HTMLTextAreaElement>) => void;
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

  const onTextareaKeyDown = (e: KeyboardEvent<HTMLTextAreaElement>) => {
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
