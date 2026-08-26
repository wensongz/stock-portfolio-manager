import { useEffect, useMemo, useState } from "react";
import {
  Alert,
  Button,
  Card,
  Col,
  Empty,
  Row,
  Select,
  Space,
  Spin,
  Statistic,
  Table,
  Tag,
  Typography,
} from "antd";
import type { ColumnsType } from "antd/es/table";
import { useNavigate } from "react-router-dom";
import { usePnlColor } from "../../hooks/usePnlColor";
import { useAccountStore } from "../../stores/accountStore";
import { useChatStore } from "../../stores/chatStore";
import { useOptionReviewStore } from "../../stores/optionReviewStore";
import type {
  Currency,
  OptionCampaign,
  OptionReviewDataQuality,
  OptionUnderlyingReview,
} from "../../types";
import {
  OPTION_REVIEW_ANNUALIZED_YIELD_LABEL,
  OPTION_REVIEW_NET_PREMIUM_LABEL,
  buildOptionReviewPrompt,
  formatReviewPercent,
  getOptionReviewEmptyDescription,
  loadOptionReviewPeriodDays,
  saveOptionReviewPeriodDays,
  selectDefaultUnderlying,
  shouldShowNetPremium,
  sortUnderlyingReviews,
} from "./optionReviewViewModel";

const { Text } = Typography;

const attentionFlags = new Set(["净亏损", "低留存", "单次损失较大", "样本不足"]);

function formatCurrency(value: number, currency: Currency) {
  return new Intl.NumberFormat("zh-CN", {
    style: "currency",
    currency,
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  }).format(value);
}

function describeDataQuality(dataQuality: OptionReviewDataQuality) {
  const descriptions = [
    "每条卖出开仓记录生成一个Campaign",
    "进行中Campaign计入累计净权利金，但不计入留存率、年化收益率和最差Campaign",
  ];
  if (dataQuality.unmatched_records > 0) {
    descriptions.push(`${dataQuality.unmatched_records}条记录未匹配`);
  }
  if (dataQuality.missing_trade_dates > 0) {
    descriptions.push(`${dataQuality.missing_trade_dates}条记录缺少交易日期`);
  }
  return descriptions.join("；");
}

export default function OptionReviewTab() {
  const navigate = useNavigate();
  const { accounts, loading: accountsLoading, fetchAccounts } = useAccountStore();
  const setActiveSkillsForNextTurn = useChatStore(
    (state) => state.setActiveSkillsForNextTurn,
  );
  const { report, loading, error, fetchOptionReview, clearOptionReview } =
    useOptionReviewStore();
  const [accountId, setAccountId] = useState(
    () => localStorage.getItem("review_option_account_id") ?? "",
  );
  const [periodDays, setPeriodDays] = useState<number | null>(() =>
    loadOptionReviewPeriodDays(localStorage),
  );
  const [selectedSymbol, setSelectedSymbol] = useState<string | null>(null);
  const [accountsReady, setAccountsReady] = useState(false);
  const { pnlColorDark, pnlTag } = usePnlColor();
  const selectedAccount = accounts.find((account) => account.id === accountId) ?? null;

  useEffect(() => {
    void fetchAccounts().finally(() => setAccountsReady(true));
  }, [fetchAccounts]);

  useEffect(() => {
    if (!accountId || !accountsReady) return;
    if (!accounts.some((account) => account.id === accountId)) {
      localStorage.removeItem("review_option_account_id");
      setAccountId("");
    }
  }, [accountId, accounts, accountsReady]);

  useEffect(() => {
    if (!accountId) {
      clearOptionReview();
      return;
    }
    localStorage.setItem("review_option_account_id", accountId);
    void fetchOptionReview(accountId, periodDays);
  }, [accountId, periodDays, fetchOptionReview, clearOptionReview]);

  useEffect(() => {
    setSelectedSymbol(selectDefaultUnderlying(report));
  }, [report]);

  const sortedUnderlyings = useMemo(
    () => sortUnderlyingReviews(report?.underlyings ?? []),
    [report],
  );
  const selectedUnderlying = useMemo(
    () => report?.underlyings.find((item) => item.underlying === selectedSymbol) ?? null,
    [report, selectedSymbol],
  );
  const selectedCampaigns = useMemo(
    () =>
      [...(selectedUnderlying?.campaigns ?? [])].sort(
        (left, right) =>
          right.started_at.localeCompare(left.started_at) || left.id.localeCompare(right.id),
      ),
    [selectedUnderlying],
  );

  const handleAiReview = () => {
    if (!selectedUnderlying || !selectedAccount) return;
    setActiveSkillsForNextTurn(["options-review"]);
    navigate("/ai-assistant", {
      state: {
        prefillPrompt: buildOptionReviewPrompt({
          accountId: selectedAccount.id,
          accountName: selectedAccount.name,
          symbol: selectedUnderlying.underlying,
          periodDays,
        }),
      },
    });
  };

  const currency = report?.currency ?? "USD";

  const renderPnl = (value: number) => (
    <Text style={{ color: pnlColorDark(value) }}>{formatCurrency(value, currency)}</Text>
  );
  const renderPercent = (value: number | null) =>
    value == null ? (
      "—"
    ) : (
      <Text style={{ color: pnlColorDark(value) }}>{formatReviewPercent(value)}</Text>
    );

  const underlyingColumns: ColumnsType<OptionUnderlyingReview> = [
    { title: "标的", dataIndex: "underlying", width: 90 },
    {
      title: "Campaign",
      width: 160,
      render: (_: unknown, row: OptionUnderlyingReview) =>
        `${row.completed_campaigns} 完成 / ${row.active_campaigns} 进行中`,
    },
    {
      title: OPTION_REVIEW_NET_PREMIUM_LABEL,
      dataIndex: "net_premium_pnl",
      align: "right",
      width: 160,
      render: (value: number) => renderPnl(value),
    },
    {
      title: "留存率",
      dataIndex: "retention_rate",
      align: "right",
      width: 80,
      render: (value: number | null, row) =>
        row.completed_campaigns > 0 ? renderPercent(value) : "—",
    },
    {
      title: OPTION_REVIEW_ANNUALIZED_YIELD_LABEL,
      dataIndex: "annualized_yield_on_notional",
      align: "right",
      width: 140,
      render: (value: number | null, row) =>
        row.completed_campaigns > 0 ? renderPercent(value) : "—",
    },
    {
      title: "最差Campaign",
      dataIndex: "worst_campaign_pnl",
      align: "right",
      width: 150,
      render: (value: number | null, row) =>
        row.completed_campaigns > 0 && value != null ? renderPnl(value) : "—",
    },
    {
      title: "事实标签",
      dataIndex: "flags",
      width: 190,
      render: (flags: string[]) => (
        <Space size={[4, 4]} wrap>
          {flags.map((flag) => {
            let color = "default";
            if (attentionFlags.has(flag)) color = "orange";
            else if (flag === "高留存") color = pnlTag(1);
            return (
              <Tag key={flag} color={color}>
                {flag}
              </Tag>
            );
          })}
        </Space>
      ),
    },
  ];

  const campaignColumns: ColumnsType<OptionCampaign> = [
    {
      title: "期权标识",
      dataIndex: "option_symbol",
      width: 175,
    },
    {
      title: "合约数",
      dataIndex: "contracts",
      align: "right",
      width: 80,
    },
    {
      title: "状态",
      dataIndex: "status",
      width: 60,
      render: (status: OptionCampaign["status"]) =>
        status === "active" ? <Tag color="orange">进行中</Tag> : <Tag>已完成</Tag>,
    },
    {
      title: "毛权利金",
      dataIndex: "gross_premium",
      align: "right",
      width: 100,
      render: (value: number) => formatCurrency(value, currency),
    },
    {
      title: "买回成本",
      dataIndex: "close_cost",
      align: "right",
      width: 100,
      render: (value: number) => formatCurrency(value, currency),
    },
    {
      title: "费用",
      dataIndex: "fees",
      align: "right",
      width: 100,
      render: (value: number) => formatCurrency(value, currency),
    },
    {
      title: "净权利金（含进行中）",
      dataIndex: "net_premium_pnl",
      align: "right",
      width: 140,
      render: (value: number | null) => (value == null ? "—" : renderPnl(value)),
    },
    {
      title: "留存率",
      dataIndex: "retention_rate",
      align: "right",
      width: 80,
      render: (value: number | null, campaign) =>
        campaign.status === "active" ? "—" : renderPercent(value),
    },
    {
      title: OPTION_REVIEW_ANNUALIZED_YIELD_LABEL,
      dataIndex: "annualized_yield_on_notional",
      align: "right",
      width: 150,
      render: (value: number | null, campaign) =>
        campaign.status === "active" ? "—" : renderPercent(value),
    },
  ];

  const hasCompletedCampaigns = (report?.summary.completed_campaigns ?? 0) > 0;
  const hasNetPremium = report ? shouldShowNetPremium(report.summary) : false;
  const dataQualityNotice = report ? (
    <Alert
      type="info"
      showIcon
      title="数据质量说明"
      description={describeDataQuality(report.data_quality)}
    />
  ) : null;

  return (
    <div className="space-y-4" style={{ minWidth: 0 }}>
      <style>{`
        .option-review-selected-row > td {
          background: color-mix(in srgb, var(--color-info) 12%, transparent) !important;
        }
      `}</style>
      <Space wrap>
        <Select
          aria-label="期权复盘账户"
          value={accountId || undefined}
          placeholder="选择账户"
          loading={accountsLoading}
          onChange={setAccountId}
          options={accounts.map((account) => ({ value: account.id, label: account.name }))}
          style={{ minWidth: 220 }}
        />
        <Select
          aria-label="期权复盘周期"
          value={periodDays == null ? "all" : String(periodDays)}
          onChange={(value) => {
            const nextPeriodDays = value === "all" ? null : Number(value);
            saveOptionReviewPeriodDays(localStorage, nextPeriodDays);
            setPeriodDays(nextPeriodDays);
          }}
          options={[
            { value: "365", label: "最近365天" },
            { value: "730", label: "最近730天" },
            { value: "all", label: "全部历史" },
          ]}
          style={{ minWidth: 140 }}
        />
      </Space>

      {error ? <Alert type="error" showIcon title={error} /> : null}
      {accountsReady && accounts.length === 0 ? <Empty description="请先创建账户" /> : null}
      {(!accountsReady || accounts.length > 0) && !accountId ? (
        <Empty description="请先选择账户" />
      ) : null}
      {accountId && loading ? <Spin /> : null}
      {accountId && !loading && report && report.underlyings.length === 0 ? (
        <>
          {dataQualityNotice}
          <Empty description={getOptionReviewEmptyDescription(report.data_quality)} />
        </>
      ) : null}

      {accountId && !loading && report && report.underlyings.length > 0 ? (
        <>
          <Row gutter={[16, 16]}>
            <Col xs={24} sm={12} xl={8}>
              <Card>
                <Statistic
                  title={OPTION_REVIEW_NET_PREMIUM_LABEL}
                  value={
                    hasNetPremium
                      ? formatCurrency(report.summary.net_premium_pnl, report.currency)
                      : "—"
                  }
                  styles={{
                    content: {
                      color: hasNetPremium
                        ? pnlColorDark(report.summary.net_premium_pnl)
                        : undefined,
                    },
                  }}
                />
              </Card>
            </Col>
            <Col xs={24} sm={12} xl={8}>
              <Card>
                <Statistic
                  title="留存率"
                  value={
                    hasCompletedCampaigns
                      ? formatReviewPercent(report.summary.retention_rate)
                      : "—"
                  }
                  styles={{
                    content: {
                      color:
                        hasCompletedCampaigns && report.summary.retention_rate != null
                          ? pnlColorDark(report.summary.retention_rate)
                          : undefined,
                    },
                  }}
                />
              </Card>
            </Col>
            <Col xs={24} sm={12} xl={8}>
              <Card>
                <Statistic
                  title={OPTION_REVIEW_ANNUALIZED_YIELD_LABEL}
                  value={
                    hasCompletedCampaigns
                      ? formatReviewPercent(report.summary.annualized_yield_on_notional)
                      : "—"
                  }
                  styles={{
                    content: {
                      color:
                        hasCompletedCampaigns &&
                        report.summary.annualized_yield_on_notional != null
                          ? pnlColorDark(report.summary.annualized_yield_on_notional)
                          : undefined,
                    },
                  }}
                />
              </Card>
            </Col>
          </Row>

          {dataQualityNotice}

          <Card title="个股汇总" style={{ overflow: "hidden" }}>
            <Table<OptionUnderlyingReview>
              rowKey="underlying"
              columns={underlyingColumns}
              dataSource={sortedUnderlyings}
              pagination={false}
              scroll={{ x: 980 }}
              rowClassName={(row) =>
                row.underlying === selectedSymbol ? "option-review-selected-row" : ""
              }
              onRow={(row) => ({
                onClick: () => setSelectedSymbol(row.underlying),
                style: { cursor: "pointer" },
              })}
            />
          </Card>

          {selectedUnderlying ? (
            <Card
              title={`${selectedUnderlying.underlying} Campaign详情`}
              extra={
                <Button onClick={handleAiReview}>AI复盘这只股票</Button>
              }
              style={{ overflow: "hidden" }}
            >
              <Table<OptionCampaign>
                rowKey="id"
                columns={campaignColumns}
                dataSource={selectedCampaigns}
                pagination={false}
                scroll={{ x: 1060 }}
              />
            </Card>
          ) : null}
        </>
      ) : null}
    </div>
  );
}
