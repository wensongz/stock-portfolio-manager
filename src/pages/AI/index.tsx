import { useEffect, useMemo, useState } from "react";
import {
  Card,
  Form,
  Input,
  Button,
  Select,
  Typography,
  Alert,
  message,
  Divider,
  Space,
  Tooltip,
  Switch,
  Tabs,
  Segmented,
  Modal,
  Tag,
  Popconfirm,
  Empty,
  Dropdown,
  Spin,
} from "antd";
import {
  SaveOutlined,
  ReloadOutlined,
  EditOutlined,
  InfoCircleOutlined,
  UndoOutlined,
  PlusOutlined,
  DeleteOutlined,
  ThunderboltOutlined,
  EyeOutlined,
  CodeOutlined,
  CopyOutlined,
  ExportOutlined,
  ImportOutlined,
  ExperimentOutlined,
  MoreOutlined,
} from "@ant-design/icons";
import ReactMarkdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { useNavigate } from "react-router-dom";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";
import { useAiStore } from "../../stores/aiStore";
import { useSkillStore } from "../../stores/skillStore";
import { useChatStore } from "../../stores/chatStore";
import type { AiModelInfo, AiProvider, Skill } from "../../types";

const { Text, Paragraph } = Typography;
const { TextArea } = Input;

interface ProviderOption {
  value: AiProvider;
  label: string;
  hint: string;
  default_base_url?: string;
  needs_key: boolean;
  key_placeholder?: string;
}

/** The providers we support. All expose an OpenAI-compatible `/models`
 *  endpoint, so models are always fetched dynamically from the API. */
const PROVIDERS: ProviderOption[] = [
  {
    value: "openai",
    label: "OpenAI",
    hint: "官方 OpenAI API（ChatGPT 背后的服务）",
    default_base_url: "https://api.openai.com/v1",
    needs_key: true,
    key_placeholder: "sk-...",
  },
  {
    value: "ollama",
    label: "Ollama（本地）",
    hint: "本地运行的 Ollama 服务，无需 API Key",
    default_base_url: "http://localhost:11434",
    needs_key: false,
  },
  {
    value: "openrouter",
    label: "OpenRouter",
    hint: "聚合多家模型（OpenAI、Google、Meta 等），OpenAI 兼容 API",
    default_base_url: "https://openrouter.ai/api/v1",
    needs_key: true,
    key_placeholder: "sk-or-...",
  },
  {
    value: "kimi",
    label: "Kimi（月之暗面）",
    hint: "国内 Kimi / Moonshot，OpenAI 兼容 API",
    default_base_url: "https://api.moonshot.ai/v1",
    needs_key: true,
    key_placeholder: "sk-...",
  },
  {
    value: "glm",
    label: "GLM（智谱）",
    hint: "智谱 GLM 系列模型，OpenAI 兼容 API（v4 端点）",
    default_base_url: "https://open.bigmodel.cn/api/paas/v4",
    needs_key: true,
    key_placeholder: "xxx.xxx",
  },
  {
    value: "mimo",
    label: "MiMo（小米）",
    hint: "小米 MiMo 大模型开放平台，OpenAI 兼容 API",
    default_base_url: "https://api.xiaomimimo.com/v1",
    needs_key: true,
    key_placeholder: "sk-...",
  },
  {
    value: "deepseek",
    label: "DeepSeek（深度求索）",
    hint: "DeepSeek 官方 API，OpenAI 兼容",
    default_base_url: "https://api.deepseek.com",
    needs_key: true,
    key_placeholder: "sk-...",
  },
  {
    value: "anthropic",
    label: "Anthropic（Claude）",
    hint: "Claude 官方 API，使用 Anthropic Messages 协议",
    default_base_url: "https://api.anthropic.com",
    needs_key: true,
    key_placeholder: "sk-ant-...",
  },
];

function providerOf(value: string): ProviderOption | undefined {
  return PROVIDERS.find((p) => p.value === value);
}

/** The "API 设置" tab — LLM connection (provider / key / endpoint / model /
 *  system prompt) plus the static "功能说明" card. Kept together because both
 *  belong to API-level configuration. */
function ApiSettingsCard() {
  const { config, loading, fetchConfig, updateConfig, fetchModels, getDefaultSystemPrompt } =
    useAiStore();
  const [form] = Form.useForm();
  const [models, setModels] = useState<AiModelInfo[]>([]);
  const [modelsLoading, setModelsLoading] = useState(false);
  const [manualModel, setManualModel] = useState(false);

  useEffect(() => {
    fetchConfig();
  }, [fetchConfig]);

  const handleRestorePrompt = async () => {
    try {
      const def = await getDefaultSystemPrompt();
      form.setFieldValue("system_prompt", def);
      message.success("已恢复默认提示词（需点击保存生效）");
    } catch (err) {
      message.error("恢复失败：" + String(err));
    }
  };

  useEffect(() => {
    if (config) {
      form.setFieldsValue({
        provider: config.provider,
        api_key: config.api_key,
        model: config.model,
        base_url: config.base_url || "",
        system_prompt: config.system_prompt,
      });
      // If the saved model isn't empty but also isn't in any list we know,
      // default to manual mode so the user can still see/edit it.
      setManualModel(Boolean(config.model));
    }
  }, [config, form]);

  const selectedProvider = Form.useWatch("provider", form);
  const apiKey = Form.useWatch("api_key", form);
  const baseUrl = Form.useWatch("base_url", form);

  const provider = useMemo(
    () => providerOf(selectedProvider ?? "") ?? PROVIDERS[0],
    [selectedProvider],
  );

  const handleFetchModels = async () => {
    if (provider.needs_key && !apiKey) {
      message.warning("请先填写 API Key");
      return;
    }
    setModelsLoading(true);
    try {
      const list = await fetchModels({
        provider: provider.value,
        api_key: apiKey,
        base_url: baseUrl || null,
      });
      setModels(list);
      if (list.length === 0) {
        message.info("API 未返回任何模型，请手动输入模型名称");
        setManualModel(true);
      } else {
        message.success(`已获取 ${list.length} 个模型`);
        // Keep manual mode off unless the current model isn't in the list.
        const current = form.getFieldValue("model");
        if (current && !list.some((m) => m.id === current)) {
          form.setFieldValue("model", list[0].id);
        } else if (!current) {
          form.setFieldValue("model", list[0].id);
        }
        setManualModel(false);
      }
    } catch (err) {
      message.error("获取模型失败：" + String(err) + "，请改为手动输入");
      setManualModel(true);
    } finally {
      setModelsLoading(false);
    }
  };

  // Auto-fetch models when the user has entered a key (for key-less providers,
  // immediately on selection) and picks a provider. All supported providers
  // expose an OpenAI-compatible `/models` endpoint.
  useEffect(() => {
    if (
      selectedProvider &&
      (provider.needs_key ? Boolean(apiKey) : true) &&
      !manualModel
    ) {
      handleFetchModels();
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedProvider]);

  const handleSave = async () => {
    try {
      const values = await form.validateFields();
      const success = await updateConfig({
        provider: values.provider,
        api_key: values.api_key,
        model: values.model,
        base_url: values.base_url || null,
        system_prompt: values.system_prompt,
        tools_enabled: config?.tools_enabled ?? true,
      });
      if (success) {
        message.success("AI 配置已保存");
      }
    } catch (err) {
      message.error("保存失败: " + String(err));
    }
  };

  const modelOptions = useMemo(
    () =>
      models.map((m) => ({
        value: m.id,
        label: m.name ? `${m.name}（${m.id}）` : m.id,
      })),
    [models],
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      <Card title="API 配置">
        <Form form={form} layout="vertical" style={{ maxWidth: 600 }}>
          <Form.Item name="provider" label="AI 提供商" rules={[{ required: true }]}>
            <Select
              options={PROVIDERS.map((p) => ({
                value: p.value,
                label: (
                  <Space>
                    <span>{p.label}</span>
                    <Tooltip title={p.hint}>
                      <InfoCircleOutlined style={{ color: "var(--color-text-tertiary)" }} />
                    </Tooltip>
                  </Space>
                ),
              }))}
            />
          </Form.Item>

          <Form.Item
            name="api_key"
            label={
              <Space>
                API Key
                {provider.needs_key ? null : (
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    （该提供商不需要）
                  </Text>
                )}
              </Space>
            }
            rules={
              provider.needs_key
                ? [{ required: true, message: "请输入 API Key" }]
                : []
            }
          >
            <Input.Password placeholder={provider.key_placeholder ?? ""} />
          </Form.Item>

          <Form.Item
            name="base_url"
            label={
              <Space>
                API 端点
                <Text type="secondary" style={{ fontSize: 12 }}>
                  （留空使用默认：{provider.default_base_url ?? "—"}）
                </Text>
              </Space>
            }
          >
            <Input placeholder={provider.default_base_url ?? ""} />
          </Form.Item>

          <Form.Item
            name="model"
            label={
              <Space
                style={{ justifyContent: "space-between", width: "100%" }}
              >
                <span>模型</span>
                <Space size="small">
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    手动输入
                  </Text>
                  <Switch
                    size="small"
                    checked={manualModel}
                    onChange={(v) => {
                      setManualModel(v);
                      if (v) {
                        setModels([]);
                      }
                    }}
                  />
                  <Button
                    size="small"
                    type="link"
                    icon={<ReloadOutlined />}
                    loading={modelsLoading}
                    onClick={handleFetchModels}
                    disabled={manualModel}
                  >
                    获取模型
                  </Button>
                </Space>
              </Space>
            }
            rules={[{ required: true, message: "请选择或输入模型" }]}
          >
            {manualModel ? (
              <Input
                prefix={<EditOutlined />}
                placeholder="例如：gpt-4o、claude-3-7-sonnet、llama3.2 ..."
              />
            ) : (
              <Select
                showSearch
                placeholder="点击「获取模型」拉取可用模型"
                options={modelOptions}
                notFoundContent={
                  modelsLoading
                    ? "加载中..."
                    : "暂无模型，请点击「获取模型」或切换为手动输入"
                }
              />
            )}
          </Form.Item>

          <Form.Item
            name="system_prompt"
            label={
              <Space
                style={{ justifyContent: "space-between", width: "100%" }}
              >
                <span>系统提示词</span>
                <Button
                  size="small"
                  type="link"
                  icon={<UndoOutlined />}
                  onClick={handleRestorePrompt}
                >
                  恢复默认
                </Button>
              </Space>
            }
            extra={
              <Text type="secondary" style={{ fontSize: 12 }}>
                定义 AI 的角色、职责与回答风格。开启对话中的「注入数据」后，会附带使用缓存行情的持仓、近期交易与绩效快照；从复盘或组合提醒入口进入时，会提供对应范围的数据。技能会补充分析步骤与输出格式。已有提示词不会自动覆盖，可点击「恢复默认」载入最新版，再保存生效。
              </Text>
            }
          >
            <TextArea
              rows={10}
              placeholder="例如：你是一位经验丰富、客观中立的个人投资组合分析助手..."
            />
          </Form.Item>

          <Form.Item>
            <Button
              type="primary"
              icon={<SaveOutlined />}
              loading={loading}
              onClick={handleSave}
            >
              保存配置
            </Button>
          </Form.Item>
        </Form>
      </Card>

      <Card title="功能说明">
        <Paragraph>配置完成后，可在「AI 助手」中对话，或从复盘、组合提醒页面发起分析：</Paragraph>
        <ul className="list-disc list-inside space-y-1">
          <li>查询大盘与个股行情、历史价格、估值和技术指标，以及 A 股财报</li>
          <li>分析组合集中度、市场与账户分布、收益归因、月度收益和回撤风险</li>
          <li>查询交易、分红和期权持仓，基于系统计算的股票与期权复盘报告分析操作效果</li>
          <li>从组合提醒发起再平衡分析，按目标配置生成不追加资金的调整建议</li>
          <li>使用内置或自定义技能进行风险体检、季度回顾和专项分析；支持关键词触发及输入 / 选择技能</li>
          <li>管理多个会话、编辑问题重发、重新生成回答，并查看工具查询详情与 token 用量</li>
        </ul>
        <Divider />
        <Paragraph type="secondary">
          数据查询需要所选模型支持工具调用，行情与财报的覆盖范围、时效以数据源返回为准。关闭「注入数据」只停止自动附带组合快照，工具仍可按需查询数据。AI 工具用于查询和分析，不会修改持仓、交易记录或自动下单。
        </Paragraph>
        <Text type="secondary">
          注意：AI 分析仅供参考，不构成投资建议。投资有风险，入市需谨慎。
        </Text>
      </Card>
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Page shell — renders connection/data guidance and the API / Skills tabs.
// (No page title here: AIPage is itself the content of the "🤖 AI 配置" tab on
// the Settings page, which already shows the page title and tab label.)
// ─────────────────────────────────────────────────────────────────────────────

export default function AIPage() {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      <Alert
        type="info"
        title="连接与数据说明"
        description="支持 OpenAI、Anthropic（Claude）、Ollama、OpenRouter、Kimi、GLM（智谱）、MiMo（小米）和 DeepSeek。可获取模型列表或手动输入模型名称，本地 Ollama 无需 API Key。API Key 保存在本地，并用于向你配置的 API 端点鉴权；对话、启用的技能及随请求附带的组合数据和工具结果会发送至该端点。请按所选服务了解数据使用政策与费用。"
        showIcon
      />

      <Tabs
        // Keep both tabs mounted so the API Form's field values aren't reset
        // when switching to Skills and back.
        destroyOnHidden={false}
        items={[
          {
            key: "api",
            label: "API 设置",
            children: <ApiSettingsCard />,
          },
          {
            key: "skills",
            label: "技能设置",
            children: <SkillManager />,
          },
        ]}
      />
    </div>
  );
}

// ─────────────────────────────────────────────────────────────────────────────
// Skill management panel
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Strict kebab-case: lowercase letter first, then lowercase letters / digits /
 * hyphen-separated segments. No leading digit, no consecutive or trailing
 * hyphens. Examples: `my-risk-skill`, `portfolio2`, `q3-report`.
 */
const SKILL_ID_PATTERN = /^[a-z]([a-z0-9]*)(-[a-z0-9]+)*$/;

/** Validate a skill id; returns an error message or null when valid. */
function validateSkillId(id: string): string | null {
  const trimmed = id.trim();
  if (!trimmed) return "请填写技能 ID";
  if (!SKILL_ID_PATTERN.test(trimmed)) {
    return "技能 ID 需为小写字母开头的 kebab-case（仅小写字母、数字、连字符，例如 my-risk-skill）";
  }
  return null;
}

/** A blank skill template the "新建技能" modal starts from. */
function emptyDraft(): Skill {
  return {
    id: "",
    name: "",
    description: "",
    trigger: [],
    enabled: true,
    content: "",
    source: "user",
    updatedAt: "",
  };
}

function SkillManager() {
  const {
    skills,
    loading,
    error,
    fetchSkills,
    saveSkill,
    deleteSkill,
    resetSkills,
    cloneSkill,
    exportSkill,
    importSkill,
  } = useSkillStore();
  const navigate = useNavigate();
  const setActiveSkillsForNextTurn = useChatStore(
    (s) => s.setActiveSkillsForNextTurn,
  );
  const [modalOpen, setModalOpen] = useState(false);
  const [draft, setDraft] = useState<Skill>(emptyDraft());
  const [saving, setSaving] = useState(false);
  // Editor mode: "edit" shows the raw Markdown textarea, "preview" renders it.
  // The toggle lives in the modal header so the user can flip while typing.
  const [editorMode, setEditorMode] = useState<"edit" | "preview">("edit");

  useEffect(() => {
    fetchSkills();
  }, [fetchSkills]);

  const openCreate = () => {
    setDraft(emptyDraft());
    setEditorMode("edit");
    setModalOpen(true);
  };

  const openEdit = (skill: Skill) => {
    // Edit by copy: always write back as a user-owned skill (save_skill drops
    // the builtin marker), so editing a builtin effectively forks it.
    setDraft({ ...skill, source: "user" });
    setEditorMode("edit");
    setModalOpen(true);
  };

  const handleSaveDraft = async () => {
    const trimmedId = draft.id.trim();
    const trimmedName = draft.name.trim();
    const idError = validateSkillId(trimmedId);
    if (idError) {
      message.warning(idError);
      return;
    }
    if (!trimmedName) {
      message.warning("请填写技能名称");
      return;
    }
    if (!draft.content.trim()) {
      message.warning("请填写技能正文（给 AI 的指令）");
      return;
    }
    setSaving(true);
    const ok = await saveSkill({
      ...draft,
      id: trimmedId,
      name: trimmedName,
    });
    setSaving(false);
    if (ok) {
      message.success("技能已保存");
      setModalOpen(false);
    }
  };

  const handleToggle = async (skill: Skill, enabled: boolean) => {
    // Flip enabled in place. save_skill writes the whole record back.
    await saveSkill({ ...skill, enabled });
  };

  const handleDelete = async (id: string) => {
    const ok = await deleteSkill(id);
    if (ok) message.success("技能已删除");
    else message.error("删除失败");
  };

  const handleReset = async () => {
    const ok = await resetSkills();
    if (ok) message.success("已恢复内置技能");
    else message.error("恢复失败");
  };

  // Clone a skill under a new id. Suggests `{source}-copy` and falls back to
  // `-copy-2`, `-copy-3`, … if that's taken, then saves immediately. The user
  // can rename via edit afterwards.
  const handleClone = async (skill: Skill) => {
    const taken = new Set(skills.map((s) => s.id));
    const base = `${skill.id}-copy`;
    let newId = base;
    let n = 2;
    while (taken.has(newId)) {
      newId = `${base}-${n}`;
      n += 1;
    }
    const cloned = await cloneSkill(skill.id, newId);
    if (cloned) {
      message.success(`已克隆为「${cloned.name}」`);
    } else {
      message.error("克隆失败");
    }
  };

  // Jump to the AI assistant with this skill staged for explicit activation
  // on the next send. The user lands on the welcome screen ready to type.
  const handleTest = (skill: Skill) => {
    setActiveSkillsForNextTurn([skill.id]);
    navigate("/ai-assistant");
    message.success(`已激活技能：${skill.name}，输入问题后发送即生效`);
  };

  const handleExport = async (skill: Skill) => {
    try {
      const path = await saveDialog({
        title: "导出技能",
        defaultPath: `${skill.id}.md`,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (!path) return;
      const written = await exportSkill(skill.id, String(path));
      if (written) message.success(`已导出到 ${written}`);
      else message.error("导出失败");
    } catch (err) {
      message.error("导出失败：" + String(err));
    }
  };

  const handleImport = async () => {
    try {
      const path = await openDialog({
        title: "导入技能",
        multiple: false,
        filters: [{ name: "Markdown", extensions: ["md"] }],
      });
      if (!path) return;
      const imported = await importSkill(String(path));
      if (imported) {
        message.success(`已导入技能「${imported.name}」`);
      } else {
        message.error("导入失败");
      }
    } catch (err) {
      message.error("导入失败：" + String(err));
    }
  };

  return (
    <Card
      title={
        <Space>
          <ThunderboltOutlined />
          <span>技能管理</span>
        </Space>
      }
      extra={
        <Space>
          <Button icon={<PlusOutlined />} onClick={openCreate}>
            新建技能
          </Button>
          <Button icon={<ImportOutlined />} onClick={handleImport}>
            导入
          </Button>
          <Popconfirm
            title="恢复内置技能？"
            description="将清除你对内置技能的所有改动并重新写入出厂版本，自定义技能会保留。"
            okText="恢复"
            cancelText="取消"
            onConfirm={handleReset}
          >
            <Button icon={<UndoOutlined />}>恢复内置</Button>
          </Popconfirm>
        </Space>
      }
    >
      {error && (
        <Alert
          type="error"
          showIcon
          title="技能加载失败"
          description={error}
          style={{ marginBottom: 12 }}
          action={
            <Button size="small" onClick={() => fetchSkills()}>
              重试
            </Button>
          }
        />
      )}
      <Paragraph type="secondary" style={{ marginBottom: 12 }}>
        技能是一段附加给 AI 的指令。开启「注入数据」后，匹配关键词的技能会自动激活；在 AI
        助手中输入 <Text code>/</Text> 可手动选择技能。
      </Paragraph>
      {skills.length === 0 && !loading ? (
        <Empty description="暂无技能，点击「新建技能」或「导入」按钮添加" />
      ) : (
        <Spin spinning={loading}>
          <div className="flex flex-col gap-2">
            {skills.map((skill) => (
              <div
                key={skill.id}
                className="flex items-start justify-between gap-3"
                style={{
                  padding: "12px 0",
                  borderBottom: "1px solid #f0f0f0",
                }}
              >
                <div className="flex-1 min-w-0">
                  <Space size="small">
                    <span>{skill.name}</span>
                    <Tag
                      color={skill.source === "builtin" ? "blue" : "green"}
                      style={{ marginInlineEnd: 0 }}
                    >
                      {skill.source === "builtin" ? "内置" : "自定义"}
                    </Tag>
                    <Text type="secondary" style={{ fontSize: 12 }}>
                      {skill.id}
                    </Text>
                  </Space>
                  <div style={{ marginTop: 4 }}>
                    <Space orientation="vertical" size={2} style={{ width: "100%" }}>
                      {skill.description && (
                        <Text type="secondary">{skill.description}</Text>
                      )}
                      {skill.trigger.length > 0 && (
                        <Space size={4} wrap>
                          <Text type="secondary" style={{ fontSize: 12 }}>
                            触发词：
                          </Text>
                          {skill.trigger.map((t) => (
                            <Tag key={t} style={{ marginInlineEnd: 0, fontSize: 12 }}>
                              {t}
                            </Tag>
                          ))}
                        </Space>
                      )}
                    </Space>
                  </div>
                </div>
                <Space size="small" className="flex-shrink-0">
                  <Tooltip title={skill.enabled ? "已启用自动激活" : "已停用"}>
                    <Switch
                      size="small"
                      checked={skill.enabled}
                      onChange={(checked) => handleToggle(skill, checked)}
                    />
                  </Tooltip>
                  <Button
                    size="small"
                    icon={<EditOutlined />}
                    onClick={() => openEdit(skill)}
                  >
                    编辑
                  </Button>
                  <Dropdown
                    menu={{
                      items: [
                        {
                          key: "test",
                          icon: <ExperimentOutlined />,
                          label: "测试技能",
                          onClick: () => handleTest(skill),
                        },
                        {
                          key: "clone",
                          icon: <CopyOutlined />,
                          label: "克隆",
                          onClick: () => handleClone(skill),
                        },
                        {
                          key: "export",
                          icon: <ExportOutlined />,
                          label: "导出…",
                          onClick: () => handleExport(skill),
                        },
                        { type: "divider" },
                        {
                          key: "delete",
                          icon: <DeleteOutlined />,
                          label: "删除",
                          danger: true,
                          onClick: () => handleDelete(skill.id),
                        },
                      ],
                    }}
                    trigger={["click"]}
                  >
                    <Button size="small" icon={<MoreOutlined />}>
                      更多
                    </Button>
                  </Dropdown>
                </Space>
              </div>
            ))}
          </div>
        </Spin>
      )}

      <Modal
        open={modalOpen}
        title={draft.content && skills.some((s) => s.id === draft.id) ? "编辑技能" : "新建技能"}
        onCancel={() => setModalOpen(false)}
        footer={[
          <Button key="cancel" onClick={() => setModalOpen(false)}>
            取消
          </Button>,
          <Button
            key="save"
            type="primary"
            icon={<SaveOutlined />}
            loading={saving}
            onClick={handleSaveDraft}
          >
            保存
          </Button>,
        ]}
        width={760}
        // Fixed body height keeps the editor visible and prevents the modal
        // from growing with content. Top form + bottom editor share the space.
        styles={{ body: { height: "calc(100vh - 200px)", minHeight: 480, padding: 0 } }}
        destroyOnHidden
      >
        <div className="flex h-full flex-col">
          {/* Top — compact metadata form (single column, fixed height) */}
          <div
            className="flex-shrink-0"
            style={{ padding: "16px 20px", borderBottom: "1px solid #f0f0f0" }}
          >
            <Form layout="vertical" style={{ maxWidth: "100%" }}>
              {/* First row: ID + name side by side */}
              <div className="flex gap-3">
                <Form.Item
                  label="技能 ID"
                  extra="英文 kebab-case，保存后不可改名"
                  // Show inline validation only while typing (new skills). When
                  // editing an existing skill the id is disabled and already
                  // valid, so we skip the check to avoid a stale error state.
                  validateStatus={
                    draft.id && !skills.some((s) => s.id === draft.id)
                      ? validateSkillId(draft.id)
                        ? "error"
                        : "success"
                      : ""
                  }
                  help={
                    draft.id && !skills.some((s) => s.id === draft.id)
                      ? validateSkillId(draft.id) ?? undefined
                      : undefined
                  }
                  style={{ marginBottom: 12, flex: 1 }}
                >
                  <Input
                    value={draft.id}
                    onChange={(e) => setDraft({ ...draft, id: e.target.value })}
                    placeholder="my-risk-skill"
                    disabled={skills.some((s) => s.id === draft.id) && !!draft.content}
                  />
                </Form.Item>
                <Form.Item
                  label="技能名称"
                  style={{ marginBottom: 12, flex: 1 }}
                >
                  <Input
                    value={draft.name}
                    onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                    placeholder="持仓风险分析"
                  />
                </Form.Item>
                {/* Inline enable switch aligned with the row baseline */}
                <Form.Item label="启用" style={{ marginBottom: 12, flex: "0 0 auto" }}>
                  <Switch
                    checked={draft.enabled}
                    onChange={(checked) => setDraft({ ...draft, enabled: checked })}
                  />
                </Form.Item>
              </div>
              <Form.Item
                label="描述"
                extra="显示在技能列表与快捷提示中"
                style={{ marginBottom: 12 }}
              >
                <TextArea
                  value={draft.description}
                  onChange={(e) => setDraft({ ...draft, description: e.target.value })}
                  placeholder="分析当前投资组合的风险敞口与集中度"
                  autoSize={{ minRows: 2, maxRows: 4 }}
                />
              </Form.Item>
              <Form.Item
                label="触发词"
                extra="输入后回车或按逗号自动分隔，匹配后自动激活"
                style={{ marginBottom: 0 }}
              >
                <Select
                  mode="tags"
                  value={draft.trigger}
                  onChange={(value) => setDraft({ ...draft, trigger: value })}
                  tokenSeparators={[",", "，"]}
                  placeholder="输入触发词后回车，例如：风险"
                  open={false}
                  suffixIcon={null}
                  style={{ width: "100%" }}
                />
              </Form.Item>
            </Form>
          </div>

          {/* Bottom — full-width Markdown editor / preview fills remaining height */}
          <div className="flex-1 flex flex-col min-h-0">
            <div
              className="flex items-center justify-between flex-shrink-0"
              style={{ padding: "8px 16px", borderBottom: "1px solid #f0f0f0" }}
            >
              <Segmented
                size="small"
                value={editorMode}
                onChange={(v) => setEditorMode(v as "edit" | "preview")}
                options={[
                  { label: "编辑", value: "edit", icon: <CodeOutlined /> },
                  { label: "预览", value: "preview", icon: <EyeOutlined /> },
                ]}
              />
              <Text type="secondary" style={{ fontSize: 12 }}>
                激活时作为附加指令注入给 AI
              </Text>
            </div>
            <div className="flex-1 min-h-0 p-3">
              {editorMode === "edit" ? (
                <TextArea
                  value={draft.content}
                  onChange={(e) => setDraft({ ...draft, content: e.target.value })}
                  placeholder={"# 技能标题\n\n## 任务\n1. ...\n2. ...\n\n## 输出格式\n..."}
                  style={{
                    height: "100%",
                    resize: "none",
                    fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
                    fontSize: 13,
                    lineHeight: 1.6,
                  }}
                />
              ) : (
                <div
                  className="ai-chat-md"
                  style={{
                    height: "100%",
                    overflow: "auto",
                    padding: "4px 8px",
                  }}
                >
                  {draft.content.trim() ? (
                    <ReactMarkdown remarkPlugins={[remarkGfm]}>
                      {draft.content}
                    </ReactMarkdown>
                  ) : (
                    <Text type="secondary">
                      没有正文可预览，切回「编辑」开始编写。
                    </Text>
                  )}
                </div>
              )}
            </div>
          </div>
        </div>
      </Modal>
    </Card>
  );
}
