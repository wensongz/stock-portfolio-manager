import { useEffect, useMemo } from "react";
import { Alert, Button, Card, Col, Divider, Row, Select, Space, Typography } from "antd";
import { ReloadOutlined, LineChartOutlined } from "@ant-design/icons";
import { usePerformanceStore } from "../../stores/performanceStore";
import { useAccountStore } from "../../stores/accountStore";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import TimeRangeSelector from "./TimeRangeSelector";
import PerformanceSummaryCards from "./PerformanceSummaryCards";
import ReturnChart from "./ReturnChart";
import DrawdownChart from "./DrawdownChart";
import AttributionChart from "./AttributionChart";
import MonthlyReturnsTable from "./MonthlyReturnsTable";
import RankingChart from "./RankingChart";
import RiskMetricsPanel from "./RiskMetricsPanel";
import type { Currency, Market } from "../../types";

const { Title } = Typography;

const MARKETS = [
  { value: "US", label: "🇺🇸 美股" },
  { value: "CN", label: "🇨🇳 A股" },
  { value: "HK", label: "🇭🇰 港股" },
];

const MARKET_CURRENCY: Record<Market, Currency> = {
  US: "USD",
  CN: "CNY",
  HK: "HKD",
};

export default function PerformancePage() {
  const {
    timeRange,
    customStart,
    customEnd,
    selectedBenchmarks,
    selectedMarket,
    selectedAccountId,
    summary,
    returnSeries,
    benchmarkSeries,
    benchmarkErrors,
    drawdown,
    attribution,
    monthlyReturns,
    holdingPerformances,
    riskMetrics,
    loading,
    error,
    setTimeRange,
    setBenchmarks,
    setMarket,
    setAccountId,
    fetchBenchmark,
  } = usePerformanceStore();

  const { accounts, fetchAccounts } = useAccountStore();
  const { baseCurrency, convertWithCachedRates, rates, fetchRates } = useExchangeRateStore();

  const sourceCurrency = summary?.valuation_currency
    ?? MARKET_CURRENCY[(selectedMarket ?? accounts.find((a) => a.id === selectedAccountId)?.market ?? "US") as Market]
    ?? "USD";
  const canConvert = rates != null && [rates.usd_cny, rates.usd_hkd].every(
    (rate) => Number.isFinite(rate) && rate > 0,
  );
  const currency = selectedMarket || selectedAccountId || !canConvert ? sourceCurrency : baseCurrency;

  // Convert each holding's PnL to the display currency, then rank.
  const { topGainers, topLosers } = useMemo(() => {
    const converted = holdingPerformances.map((holding) => ({
      ...holding,
      pnl: convertWithCachedRates(holding.pnl, sourceCurrency, currency),
    }));
    return {
      topGainers: converted
        .filter((h) => h.pnl >= 0)
        .sort((a, b) => b.pnl - a.pnl)
        .slice(0, 10),
      topLosers: converted
        .filter((h) => h.pnl < 0)
        .sort((a, b) => a.pnl - b.pnl)
        .slice(0, 10),
    };
  }, [holdingPerformances, sourceCurrency, currency, convertWithCachedRates, rates]);

  const displaySummary = useMemo(() => {
    if (!summary) return null;
    return {
      ...summary,
      start_value: convertWithCachedRates(summary.start_value, sourceCurrency, currency),
      end_value: convertWithCachedRates(summary.end_value, sourceCurrency, currency),
      total_pnl: convertWithCachedRates(summary.total_pnl, sourceCurrency, currency),
    };
  }, [summary, sourceCurrency, currency, convertWithCachedRates, rates]);

  const displayAttribution = useMemo(() => {
    if (!attribution) return null;
    const convertItems = (items: typeof attribution.by_market) =>
      items.map((item) => ({
        ...item,
        pnl: convertWithCachedRates(item.pnl, sourceCurrency, currency),
      }));
    return {
      ...attribution,
      total_pnl: convertWithCachedRates(attribution.total_pnl, sourceCurrency, currency),
      by_market: convertItems(attribution.by_market),
      by_category: convertItems(attribution.by_category),
      by_holding: convertItems(attribution.by_holding),
    };
  }, [attribution, sourceCurrency, currency, convertWithCachedRates, rates]);

  useEffect(() => {
    fetchAccounts();
    if (!rates) fetchRates();
    // fetchAll is stable from the Zustand store - use getState() to avoid stale closure
    usePerformanceStore.getState().fetchAll();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  const handleTimeRangeChange = (range: typeof timeRange, start?: string, end?: string) => {
    setTimeRange(range, start, end);
    usePerformanceStore.getState().fetchAll();
  };

  const handleBenchmarkChange = (symbols: string[]) => {
    const prev = selectedBenchmarks;
    setBenchmarks(symbols);
    // Fetch newly added benchmarks
    for (const sym of symbols) {
      if (!prev.includes(sym)) {
        fetchBenchmark(sym);
      }
    }
  };

  const handleMarketChange = (value: string | undefined) => {
    void setMarket(value ?? null);
  };

  const handleAccountChange = (value: string | undefined) => {
    void setAccountId(value ?? null);
  };

  return (
    <div>
      {/* Header */}
      <div className="flex justify-between items-start mb-4">
        <Title level={2} className="!mb-0">
          <LineChartOutlined style={{ color: "#52c41a" }} /> 绩效分析
        </Title>
        <div className="flex flex-col items-end gap-1">
          <Space>
            <Select
              value={selectedMarket ?? undefined}
              onChange={handleMarketChange}
              placeholder="按市场"
              allowClear
              style={{ width: 130 }}
              size="small"
            >
              {MARKETS.map((m) => (
                <Select.Option key={m.value} value={m.value}>
                  {m.label}
                </Select.Option>
              ))}
            </Select>
            <Select
              value={selectedAccountId ?? undefined}
              onChange={handleAccountChange}
              placeholder="按账户"
              allowClear
              style={{ width: 180 }}
              size="small"
            >
              {accounts.map((a) => (
                <Select.Option key={a.id} value={a.id}>
                  {a.name} ({a.market})
                </Select.Option>
              ))}
            </Select>
          </Space>
          <Space wrap>
            <TimeRangeSelector
              timeRange={timeRange}
              customStart={customStart}
              customEnd={customEnd}
              onChange={handleTimeRangeChange}
            />
            <Button
              icon={<ReloadOutlined />}
              onClick={() => usePerformanceStore.getState().fetchAll(true)}
              loading={loading}
              size="small"
            >
              刷新
            </Button>
          </Space>
        </div>
      </div>

      {error && (
        <Alert
          type="error"
          title="绩效数据刷新失败"
          description={summary ? `当前仍显示上次成功加载的数据。${error}` : error}
          showIcon
          className="mb-4"
        />
      )}

      {/* Summary cards */}
      <PerformanceSummaryCards summary={displaySummary} loading={loading} currency={currency} />

      <Divider />

      {/* Return chart + Drawdown chart */}
      <Row gutter={[16, 16]}>
        <Col xs={24} lg={15}>
          <Card size="small">
            <ReturnChart
              returnSeries={returnSeries}
              benchmarkSeries={benchmarkSeries}
              benchmarkErrors={benchmarkErrors}
              startDate={summary?.start_date}
              selectedBenchmarks={selectedBenchmarks}
              onBenchmarkChange={handleBenchmarkChange}
            />
          </Card>
        </Col>
        <Col xs={24} lg={9}>
          <Card size="small">
            <DrawdownChart drawdown={drawdown} height={320} />
          </Card>
        </Col>
      </Row>

      <Divider />

      {/* Attribution chart */}
      <Card size="small">
        <AttributionChart attribution={displayAttribution} height={300} currency={currency} />
      </Card>

      <Divider />

      {/* Monthly returns + Risk metrics */}
      <Card size="small">
        <MonthlyReturnsTable data={monthlyReturns} />
      </Card>
      <Card size="small">
        <Typography.Text strong>⚠️ 风险指标</Typography.Text>
        <div className="mt-2">
          <RiskMetricsPanel metrics={riskMetrics} loading={loading} />
        </div>
      </Card>

      <Divider />

      {/* Ranking */}
      <Card size="small">
        <RankingChart gainers={topGainers} losers={topLosers} height={360} currency={currency} />
      </Card>
    </div>
  );
}
