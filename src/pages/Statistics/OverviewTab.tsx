import { Row, Col, Card, Statistic, Spin, Empty } from "antd";
import PieChart from "../../components/charts/PieChart";
import BarChart from "../../components/charts/BarChart";
import type { StatisticsOverview } from "../../types";

interface Props {
  overview: StatisticsOverview | null;
  loading: boolean;
}

function pnlColor(pnl: number) {
  return pnl >= 0 ? "#22C55E" : "#EF4444";
}

export default function OverviewTab({ overview, loading }: Props) {
  if (loading && !overview) {
    return (
      <div className="flex justify-center py-16">
        <Spin size="large" />
      </div>
    );
  }
  if (!overview) {
    return <Empty description="暂无数据" />;
  }

  const totalPnlPos = overview.total_pnl >= 0;

  const gainersData = overview.top_gainers.map((g) => ({
    name: g.symbol,
    value: parseFloat(g.pnl.toFixed(2)),
  }));
  const losersData = overview.top_losers.map((g) => ({
    name: g.symbol,
    value: parseFloat(g.pnl.toFixed(2)),
  }));

  return (
    <div>
      {/* Summary stats */}
      <Row gutter={[16, 16]} className="mb-4">
        <Col xs={24} sm={8}>
          <Card>
            <Statistic
              title="总市值 (USD)"
              value={overview.total_market_value.toFixed(2)}
              prefix="$"
            />
          </Card>
        </Col>
        <Col xs={24} sm={8}>
          <Card>
            <Statistic
              title="总成本 (USD)"
              value={overview.total_cost.toFixed(2)}
              prefix="$"
            />
          </Card>
        </Col>
        <Col xs={24} sm={8}>
          <Card>
            <Statistic
              title="总盈亏 (USD)"
              value={`${totalPnlPos ? "+" : ""}${overview.total_pnl.toFixed(2)}`}
              valueStyle={{ color: pnlColor(overview.total_pnl) }}
              prefix="$"
              suffix={`(${totalPnlPos ? "+" : ""}${overview.total_pnl_percent.toFixed(2)}%)`}
            />
          </Card>
        </Col>
      </Row>

      {/* Distribution charts */}
      <Row gutter={[16, 16]}>
        <Col xs={24} md={8}>
          <Card title="市场分布">
            <PieChart data={overview.market_distribution} height={260} />
          </Card>
        </Col>
        <Col xs={24} md={8}>
          <Card title="类别分布">
            <PieChart data={overview.category_distribution} height={260} />
          </Card>
        </Col>
        <Col xs={24} md={8}>
          <Card title="账户分布">
            <PieChart data={overview.account_distribution} height={260} />
          </Card>
        </Col>
      </Row>

      {/* Stock distribution chart */}
      {overview.stock_distribution.length > 0 && (
        <Row gutter={[16, 16]} className="mt-4">
          <Col xs={24}>
            <Card title="个股分布">
              <PieChart data={overview.stock_distribution} height={360} />
            </Card>
          </Col>
        </Row>
      )}

      {/* PnL charts */}
      {(gainersData.length > 0 || losersData.length > 0) && (
        <Row gutter={[16, 16]} className="mt-4">
          <Col xs={24} md={12}>
            <Card title="盈利 Top 5">
              <BarChart data={gainersData} colorByValue height={220} />
            </Card>
          </Col>
          <Col xs={24} md={12}>
            <Card title="亏损 Top 5">
              <BarChart data={losersData} colorByValue height={220} />
            </Card>
          </Col>
        </Row>
      )}
    </div>
  );
}
