import { useEffect } from "react";
import { Row, Col, Card, Statistic, Spin, Empty, Select } from "antd";
import PieChart from "../../components/charts/PieChart";
import { useStatisticsStore } from "../../stores/dashboardStore";
import type { MarketStatistics } from "../../types";

interface Props {
  selectedMarket: string;
  onMarketChange: (m: string) => void;
}

const markets = [
  { value: "US", label: "🇺🇸 美股" },
  { value: "CN", label: "🇨🇳 A股" },
  { value: "HK", label: "🇭🇰 港股" },
];

const marketCurrency: Record<string, { code: string; symbol: string }> = {
  US: { code: "USD", symbol: "$" },
  CN: { code: "CNY", symbol: "¥" },
  HK: { code: "HKD", symbol: "HK$" },
};

function pnlColor(pnl: number) {
  return pnl >= 0 ? "#22C55E" : "#EF4444";
}

export default function MarketTab({ selectedMarket, onMarketChange }: Props) {
  const { marketStats, fetchMarketStats } = useStatisticsStore();

  useEffect(() => {
    fetchMarketStats(selectedMarket);
  }, [selectedMarket, fetchMarketStats]);

  const stats: MarketStatistics | undefined = marketStats[selectedMarket];

  return (
    <div>
      <div className="mb-4">
        <Select
          value={selectedMarket}
          onChange={onMarketChange}
          style={{ width: 150 }}
        >
          {markets.map((m) => (
            <Select.Option key={m.value} value={m.value}>
              {m.label}
            </Select.Option>
          ))}
        </Select>
      </div>

      {!stats ? (
        <div className="flex justify-center py-16">
          <Spin size="large" />
        </div>
      ) : stats.holdings.length === 0 ? (
        <Empty description="该市场暂无持仓" />
      ) : (
        <>
          <Row gutter={[16, 16]} className="mb-4">
            <Col xs={24} sm={8}>
              <Card>
                <Statistic title={`市场总市值 (${marketCurrency[selectedMarket]?.code ?? "USD"})`} value={stats.total_market_value.toFixed(2)} prefix={marketCurrency[selectedMarket]?.symbol ?? "$"} />
              </Card>
            </Col>
            <Col xs={24} sm={8}>
              <Card>
                <Statistic title={`市场总成本 (${marketCurrency[selectedMarket]?.code ?? "USD"})`} value={stats.total_cost.toFixed(2)} prefix={marketCurrency[selectedMarket]?.symbol ?? "$"} />
              </Card>
            </Col>
            <Col xs={24} sm={8}>
              <Card>
                <Statistic
                  title={`市场总盈亏 (${marketCurrency[selectedMarket]?.code ?? "USD"})`}
                  value={`${stats.total_pnl >= 0 ? "+" : ""}${stats.total_pnl.toFixed(2)}`}
                  valueStyle={{ color: pnlColor(stats.total_pnl) }}
                  prefix={marketCurrency[selectedMarket]?.symbol ?? "$"}
                  suffix={`(${stats.total_pnl >= 0 ? "+" : ""}${stats.total_pnl_percent.toFixed(2)}%)`}
                />
              </Card>
            </Col>
          </Row>

          <Row gutter={[16, 16]}>
            {stats.account_distribution.length > 0 && (
              <Col xs={24} md={8}>
                <Card title="账户分布">
                  <PieChart data={stats.account_distribution} height={260} />
                </Card>
              </Col>
            )}
            {stats.category_distribution.length > 0 && (
              <Col xs={24} md={8}>
                <Card title="类别分布">
                  <PieChart data={stats.category_distribution} height={260} />
                </Card>
              </Col>
            )}
            {stats.stock_distribution.length > 0 && (
              <Col xs={24} md={8}>
                <Card title="个股分布">
                  <PieChart data={stats.stock_distribution} height={260} />
                </Card>
              </Col>
            )}
          </Row>
        </>
      )}
    </div>
  );
}
