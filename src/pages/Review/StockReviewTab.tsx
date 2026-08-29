import { Alert, Button, Card, Empty, Space, Spin, Typography } from "antd";
import { ReloadOutlined } from "@ant-design/icons";
import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useAccountStore } from "../../stores/accountStore";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import { stockReviewReportIdentity, useStockReviewStore } from "../../stores/stockReviewStore";
import type { Currency, StockCampaignDetail, StockReviewFilters as Filters, StockReviewOverrideInput } from "../../types";
import {
  buildStockCampaignAiPrefill,
  buildStockReviewAiPrefill,
  buildStockReviewReportFilters,
  buildStockReviewTransactionCandidates,
  getStockReviewPageState,
  loadStockReviewFilters,
  saveStockReviewFilters,
} from "./stockReviewViewModel";
import LegacyStockReviewPanel from "./LegacyStockReviewPanel";
import PortfolioComparisonChart from "./PortfolioComparisonChart";
import RebalanceAttributionPanel from "./RebalanceAttributionPanel";
import RiskStructurePanel from "./RiskStructurePanel";
import StockActionsTable from "./StockActionsTable";
import StockCampaignDrawer from "./StockCampaignDrawer";
import StockReviewDataQuality from "./StockReviewDataQuality";
import StockReviewFilters from "./StockReviewFilters";
import StockReviewSummaryCards from "./StockReviewSummaryCards";

const { Text } = Typography;

export default function StockReviewTab() {
  const navigate = useNavigate();
  const { accounts, fetchAccounts } = useAccountStore();
  const baseCurrency = useExchangeRateStore((state) => state.baseCurrency);
  const setBaseCurrency = useExchangeRateStore((state) => state.setBaseCurrency);
  const {
    report,
    reportLoading,
    campaignLoading,
    mutating,
    selectedCampaign,
    error,
    loadReport,
    loadCampaignDetail,
    saveAnnotation,
    confirmOverride,
    clearSelectedCampaign,
    clearError,
  } = useStockReviewStore();
  const [filters, setFilters] = useState<Filters>(() =>
    loadStockReviewFilters(localStorage, new Date(), baseCurrency),
  );

  useEffect(() => { void fetchAccounts(); }, [fetchAccounts]);
  useEffect(() => {
    if (filters.baseCurrency !== baseCurrency) {
      setFilters((current) => ({ ...current, baseCurrency }));
    }
  }, [baseCurrency, filters.baseCurrency]);
  useEffect(() => {
    saveStockReviewFilters(localStorage, filters);
    void loadReport(filters);
  }, [filters, loadReport]);

  const pageState = getStockReviewPageState(report, error);
  const reportFilters = useMemo<Filters | null>(
    () => report ? buildStockReviewReportFilters(report, filters.periodPreset) : null,
    [report, filters.periodPreset],
  );
  const transactionCandidates = useMemo(
    () => report ? buildStockReviewTransactionCandidates(report) : [],
    [report],
  );

  const changeFilters = (next: Filters) => {
    clearError();
    if (next.baseCurrency !== baseCurrency) setBaseCurrency(next.baseCurrency);
    setFilters(next);
  };
  const openCampaign = (campaignId: string) => {
    if (reportFilters) void loadCampaignDetail(reportFilters, campaignId);
  };
  const navigateWithPrefill = (prefill: ReturnType<typeof buildStockReviewAiPrefill>) => {
    navigate("/ai-assistant", {
      state: {
        prefillPrompt: prefill.prompt,
        prefillActiveSkill: prefill.activeSkill,
        prefillAutoSend: prefill.autoSend,
        prefillToolName: prefill.toolName,
        prefillToolArguments: prefill.toolArguments,
      },
    });
  };
  const askPortfolioAi = () => {
    if (reportFilters) navigateWithPrefill(buildStockReviewAiPrefill(reportFilters));
  };
  const askCampaignAi = (detail: StockCampaignDetail) => {
    if (reportFilters) {
      navigateWithPrefill(
        buildStockCampaignAiPrefill(
          reportFilters,
          detail.summary.symbol,
          detail.summary.campaign_id,
        ),
      );
    }
  };
  const campaignMutationContext = report && selectedCampaign ? {
    campaignId: selectedCampaign.summary.campaign_id,
    reportIdentity: stockReviewReportIdentity(report),
  } : null;
  const saveCampaignAnnotation = (input: Parameters<typeof saveAnnotation>[0]) =>
    campaignMutationContext
      ? saveAnnotation(input, campaignMutationContext)
      : Promise.resolve(null);
  const applyOverride = (input: StockReviewOverrideInput) =>
    reportFilters && campaignMutationContext
      ? confirmOverride(reportFilters, input, campaignMutationContext)
      : Promise.resolve(null);

  return (
    <div className="space-y-5">
      <Card>
        <StockReviewFilters
          filters={filters}
          accounts={accounts}
          loading={reportLoading}
          canAskAi={Boolean(reportFilters)}
          onChange={changeFilters}
          onRefresh={() => void loadReport(filters)}
          onAskAi={askPortfolioAi}
        />
      </Card>

      {error && report && (
        <Alert
          type="warning"
          showIcon
          closable
          onClose={clearError}
          message="最近一次操作未完成，仍保留上一次成功报告"
          description={error}
          action={<Button size="small" icon={<ReloadOutlined />} onClick={() => void loadReport(filters)}>重试</Button>}
        />
      )}

      {pageState.kind === "error" ? (
        <Card>
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={
              <Space orientation="vertical">
                <Text>股票复盘报告加载失败：{error}</Text>
                <Button type="primary" icon={<ReloadOutlined />} onClick={() => void loadReport(filters)}>重试</Button>
              </Space>
            }
          />
        </Card>
      ) : reportLoading && !report ? (
        <Card><div style={{ padding: 64, textAlign: "center" }}><Spin description="正在加载持久化筛选对应的组合复盘…" /></div></Card>
      ) : report ? (
        <Space orientation="vertical" size="large" style={{ width: "100%" }}>
          <StockReviewDataQuality report={report} />
          <StockReviewSummaryCards summary={report.summary} methodology={report.methodology} currency={report.methodology.query.base_currency as Currency} loading={reportLoading} />
          <PortfolioComparisonChart report={report} onOpenCampaign={openCampaign} />
          <RebalanceAttributionPanel report={report} />
          <RiskStructurePanel report={report} />
          {report.campaigns.length === 0 && (
            <Card><Empty description="所选区间没有 Campaign；报告摘要、归因、风险与动作仍按后端结果分别展示。" /></Card>
          )}
          <StockActionsTable actions={report.actions} campaigns={report.campaigns} baseCurrency={report.methodology.query.base_currency as Currency} onOpenCampaign={openCampaign} />
        </Space>
      ) : null}

      <LegacyStockReviewPanel />
      <StockCampaignDrawer
        open={campaignLoading || Boolean(selectedCampaign)}
        loading={campaignLoading}
        mutating={mutating}
        detail={selectedCampaign}
        currency={(report?.methodology.query.base_currency ?? filters.baseCurrency) as Currency}
        reportEndDate={report?.methodology.query.end_date ?? filters.endDate}
        transactionCandidates={transactionCandidates}
        onClose={clearSelectedCampaign}
        onAskAi={askCampaignAi}
        onSaveAnnotation={saveCampaignAnnotation}
        onConfirmOverride={applyOverride}
      />
    </div>
  );
}
