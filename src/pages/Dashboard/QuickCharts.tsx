import { Row, Col, Card } from "antd";
import PieChart from "../../components/charts/PieChart";
import type { DashboardSummary } from "../../types";

interface Props {
  summary: DashboardSummary | null;
}

export default function QuickCharts({ summary }: Props) {
  if (!summary) return null;

  const marketData = [
    { name: "🇺🇸 美股", value: parseFloat(summary.us_market_value.toFixed(2)) },
    { name: "🇨🇳 A股", value: parseFloat(summary.cn_market_value.toFixed(2)) },
    { name: "🇭🇰 港股", value: parseFloat(summary.hk_market_value.toFixed(2)) },
  ].filter((d) => d.value > 0);

  if (marketData.length === 0) return null;

  const total = summary.total_market_value.toFixed(0);
  const currency = summary.base_currency;
  const currencySymbol: Record<string, string> = { USD: "$", CNY: "¥", HKD: "HK$" };

  return (
    <Row gutter={[16, 16]} className="mt-4">
      <Col xs={24} md={12}>
        <Card title="市场分布" size="small">
          <PieChart
            data={marketData}
            height={280}
            currencyCode={currency}
            centerText={`${currencySymbol[currency] ?? ""}${Number(total).toLocaleString()}`}
          />
        </Card>
      </Col>
    </Row>
  );
}
