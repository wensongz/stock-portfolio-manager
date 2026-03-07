import { Card, Col, Row, Statistic, Tooltip } from "antd";
import { ArrowUpOutlined, ArrowDownOutlined, InfoCircleOutlined } from "@ant-design/icons";
import type { PerformanceSummary } from "../../types";

interface Props {
  summary: PerformanceSummary | null;
  loading: boolean;
}

function colorFor(v: number) {
  return v >= 0 ? "#3f8600" : "#cf1322";
}

function PctStat({
  title,
  value,
  suffix = "%",
  tooltip,
}: {
  title: string;
  value: number;
  suffix?: string;
  tooltip?: string;
}) {
  const color = colorFor(value);
  const prefix =
    value >= 0 ? <ArrowUpOutlined /> : <ArrowDownOutlined />;
  return (
    <Statistic
      title={
        tooltip ? (
          <span>
            {title}{" "}
            <Tooltip title={tooltip}>
              <InfoCircleOutlined style={{ fontSize: 12, color: "#999" }} />
            </Tooltip>
          </span>
        ) : (
          title
        )
      }
      value={Math.abs(value)}
      precision={2}
      valueStyle={{ color }}
      prefix={prefix}
      suffix={suffix}
    />
  );
}

export default function PerformanceSummaryCards({ summary, loading }: Props) {
  return (
    <Row gutter={[16, 16]}>
      <Col xs={12} sm={8} md={6} lg={4}>
        <Card loading={loading} size="small">
          <PctStat
            title="总收益率"
            value={summary?.total_return ?? 0}
            tooltip="Time-Weighted Return（时间加权收益率）"
          />
        </Card>
      </Col>
      <Col xs={12} sm={8} md={6} lg={4}>
        <Card loading={loading} size="small">
          <PctStat
            title="年化收益率"
            value={summary?.annualized_return ?? 0}
            tooltip="基于 TWR 年化到 365 天的收益率"
          />
        </Card>
      </Col>
      <Col xs={12} sm={8} md={6} lg={4}>
        <Card loading={loading} size="small">
          <Statistic
            title="总盈亏"
            value={Math.abs(summary?.total_pnl ?? 0)}
            precision={2}
            valueStyle={{ color: colorFor(summary?.total_pnl ?? 0) }}
            prefix={(summary?.total_pnl ?? 0) >= 0 ? <ArrowUpOutlined /> : <ArrowDownOutlined />}
            suffix="USD"
          />
        </Card>
      </Col>
      <Col xs={12} sm={8} md={6} lg={4}>
        <Card loading={loading} size="small">
          <PctStat
            title="最大回撤"
            value={summary?.max_drawdown ?? 0}
            tooltip="从历史最高点到最低点的最大跌幅"
          />
        </Card>
      </Col>
      <Col xs={12} sm={8} md={6} lg={4}>
        <Card loading={loading} size="small">
          <PctStat
            title="年化波动率"
            value={summary?.volatility ?? 0}
            tooltip="日收益率标准差 × √252（年化）"
          />
        </Card>
      </Col>
      <Col xs={12} sm={8} md={6} lg={4}>
        <Card loading={loading} size="small">
          <Statistic
            title={
              <span>
                夏普比率{" "}
                <Tooltip title="（年化收益率 − 无风险利率）/ 年化波动率，无风险利率默认 4.5%">
                  <InfoCircleOutlined style={{ fontSize: 12, color: "#999" }} />
                </Tooltip>
              </span>
            }
            value={Math.abs(summary?.sharpe_ratio ?? 0)}
            precision={2}
            valueStyle={{ color: colorFor(summary?.sharpe_ratio ?? 0) }}
          />
        </Card>
      </Col>
    </Row>
  );
}
