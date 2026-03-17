import { Row, Col, Skeleton, Alert } from "antd";
import {
  RiseOutlined,
  FallOutlined,
  DollarOutlined,
  FundOutlined,
} from "@ant-design/icons";
import StatCard from "../../components/charts/StatCard";
import type { DashboardSummary } from "../../types";

interface Props {
  summary: DashboardSummary | null;
  loading: boolean;
  error: string | null;
}

const currencySymbol: Record<string, string> = { USD: "$", CNY: "¥", HKD: "HK$" };

function fmt(value: number, currency: string) {
  return `${currencySymbol[currency] ?? ""}${value.toLocaleString("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`;
}

export default function SummaryCards({ summary, loading, error }: Props) {
  if (error) {
    return (
      <Alert
        message="无法加载仪表盘数据"
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
          value={fmt(summary.total_market_value, currency)}
          prefix={<FundOutlined />}
          valueStyle={{ fontSize: 20 }}
        />
      </Col>
      <Col xs={24} sm={12} md={6}>
        <StatCard
          title={`总成本 (${currency})`}
          value={fmt(summary.total_cost, currency)}
          prefix={<DollarOutlined />}
          valueStyle={{ fontSize: 20 }}
        />
      </Col>
      <Col xs={24} sm={12} md={6}>
        <StatCard
          title="总盈亏"
          value={`${pnlPositive ? "+" : ""}${fmt(summary.total_pnl, currency)}`}
          prefix={pnlPositive ? <RiseOutlined /> : <FallOutlined />}
          valueStyle={{ color: pnlPositive ? "#22C55E" : "#EF4444", fontSize: 20 }}
          change={summary.total_pnl_percent}
          changeLabel="盈亏%"
        />
      </Col>
      <Col xs={24} sm={12} md={6}>
        <StatCard
          title="今日盈亏"
          value={`${dailyPositive ? "+" : ""}${fmt(summary.daily_pnl, currency)}`}
          prefix={dailyPositive ? <RiseOutlined /> : <FallOutlined />}
          valueStyle={{ color: dailyPositive ? "#22C55E" : "#EF4444", fontSize: 20 }}
        />
      </Col>
    </Row>
  );
}
