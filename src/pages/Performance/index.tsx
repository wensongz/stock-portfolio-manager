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

const MARKET_CURRENCY: Record<Market, string> = {
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

  // Derive currency from the selected account or market filter, falling
  // back to the dashboard's base currency setting (not hardcoded "USD").
  const currency = useMemo(() => {
    if (selectedAccountId) {
      const account = accounts.find((a) => a.id === selectedAccountId);
      if (account) return MARKET_CURRENCY[account.market] ?? baseCurrency;
    }
    if (selectedMarket) {
      return MARKET_CURRENCY[selectedMarket as Market] ?? baseCurrency;
    }
    return baseCurrency;
  }, [selectedAccountId, selectedMarket, accounts, baseCurrency]);

  // Convert each holding's PnL to the display currency, then rank.
  const { topGainers, topLosers } = useMemo(() => {
    const converted = holdingPerformances.map((h) => {
      const fromCurrency = (selectedMarket || selectedAccountId
        ? MARKET_CURRENCY[h.market as Market] ?? "USD"
        : "USD") as Currency;
      const convertedPnl = convertWithCachedRates(h.pnl, fromCurrency, currency as Currency);
      return { ...h, pnl: convertedPnl };
    });
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
  }, [holdingPerformances, selectedMarket, selectedAccountId, convertWithCachedRates, currency, rates]);

  // Portfolio-wide values are persisted in USD; filtered values remain in the
  // selected market/account currency. Convert monetary summary fields before
  // adding the display-currency suffix.
  const displaySummary = useMemo(() => {
    if (!summary) return null;
    const sourceCurrency = (selectedMarket || selectedAccountId ? currency : "USD") as Currency;
    const targetCurrency = currency as Currency;
    return {
      ...summary,
      start_value: convertWithCachedRates(summary.start_value, sourceCurrency, targetCurrency),
      end_value: convertWithCachedRates(summary.end_value, sourceCurrency, targetCurrency),
      total_pnl: convertWithCachedRates(summary.total_pnl, sourceCurrency, targetCurrency),
    };
  }, [summary, selectedMarket, selectedAccountId, currency, convertWithCachedRates, rates]);

  // Unfiltered attribution is normalised to USD by the backend before market
  // and category aggregation. Filtered attribution stays in that market's
  // native currency. Convert the final amounts to the selected display unit.
  const displayAttribution = useMemo(() => {
    if (!attribution) return null;
    const sourceCurrency = (selectedMarket || selectedAccountId ? currency : "USD") as Currency;
    const targetCurrency = currency as Currency;
    const convertItems = (items: typeof attribution.by_market) =>
      items.map((item) => ({
        ...item,
        pnl: convertWithCachedRates(item.pnl, sourceCurrency, targetCurrency),
      }));
    return {
      ...attribution,
      total_pnl: convertWithCachedRates(attribution.total_pnl, sourceCurrency, targetCurrency),
      by_market: convertItems(attribution.by_market),
      by_category: convertItems(attribution.by_category),
      by_holding: convertItems(attribution.by_holding),
    };
  }, [attribution, selectedMarket, selectedAccountId, currency, convertWithCachedRates, rates]);

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
        <AttributionChart attribution={displayAttribution} height={300} currency={currency} />
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
        <RankingChart gainers={topGainers} losers={topLosers} height={360} currency={currency} />
      </Card>
    </div>
  );
}
