import { useCallback, useEffect } from "react";
import { Typography, Select, Divider, Card, Row, Col, Statistic, Spin, Button, Tooltip } from "antd";
import { ReloadOutlined, SyncOutlined, DashboardOutlined } from "@ant-design/icons";
import { useDashboardStore } from "../../stores/dashboardStore";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import { useQuoteStore } from "../../stores/quoteStore";
import type { Currency } from "../../types";
import SummaryCards from "./SummaryCards";
import HoldingsTable from "./HoldingsTable";
import QuickCharts from "./QuickCharts";
import { refreshDashboardQuotes } from "./dashboardRefresh";
import dayjs from "dayjs";

const { Title, Text } = Typography;

export default function DashboardPage() {
  const { summary, holdingDetails, loading, error, fetchReport } = useDashboardStore();
  const { baseCurrency, setBaseCurrency } = useExchangeRateStore();
  const { loading: quotesLoading, lastUpdatedAt, fetchHoldingQuotes } = useQuoteStore();
  const rates = summary?.exchange_rates ?? null;

  useEffect(() => {
    void fetchReport(useExchangeRateStore.getState().baseCurrency);
  }, [fetchReport]);

  const handleCurrencyChange = (currency: Currency) => {
    setBaseCurrency(currency);
    fetchReport(currency);
  };

  const handleRefreshQuotes = useCallback(async () => {
    await refreshDashboardQuotes({
      fetchHoldingQuotes,
      fetchReport,
      getBaseCurrency: () => useExchangeRateStore.getState().baseCurrency,
    });
  }, [fetchHoldingQuotes, fetchReport]);

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <Title level={2} className="!mb-0">
          <DashboardOutlined style={{ color: "#13c2c2" }} /> 仪表盘
        </Title>
        <div className="flex items-center gap-2">
          <Tooltip title={lastUpdatedAt ? `上次更新: ${dayjs(lastUpdatedAt).format("HH:mm:ss")}` : "点击刷新行情"}>
            <Button
              icon={quotesLoading ? <SyncOutlined spin /> : <ReloadOutlined />}
              onClick={handleRefreshQuotes}
              size="small"
              disabled={quotesLoading}
            >
              刷新行情
            </Button>
          </Tooltip>
          <Text type="secondary">基准货币:</Text>
          <Select
            value={baseCurrency}
            onChange={handleCurrencyChange}
            size="small"
            style={{ width: 120 }}
          >
            <Select.Option value="USD">USD 美元</Select.Option>
            <Select.Option value="CNY">CNY 人民币</Select.Option>
            <Select.Option value="HKD">HKD 港元</Select.Option>
          </Select>
        </div>
      </div>

      {/* Summary Cards */}
      <SummaryCards summary={summary} loading={loading} error={error} />

      {/* Quick market distribution chart */}
      <QuickCharts summary={summary} />

      {/* Exchange Rates Card */}
      <Row gutter={[16, 16]} className="mt-4">
        <Col span={24}>
          <Card
            title="💱 实时汇率"
            extra={
              loading ? (
                <Spin size="small" />
              ) : rates ? (
                <Text type="secondary" style={{ fontSize: 12 }}>
                  更新于 {dayjs(rates.updated_at).format("YYYY-MM-DD HH:mm")}
                </Text>
              ) : null
            }
          >
            {rates ? (
              <Row gutter={[32, 0]}>
                <Col>
                  <Statistic title="USD / CNY" value={rates.usd_cny.toFixed(4)} />
                </Col>
                <Col>
                  <Statistic title="USD / HKD" value={rates.usd_hkd.toFixed(4)} />
                </Col>
                <Col>
                  <Statistic title="CNY / HKD" value={rates.cny_hkd.toFixed(4)} />
                </Col>
                <Col>
                  <Statistic title="CNY / USD" value={(1 / rates.usd_cny).toFixed(4)} />
                </Col>
                <Col>
                  <Statistic title="HKD / USD" value={(1 / rates.usd_hkd).toFixed(4)} />
                </Col>
                <Col>
                  <Statistic title="HKD / CNY" value={(rates.usd_cny / rates.usd_hkd).toFixed(4)} />
                </Col>
              </Row>
            ) : (
              !loading && <Text type="secondary">暂无汇率数据</Text>
            )}
          </Card>
        </Col>
      </Row>

      <Divider>持仓概览</Divider>

      {/* Holdings Detail Table */}
      <HoldingsTable holdings={holdingDetails} loading={loading} />
    </div>
  );
}
