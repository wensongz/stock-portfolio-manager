import { Alert, Button, Card, Empty, Space, Spin, Typography } from "antd";
import { ReloadOutlined } from "@ant-design/icons";
import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useAccountStore } from "../../stores/accountStore";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import { useStockReviewStore } from "../../stores/stockReviewStore";
import type { Currency, Market, StockCampaignDetail, StockReviewFilters as Filters, StockReviewOverrideInput } from "../../types";
import {
  buildStockCampaignAiPrefill,
  buildStockReviewAiPrefill,
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
  const reportFilters = useMemo<Filters | null>(() => report ? {
    accountId: report.methodology.query.account_id,
    periodPreset: filters.periodPreset,
    startDate: report.methodology.query.start_date,
    endDate: report.methodology.query.end_date,
    market: report.methodology.query.market as Market | null,
    benchmarkSymbol: report.methodology.query.benchmark_symbol,
    baseCurrency: report.methodology.query.base_currency as Currency,
  } : null, [report, filters.periodPreset]);

  const changeFilters = (next: Filters) => {
    clearError();
    if (next.baseCurrency !== baseCurrency) setBaseCurrency(next.baseCurrency);
    setFilters(next);
  };
  const openCampaign = (campaignId: string) => void loadCampaignDetail(filters, campaignId);
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
  const applyOverride = (input: StockReviewOverrideInput) => confirmOverride(filters, input);

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
      ) : report && pageState.kind === "empty" ? (
        <Space orientation="vertical" size="middle" style={{ width: "100%" }}>
          <StockReviewDataQuality report={report} />
          <Card><Empty description="所选范围没有可展示的持仓曲线、调仓动作或 Campaign。无需填写表单；可调整筛选或刷新数据。" /></Card>
        </Space>
      ) : report ? (
        <Space orientation="vertical" size="large" style={{ width: "100%" }}>
          <StockReviewDataQuality report={report} />
          <StockReviewSummaryCards summary={report.summary} currency={report.methodology.query.base_currency as Currency} loading={reportLoading} />
          <PortfolioComparisonChart report={report} onOpenCampaign={openCampaign} />
          <RebalanceAttributionPanel report={report} />
          <RiskStructurePanel report={report} />
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
        onClose={clearSelectedCampaign}
        onAskAi={askCampaignAi}
        onSaveAnnotation={saveAnnotation}
        onConfirmOverride={applyOverride}
      />
    </div>
  );
}
