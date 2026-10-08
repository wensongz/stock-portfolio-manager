import { Row, Col, Skeleton, Alert } from "antd";
import {
  RiseOutlined,
  FallOutlined,
  DollarOutlined,
  FundOutlined,
} from "@ant-design/icons";
import StatCard from "../../components/charts/StatCard";
import type { DashboardSummary } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";
import { formatMoney } from "../../lib/formatMoney";

interface Props {
  summary: DashboardSummary | null;
  loading: boolean;
  error: string | null;
}

export default function SummaryCards({ summary, loading, error }: Props) {
  const { pnlColor } = usePnlColor();
  if (error) {
    return (
      <Alert
        title="无法加载仪表盘数据"
        description={error}
        type="warning"
        showIcon
      />
    );
  }
  if (loading && !summary) {
    return <Skeleton active />;
  }
  if (!summary) {
    return null;
  }

  const currency = summary.base_currency;
  const pnlPositive = summary.total_pnl >= 0;
  const dailyPositive = summary.daily_pnl >= 0;

  return (
    <Row gutter={[16, 16]}>
      <Col xs={24} sm={12} md={6}>
        <StatCard
          title={`总市值 (${currency})`}
          value={formatMoney(summary.total_market_value, currency)}
          prefix={<FundOutlined />}
          valueStyle={{ fontSize: 20 }}
        />
      </Col>
      <Col xs={24} sm={12} md={6}>
        <StatCard
          title={`总成本 (${currency})`}
          value={formatMoney(summary.total_cost, currency)}
          prefix={<DollarOutlined />}
          valueStyle={{ fontSize: 20 }}
        />
      </Col>
      <Col xs={24} sm={12} md={6}>
        <StatCard
          title="总盈亏"
          value={formatMoney(summary.total_pnl, currency, 2, { signDisplay: "always" })}
          prefix={pnlPositive ? <RiseOutlined /> : <FallOutlined />}
          valueStyle={{ color: pnlColor(summary.total_pnl), fontSize: 20 }}
          change={summary.total_pnl_percent}
          changeLabel="盈亏%"
        />
      </Col>
      <Col xs={24} sm={12} md={6}>
        <StatCard
          title="今日盈亏"
          value={formatMoney(summary.daily_pnl, currency, 2, { signDisplay: "always" })}
          prefix={dailyPositive ? <RiseOutlined /> : <FallOutlined />}
          valueStyle={{ color: pnlColor(summary.daily_pnl), fontSize: 20 }}
        />
      </Col>
    </Row>
  );
}
