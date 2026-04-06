import { useEffect } from "react";
import { Button, Card, Col, Divider, Row, Select, Space, Typography } from "antd";
import { ReloadOutlined } from "@ant-design/icons";
import { usePerformanceStore } from "../../stores/performanceStore";
import { useAccountStore } from "../../stores/accountStore";
import TimeRangeSelector from "./TimeRangeSelector";
import PerformanceSummaryCards from "./PerformanceSummaryCards";
import ReturnChart from "./ReturnChart";
import DrawdownChart from "./DrawdownChart";
import AttributionChart from "./AttributionChart";
import MonthlyReturnsTable from "./MonthlyReturnsTable";
import RankingChart from "./RankingChart";
import RiskMetricsPanel from "./RiskMetricsPanel";

const { Title } = Typography;

const MARKETS = [
  { value: "US", label: "🇺🇸 美股" },
  { value: "CN", label: "🇨🇳 A股" },
  { value: "HK", label: "🇭🇰 港股" },
];

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
    drawdown,
    attribution,
    monthlyReturns,
    topGainers,
    topLosers,
    riskMetrics,
    loading,
    setTimeRange,
    setBenchmarks,
    setMarket,
    setAccountId,
    fetchBenchmark,
  } = usePerformanceStore();

  const { accounts, fetchAccounts } = useAccountStore();

  useEffect(() => {
    fetchAccounts();
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
    setMarket(value ?? null);
    setTimeout(() => usePerformanceStore.getState().fetchAll(), 0);
  };

  const handleAccountChange = (value: string | undefined) => {
    setAccountId(value ?? null);
    setTimeout(() => usePerformanceStore.getState().fetchAll(), 0);
  };

  return (
    <div>
      {/* Header */}
      <div className="flex justify-between items-center mb-2">
        <Title level={2} className="!mb-0">
          📊 绩效分析
        </Title>
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
      </div>
      <div className="flex justify-end items-center mb-4">
        <Space wrap>
          <TimeRangeSelector
            timeRange={timeRange}
            customStart={customStart}
            customEnd={customEnd}
            onChange={handleTimeRangeChange}
          />
          <Button
            icon={<ReloadOutlined />}
            onClick={() => usePerformanceStore.getState().fetchAll()}
            loading={loading}
            size="small"
          >
            刷新
          </Button>
        </Space>
      </div>

      {/* Summary cards */}
      <PerformanceSummaryCards summary={summary} loading={loading} />

      <Divider />

      {/* Return chart + Drawdown chart */}
      <Row gutter={[16, 16]}>
        <Col xs={24} lg={16}>
          <Card size="small">
            <ReturnChart
              returnSeries={returnSeries}
              benchmarkSeries={benchmarkSeries}
              selectedBenchmarks={selectedBenchmarks}
              onBenchmarkChange={handleBenchmarkChange}
            />
          </Card>
        </Col>
        <Col xs={24} lg={8}>
          <Card size="small">
            <DrawdownChart drawdown={drawdown} height={320} />
          </Card>
        </Col>
      </Row>

      <Divider />

      {/* Attribution chart */}
      <Card size="small">
        <AttributionChart attribution={attribution} height={300} />
      </Card>

      <Divider />

      {/* Monthly returns + Risk metrics */}
      <Row gutter={[16, 16]}>
        <Col xs={24} xl={16}>
          <Card size="small">
            <MonthlyReturnsTable data={monthlyReturns} />
          </Card>
        </Col>
        <Col xs={24} xl={8}>
          <Card size="small">
            <Typography.Text strong>⚠️ 风险指标</Typography.Text>
            <div className="mt-2">
              <RiskMetricsPanel metrics={riskMetrics} loading={loading} />
            </div>
          </Card>
        </Col>
      </Row>

      <Divider />

      {/* Ranking */}
      <Card size="small">
        <RankingChart gainers={topGainers} losers={topLosers} height={360} />
      </Card>
    </div>
  );
}
