import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Alert,
  Badge,
  Button,
  Card,
  Empty,
  InputNumber,
  List,
  Modal,
  Select,
  Space,
  Spin,
  Switch,
  Table,
  Tag,
  Tooltip,
  Typography,
  message,
  notification,
} from "antd";
import {
  EditOutlined,
  ReloadOutlined,
  RobotOutlined,
  SaveOutlined,
} from "@ant-design/icons";
import type {
  Account,
  Category,
  PortfolioAlertConfig,
  PortfolioAlertNotification,
  PortfolioAlertScope,
  SavePortfolioAlertConfigInput,
} from "../../types";
import PieChart from "../../components/charts/PieChart";
import { useAccountStore } from "../../stores/accountStore";
import { useCategoryStore } from "../../stores/categoryStore";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import {
  portfolioAlertScopeKey,
  portfolioAlertStore,
  usePortfolioAlertStore,
} from "../../stores/portfolioAlertStore";
import { useQuoteStore } from "../../stores/quoteStore";
import {
  buildPortfolioAlertDisplayModel,
  buildPortfolioAlertNotificationPresentation,
  buildPortfolioAlertScopeOptions,
  decideDeletedPortfolioAlertScopeTransition,
  mergePortfolioAlertDraftCategories,
  overallScope,
  resolvePortfolioAlertCurrency,
  validatePortfolioAlertDraft,
  type PortfolioAlertDisplayRow,
  type PortfolioAlertDraft,
} from "./portfolioAlertViewModel";

const { Text, Title } = Typography;

function createDraft(
  scope: PortfolioAlertScope,
  categories: Category[],
  accounts: Account[],
  overallCurrency: "USD" | "CNY" | "HKD",
  config?: PortfolioAlertConfig | null,
): PortfolioAlertDraft {
  return mergePortfolioAlertDraftCategories({
    id: config?.id ?? null,
    scope,
    baseCurrency: resolvePortfolioAlertCurrency(scope, accounts, overallCurrency),
    deviationThreshold: config?.deviationThreshold ?? 20,
    concentrationThreshold: config?.concentrationThreshold ?? 20,
    isActive: config?.isActive ?? true,
    targets: config?.targets ?? [],
  }, categories);
}

function draftFingerprint(draft: PortfolioAlertDraft): string {
  return JSON.stringify({
    scope: draft.scope,
    deviationThreshold: draft.deviationThreshold,
    concentrationThreshold: draft.concentrationThreshold,
    isActive: draft.isActive,
    targets: draft.targets,
  });
}

function configFingerprint(config?: PortfolioAlertConfig | null): string {
  return config
    ? JSON.stringify({
        id: config.id,
        scope: config.scope,
        deviationThreshold: config.deviationThreshold,
        concentrationThreshold: config.concentrationThreshold,
        isActive: config.isActive,
        targets: config.targets,
      })
    : "unconfigured";
}

function alertType(statusColor: string): "info" | "success" | "warning" | "error" {
  if (statusColor === "success") return "success";
  if (statusColor === "warning") return "warning";
  if (statusColor === "error") return "error";
  return "info";
}

function formatEvaluatedAt(value: string | null): string {
  if (!value) return "尚无成功评估";
  const date = new Date(value);
  return Number.isNaN(date.getTime()) ? value : date.toLocaleString("zh-CN");
}

export default function PortfolioAlertsTab() {
  const accounts = useAccountStore((state) => state.accounts);
  const fetchAccounts = useAccountStore((state) => state.fetchAccounts);
  const categories = useCategoryStore((state) => state.categories);
  const fetchCategories = useCategoryStore((state) => state.fetchCategories);
  const categoriesLoading = useCategoryStore((state) => state.loading);
  const baseCurrency = useExchangeRateStore((state) => state.baseCurrency);
  const lastUpdatedAt = useQuoteStore((state) => state.lastUpdatedAt);
  const selectedScopeKey = usePortfolioAlertStore((state) => state.selectedScopeKey);
  const currentView = usePortfolioAlertStore(
    (state) => state.viewsByScope[state.selectedScopeKey],
  );
  const loading = usePortfolioAlertStore(
    (state) => state.loadingByScope[state.selectedScopeKey] ?? false,
  );
  const error = usePortfolioAlertStore(
    (state) => state.errorsByScope[state.selectedScopeKey],
  );
  const pendingNotifications = usePortfolioAlertStore(
    (state) => state.pendingNotifications,
  );
  const selectScope = usePortfolioAlertStore((state) => state.selectScope);
  const loadScope = usePortfolioAlertStore((state) => state.loadScope);
  const saveConfig = usePortfolioAlertStore((state) => state.saveConfig);
  const setActive = usePortfolioAlertStore((state) => state.setActive);
  const evaluate = usePortfolioAlertStore((state) => state.evaluate);
  const ingestNotification = usePortfolioAlertStore((state) => state.ingestNotification);
  const takePendingNotifications = usePortfolioAlertStore(
    (state) => state.takePendingNotifications,
  );
  const [notificationApi, notificationHolder] = notification.useNotification();
  const [dataReady, setDataReady] = useState(false);
  const [editing, setEditing] = useState(true);
  const [declinedDeletedScopeKey, setDeclinedDeletedScopeKey] = useState<string | null>(null);
  const [draft, setDraft] = useState<PortfolioAlertDraft>(() =>
    createDraft(overallScope(), [], [], baseCurrency),
  );
  const [baseline, setBaseline] = useState<PortfolioAlertDraft>(() =>
    createDraft(overallScope(), [], [], baseCurrency),
  );
  const initializedRef = useRef(false);
  const initialScopeLoadedRef = useRef(false);
  const syncedConfigRef = useRef<string | null>(null);
  const categorySignatureRef = useRef<string | null>(null);
  const selectedScopeKeyRef = useRef(selectedScopeKey);
  const pendingDeletedConfirmationRef = useRef<string | null>(null);
  const visibleRef = useRef<{ scope: PortfolioAlertScope; config: PortfolioAlertConfig | null }>({
    scope: overallScope(),
    config: null,
  });
  const observedQuoteAtRef = useRef(lastUpdatedAt);

  const scopeOptions = useMemo(
    () => buildPortfolioAlertScopeOptions(accounts),
    [accounts],
  );
  const selectedScopeOption = useMemo(
    () => scopeOptions.find((option) => option.value === selectedScopeKey),
    [scopeOptions, selectedScopeKey],
  );
  const selectedScope = useMemo(
    () => selectedScopeOption?.scope
      ?? (portfolioAlertScopeKey(draft.scope) === selectedScopeKey ? draft.scope : overallScope()),
    [draft.scope, selectedScopeKey, selectedScopeOption],
  );
  const displayModel = useMemo(
    () => buildPortfolioAlertDisplayModel(currentView, categories),
    [categories, currentView],
  );
  const tableRows = useMemo(() => {
    if (!editing || displayModel.rows.length > 0) return displayModel.rows;
    return buildPortfolioAlertDisplayModel(
      currentView ? { config: currentView.config, evaluation: null } : undefined,
      categories,
    ).rows;
  }, [categories, currentView, displayModel.rows, editing]);
  const validation = useMemo(() => validatePortfolioAlertDraft(draft), [draft]);
  const dirty = useMemo(
    () => draftFingerprint(draft) !== draftFingerprint(baseline),
    [baseline, draft],
  );
  const deletedScopeTransition = useMemo(
    () => decideDeletedPortfolioAlertScopeTransition(
      selectedScope,
      scopeOptions,
      dirty,
      declinedDeletedScopeKey,
    ),
    [declinedDeletedScopeKey, dirty, scopeOptions, selectedScope],
  );
  const categorySignature = useMemo(
    () => JSON.stringify(categories.map((category) => [
      category.id,
      category.name,
      category.icon,
      category.color,
      category.sort_order,
    ])),
    [categories],
  );
  const currentConfigFingerprint = configFingerprint(currentView?.config);

  selectedScopeKeyRef.current = selectedScopeKey;
  visibleRef.current = { scope: selectedScope, config: currentView?.config ?? null };

  const resetDraftForScope = useCallback((scope: PortfolioAlertScope) => {
    const latestCategories = useCategoryStore.getState().categories;
    const latestAccounts = useAccountStore.getState().accounts;
    const latestBaseCurrency = useExchangeRateStore.getState().baseCurrency;
    const nextDraft = createDraft(
      scope,
      latestCategories,
      latestAccounts,
      latestBaseCurrency,
    );
    syncedConfigRef.current = null;
    pendingDeletedConfirmationRef.current = null;
    setDeclinedDeletedScopeKey(null);
    setDraft(nextDraft);
    setBaseline(nextDraft);
    setEditing(true);
    selectScope(scope);
    void loadScope(scope);
  }, [loadScope, selectScope]);

  useEffect(() => {
    if (initializedRef.current) return;
    initializedRef.current = true;
    void Promise.all([fetchAccounts(), fetchCategories()]).finally(() => setDataReady(true));
  }, [fetchAccounts, fetchCategories]);

  useEffect(() => {
    if (!dataReady || initialScopeLoadedRef.current) return;
    initialScopeLoadedRef.current = true;
    const option = scopeOptions.find((item) => item.value === selectedScopeKey)
      ?? scopeOptions[0];
    selectScope(option.scope);
    void loadScope(option.scope);
  }, [dataReady, loadScope, scopeOptions, selectScope, selectedScopeKey]);

  useEffect(() => {
    if (!dataReady) return;
    if (deletedScopeTransition.action === "NONE") {
      pendingDeletedConfirmationRef.current = null;
      if (declinedDeletedScopeKey !== null) setDeclinedDeletedScopeKey(null);
      return;
    }
    if (deletedScopeTransition.action === "FALLBACK") {
      resetDraftForScope(deletedScopeTransition.fallbackScope);
      return;
    }
    if (deletedScopeTransition.action === "PRESERVE") return;
    if (pendingDeletedConfirmationRef.current === deletedScopeTransition.transitionKey) return;

    const { fallbackScope, transitionKey } = deletedScopeTransition;
    pendingDeletedConfirmationRef.current = transitionKey;
    Modal.confirm({
      title: "当前账户已被删除",
      content: "当前配置还有未保存修改。是否放弃修改并切换到整体组合？",
      okText: "放弃并切换",
      cancelText: "暂时保留修改",
      onOk: () => {
        pendingDeletedConfirmationRef.current = null;
        if (selectedScopeKeyRef.current === transitionKey) {
          resetDraftForScope(fallbackScope);
        }
      },
      onCancel: () => {
        pendingDeletedConfirmationRef.current = null;
        if (selectedScopeKeyRef.current === transitionKey) {
          setDeclinedDeletedScopeKey(transitionKey);
        }
      },
    });
  }, [
    dataReady,
    declinedDeletedScopeKey,
    deletedScopeTransition,
    resetDraftForScope,
  ]);

  useEffect(() => {
    if (!currentView) return;
    const syncKey = `${selectedScopeKey}:${currentConfigFingerprint}`;
    if (syncedConfigRef.current === syncKey) return;
    syncedConfigRef.current = syncKey;
    const nextDraft = createDraft(
      selectedScope,
      categories,
      accounts,
      baseCurrency,
      currentView.config,
    );
    setDraft(nextDraft);
    setBaseline(nextDraft);
    setEditing(currentView.config === null);
  }, [
    accounts,
    baseCurrency,
    categories,
    currentConfigFingerprint,
    currentView,
    selectedScope,
    selectedScopeKey,
  ]);

  useEffect(() => {
    if (categorySignatureRef.current === categorySignature) return;
    categorySignatureRef.current = categorySignature;
    setDraft((current) => mergePortfolioAlertDraftCategories(current, categories));
    setBaseline((current) => mergePortfolioAlertDraftCategories(current, categories));
  }, [categories, categorySignature]);

  useEffect(() => {
    if (pendingNotifications.length === 0) return;
    for (const breach of takePendingNotifications()) {
      const presentation = buildPortfolioAlertNotificationPresentation(breach);
      notificationApi.warning({
        message: presentation.title,
        description: presentation.description,
        placement: "topRight",
      });
    }
  }, [notificationApi, pendingNotifications, takePendingNotifications]);

  useEffect(() => {
    let disposed = false;
    let unlisten: (() => void) | null = null;
    void listen<PortfolioAlertNotification>("portfolio-alert-triggered", (event) => {
      const incoming = event.payload;
      ingestNotification(incoming);
      if (portfolioAlertScopeKey(incoming.scope) === selectedScopeKeyRef.current) {
        void loadScope(incoming.scope);
      }
    }).then((cleanup) => {
      if (disposed) cleanup();
      else unlisten = cleanup;
    });
    return () => {
      disposed = true;
      unlisten?.();
    };
  }, [ingestNotification, loadScope]);

  useEffect(() => {
    const previous = observedQuoteAtRef.current;
    observedQuoteAtRef.current = lastUpdatedAt;
    if (!lastUpdatedAt || lastUpdatedAt === previous) return;
    if (previous) {
      const previousTime = Date.parse(previous);
      const nextTime = Date.parse(lastUpdatedAt);
      if (Number.isFinite(previousTime) && Number.isFinite(nextTime) && nextTime <= previousTime) {
        return;
      }
    }
    const { config, scope } = visibleRef.current;
    if (config?.isActive) void evaluate(config.id, scope);
  }, [evaluate, lastUpdatedAt]);

  const handleScopeChange = (nextKey: string) => {
    const option = scopeOptions.find((item) => item.value === nextKey);
    if (!option || nextKey === selectedScopeKey) return;
    if (!dirty) {
      resetDraftForScope(option.scope);
      return;
    }
    Modal.confirm({
      title: "放弃未保存的修改？",
      content: "切换组合范围会丢弃当前目标和阈值修改。",
      okText: "放弃并切换",
      cancelText: "继续编辑",
      onOk: () => resetDraftForScope(option.scope),
    });
  };

  const updateTarget = (categoryId: string, targetPercent: number | null) => {
    setDraft((current) => ({
      ...current,
      targets: current.targets.map((target) =>
        target.categoryId === categoryId
          ? { ...target, targetPercent: targetPercent ?? Number.NaN }
          : target,
      ),
    }));
  };

  const handleSave = async () => {
    if (!validation.valid || !selectedScopeOption) return;
    const latestAccounts = useAccountStore.getState().accounts;
    const latestBaseCurrency = useExchangeRateStore.getState().baseCurrency;
    const saveScope = selectedScopeOption.scope;
    const input: SavePortfolioAlertConfigInput = {
      ...mergePortfolioAlertDraftCategories(draft, useCategoryStore.getState().categories),
      id: currentView?.config?.id ?? null,
      scope: saveScope,
      baseCurrency: resolvePortfolioAlertCurrency(
        saveScope,
        latestAccounts,
        latestBaseCurrency,
      ),
    };
    await saveConfig(input);
    const scopeKey = portfolioAlertScopeKey(saveScope);
    const saveError = portfolioAlertStore.getState().errorsByScope[scopeKey];
    if (saveError) {
      message.error(`保存失败：${saveError}`);
      return;
    }
    message.success("组合提醒配置已保存");
  };

  const handleActiveChange = async (checked: boolean) => {
    const config = currentView?.config;
    if (!config) {
      setDraft((current) => ({ ...current, isActive: checked }));
      return;
    }
    await setActive(config.id, selectedScope, checked);
    const activationError = portfolioAlertStore.getState().errorsByScope[selectedScopeKey];
    if (activationError) message.error(`更新失败：${activationError}`);
  };

  const handleEvaluate = async () => {
    const config = currentView?.config;
    if (!config?.isActive) return;
    await evaluate(config.id, selectedScope);
    const evaluationError = portfolioAlertStore.getState().errorsByScope[selectedScopeKey];
    if (evaluationError) message.error(`评估失败：${evaluationError}`);
  };

  const columns = [
    {
      title: "投资类别",
      dataIndex: "name",
      key: "name",
      render: (_: string, row: PortfolioAlertDisplayRow) => (
        <Space>
          <span style={{ fontSize: 20 }}>{row.icon}</span>
          <Badge color={row.color} />
          <Text>{row.name}</Text>
          {!row.editable && <Tag>虚拟类别</Tag>}
        </Space>
      ),
    },
    {
      title: "目标占比",
      key: "targetPercent",
      width: 155,
      render: (_: unknown, row: PortfolioAlertDisplayRow) => {
        const target = draft.targets.find((item) => item.categoryId === row.categoryId);
        if (!editing || !row.editable || !target) return row.targetPercentLabel;
        const fieldError = validation.targetErrors[target.categoryId];
        return (
          <div>
            <InputNumber
              min={0}
              max={100}
              precision={2}
              value={Number.isFinite(target.targetPercent) ? target.targetPercent : null}
              status={fieldError ? "error" : undefined}
              addonAfter="%"
              onChange={(value) => updateTarget(target.categoryId, value)}
              style={{ width: 130 }}
            />
            {fieldError && <div><Text type="danger">{fieldError}</Text></div>}
          </div>
        );
      },
    },
    {
      title: "当前占比",
      dataIndex: "currentPercentLabel",
      key: "currentPercent",
      width: 110,
    },
    {
      title: "相对偏离",
      dataIndex: "relativeDeviationLabel",
      key: "relativeDeviation",
      width: 110,
    },
    {
      title: `当前金额 (${displayModel.currency})`,
      dataIndex: "currentMarketValueLabel",
      key: "currentMarketValue",
      width: 160,
    },
    {
      title: `目标金额 (${displayModel.currency})`,
      dataIndex: "targetMarketValueLabel",
      key: "targetMarketValue",
      width: 160,
    },
    {
      title: `再平衡金额 (${displayModel.currency})`,
      dataIndex: "rebalanceAmountLabel",
      key: "rebalanceAmount",
      width: 175,
      render: (label: string, row: PortfolioAlertDisplayRow) => (
        <Text type={row.rebalanceAmount < 0 ? "danger" : row.rebalanceAmount > 0 ? "success" : undefined}>
          {label}
        </Text>
      ),
    },
    {
      title: "状态",
      dataIndex: "statusLabel",
      key: "status",
      width: 90,
      render: (label: string, row: PortfolioAlertDisplayRow) => (
        <Tag color={row.statusColor}>{label}</Tag>
      ),
    },
  ];

  const breachedRows = displayModel.rows.filter((row) => row.status !== "NORMAL");

  return (
    <div className="space-y-4">
      {notificationHolder}
      <Card>
        <div className="flex flex-wrap items-center justify-between gap-4">
          <Space wrap size="middle">
            <Select
              aria-label="组合范围"
              value={selectedScopeKey}
              options={scopeOptions.map(({ value, label }) => ({ value, label }))}
              onChange={handleScopeChange}
              style={{ minWidth: 200 }}
            />
            <Space>
              <Text>启用提醒</Text>
              <Switch
                checked={currentView?.config?.isActive ?? draft.isActive}
                disabled={loading || Boolean(currentView?.config && editing)}
                onChange={(checked) => void handleActiveChange(checked)}
              />
            </Space>
            <Tag color={displayModel.statusColor}>{displayModel.statusLabel}</Tag>
            {dirty && <Tag color="orange">有未保存修改</Tag>}
          </Space>
          <Space wrap>
            {currentView?.config?.isActive && (
              <Button
                icon={<ReloadOutlined />}
                loading={loading}
                onClick={() => void handleEvaluate()}
              >
                立即评估
              </Button>
            )}
            {currentView?.config && editing && (
              <Button
                onClick={() => {
                  setDraft(baseline);
                  setEditing(false);
                }}
              >
                取消编辑
              </Button>
            )}
            {currentView?.config && !editing ? (
              <Button icon={<EditOutlined />} onClick={() => setEditing(true)}>
                编辑配置
              </Button>
            ) : (
              <Button
                type="primary"
                icon={<SaveOutlined />}
                loading={loading}
                disabled={!validation.valid || !selectedScopeOption}
                onClick={() => void handleSave()}
              >
                保存配置
              </Button>
            )}
          </Space>
        </div>

        <div className="mt-5 grid grid-cols-1 gap-4 md:grid-cols-2">
          <div>
            <Text strong>统一相对偏离阈值</Text>
            <InputNumber
              min={0}
              max={100}
              precision={2}
              value={Number.isFinite(draft.deviationThreshold) ? draft.deviationThreshold : null}
              status={validation.deviationError ? "error" : undefined}
              disabled={!editing}
              addonAfter="%"
              onChange={(value) => setDraft((current) => ({
                ...current,
                deviationThreshold: value ?? Number.NaN,
              }))}
              style={{ width: "100%", marginTop: 8 }}
            />
            {validation.deviationError && <Text type="danger">{validation.deviationError}</Text>}
          </div>
          <div>
            <Text strong>单票集中度阈值</Text>
            <InputNumber
              min={0.01}
              max={100}
              precision={2}
              value={Number.isFinite(draft.concentrationThreshold) ? draft.concentrationThreshold : null}
              status={validation.concentrationError ? "error" : undefined}
              disabled={!editing}
              addonAfter="%"
              onChange={(value) => setDraft((current) => ({
                ...current,
                concentrationThreshold: value ?? Number.NaN,
              }))}
              style={{ width: "100%", marginTop: 8 }}
            />
            {validation.concentrationError && <Text type="danger">{validation.concentrationError}</Text>}
          </div>
        </div>
        <div className="mt-4 flex flex-wrap items-center gap-3">
          <Text strong>
            目标合计：
            <Text type={validation.totalError ? "danger" : "success"}>
              {Number.isFinite(validation.totalTarget)
                ? `${validation.totalTarget.toFixed(2)}%`
                : "无效"}
            </Text>
          </Text>
          <Text type="secondary">金额计算币种：{resolvePortfolioAlertCurrency(selectedScope, accounts, baseCurrency)}</Text>
          {validation.totalError && <Text type="danger">{validation.totalError}</Text>}
        </div>
      </Card>

      {error && <Alert type="error" showIcon title="组合提醒加载失败" description={error} />}
      {deletedScopeTransition.action === "PRESERVE" && (
        <Alert
          type="warning"
          showIcon
          title="当前证券账户已被删除"
          description="未保存修改仍保留在本页。你可以从范围选择器切换并确认放弃，或直接放弃修改回到整体组合。"
          action={(
            <Button
              danger
              size="small"
              onClick={() => resetDraftForScope(overallScope())}
            >
              放弃修改并切换到整体组合
            </Button>
          )}
        />
      )}
      {displayModel.banner && (
        <Alert
          type={alertType(displayModel.statusColor)}
          showIcon
          title={displayModel.statusLabel}
          description={displayModel.banner}
        />
      )}
      {displayModel.missingDataDescriptions.length > 0 && (
        <Alert
          type="warning"
          showIcon
          title="缺失数据"
          description={displayModel.missingDataDescriptions.join("；")}
        />
      )}

      <Spin spinning={loading || categoriesLoading}>
        <div className="grid grid-cols-1 gap-4 xl:grid-cols-[minmax(300px,0.8fr)_minmax(640px,1.7fr)]">
          <Card title="当前类别占比">
            {displayModel.pieData.length > 0 ? (
              <PieChart
                data={displayModel.pieData}
                height={340}
                currencyCode={displayModel.currency}
              />
            ) : (
              <Empty description="暂无可展示的组合快照" />
            )}
          </Card>
          <Card
            title="目标、当前与再平衡"
            extra={<Text type="secondary">当前合计 {displayModel.totalCurrentLabel}</Text>}
          >
            <Table<PortfolioAlertDisplayRow>
              dataSource={tableRows}
              columns={columns}
              rowKey="key"
              pagination={false}
              scroll={{ x: 1210 }}
              locale={{ emptyText: "请先在设置中创建投资类别" }}
              size="middle"
            />
          </Card>
        </div>
      </Spin>

      <Card
        title="违规与数据质量"
        extra={(
          <Tooltip title={displayModel.aiDisabledReason ?? "将配置 ID 交给可信后端生成调仓上下文"}>
            <span>
              <Button
                type="primary"
                icon={<RobotOutlined />}
                disabled={!displayModel.canAskAi}
                data-config-id={displayModel.configId ?? undefined}
                onClick={() => message.info(`AI 调仓入口已就绪：${displayModel.configId ?? ""}`)}
              >
                AI 调仓建议
              </Button>
            </span>
          </Tooltip>
        )}
      >
        {displayModel.showReadyNormalSuccess ? (
          <Alert type="success" showIcon title="当前无需再平衡" />
        ) : breachedRows.length === 0 && displayModel.concentrationRows.length === 0 ? (
          <Alert
            type={displayModel.statusColor === "success"
              ? "info"
              : alertType(displayModel.statusColor)}
            showIcon
            title={displayModel.stale ? "历史快照" : displayModel.statusLabel}
            description={displayModel.banner ?? displayModel.aiDisabledReason ?? "当前状态无法判断是否需要再平衡。"}
          />
        ) : (
          <div className="grid grid-cols-1 gap-6 lg:grid-cols-2">
            <div>
              <Title level={5}>类别偏离</Title>
              <List
                dataSource={breachedRows}
                locale={{ emptyText: "没有类别偏离" }}
                renderItem={(row) => (
                  <List.Item>
                    <Space>
                      <Tag color={row.statusColor}>{row.statusLabel}</Tag>
                      <Text>{row.icon} {row.name}</Text>
                      <Text type={row.rebalanceAmount < 0 ? "danger" : "success"}>
                        {row.rebalanceAmount < 0 ? "建议减配" : "建议增配"} {row.rebalanceAmountLabel.replace("-", "")}
                      </Text>
                    </Space>
                  </List.Item>
                )}
              />
            </div>
            <div>
              <Title level={5}>单票集中度</Title>
              <List
                dataSource={displayModel.concentrationRows}
                locale={{ emptyText: "没有单票集中度违规" }}
                renderItem={(row) => (
                  <List.Item>
                    <div>
                      <Text type="danger">{row.warning}</Text>
                      <br />
                      <Text type="secondary">市值 {row.marketValueLabel}</Text>
                    </div>
                  </List.Item>
                )}
              />
            </div>
          </div>
        )}
        <div className="mt-5 border-t border-slate-200 pt-4 dark:border-slate-700">
          <Space wrap>
            <Text type="secondary">
              最后成功评估：{formatEvaluatedAt(displayModel.snapshotEvaluatedAt)}
            </Text>
            {displayModel.stale && <Tag color="warning">历史快照，非实时</Tag>}
            <Text type="secondary">展示币种：{displayModel.currency}</Text>
          </Space>
        </div>
      </Card>
    </div>
  );
}
