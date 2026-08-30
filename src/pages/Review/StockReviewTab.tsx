import { Alert, Button, Card, Empty, Space, Spin, Typography } from "antd";
import { ReloadOutlined } from "@ant-design/icons";
import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useAccountStore } from "../../stores/accountStore";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import { useStockOperationReviewStore } from "../../stores/stockOperationReviewStore";
import type { Currency, StockOperationReviewFilters as Filters } from "../../types";
import {
  buildStockOperationReviewAiPrefill,
  loadStockOperationReviewFilters,
  saveStockOperationReviewFilters,
} from "./stockOperationReviewViewModel";
import StockOperationActionsTable from "./StockOperationActionsTable";
import StockOperationReviewQuality from "./StockOperationReviewQuality";
import StockOperationReviewSummaryCards from "./StockOperationReviewSummaryCards";
import StockOperationSecurityTable from "./StockOperationSecurityTable";
import StockReviewFilters from "./StockReviewFilters";

const { Text } = Typography;

export default function StockReviewTab() {
  const navigate = useNavigate();
  const { accounts, fetchAccounts } = useAccountStore();
  const baseCurrency = useExchangeRateStore((state) => state.baseCurrency);
  const setBaseCurrency = useExchangeRateStore((state) => state.setBaseCurrency);
  const { report, loading, error, loadReport, clearError } = useStockOperationReviewStore();
  const [filters, setFilters] = useState<Filters>(() =>
    loadStockOperationReviewFilters(localStorage, new Date(), baseCurrency),
  );

  useEffect(() => { void fetchAccounts(); }, [fetchAccounts]);
  useEffect(() => {
    if (filters.baseCurrency !== baseCurrency) {
      setFilters((current) => ({ ...current, baseCurrency }));
    }
  }, [baseCurrency, filters.baseCurrency]);
  useEffect(() => {
    saveStockOperationReviewFilters(localStorage, filters);
    void loadReport(filters);
  }, [filters, loadReport]);

  const changeFilters = (next: Filters) => {
    clearError();
    if (next.baseCurrency !== baseCurrency) setBaseCurrency(next.baseCurrency);
    setFilters(next);
  };
  const askAi = () => {
    const prefill = buildStockOperationReviewAiPrefill(filters);
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
  const currency = (report?.query.base_currency ?? filters.baseCurrency) as Currency;

  return (
    <div className="space-y-5">
      <Card>
        <StockReviewFilters
          filters={filters}
          accounts={accounts}
          loading={loading}
          canAskAi={Boolean(report)}
          onChange={changeFilters}
          onRefresh={() => void loadReport(filters)}
          onAskAi={askAi}
        />
      </Card>

      {error && report && (
        <Alert
          type="warning"
          showIcon
          closable
          onClose={clearError}
          message="最近一次刷新未完成，仍保留上一次成功报告"
          description={error}
          action={<Button size="small" icon={<ReloadOutlined />} onClick={() => void loadReport(filters)}>重试</Button>}
        />
      )}

      {error && !report ? (
        <Card>
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={
              <Space orientation="vertical">
                <Text>股票操作复盘加载失败：{error}</Text>
                <Button type="primary" icon={<ReloadOutlined />} onClick={() => void loadReport(filters)}>重试</Button>
              </Space>
            }
          />
        </Card>
      ) : loading && !report ? (
        <Card><div style={{ padding: 64, textAlign: "center" }}><Spin description="正在生成股票操作效果复盘…" /></div></Card>
      ) : report ? (
        <Space orientation="vertical" size="large" style={{ width: "100%" }}>
          <StockOperationReviewQuality quality={report.data_quality} />
          <StockOperationReviewSummaryCards summary={report.summary} currency={currency} loading={loading} />
          {report.actions.length === 0 ? (
            <Card><Empty description="所选区间没有可评价的股票买卖操作" /></Card>
          ) : (
            <>
              <StockOperationSecurityTable
                rows={report.securities}
                baseCurrency={currency}
                reportAccountId={report.query.account_id}
              />
              <StockOperationActionsTable
                actions={report.actions}
                baseCurrency={currency}
                reportAccountId={report.query.account_id}
              />
            </>
          )}
        </Space>
      ) : null}
    </div>
  );
}
