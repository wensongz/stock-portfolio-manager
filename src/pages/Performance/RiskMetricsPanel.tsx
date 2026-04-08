import { Card, Col, Row, Statistic, Tooltip } from "antd";
import { InfoCircleOutlined } from "@ant-design/icons";
import type { RiskMetrics } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";

interface Props {
  metrics: RiskMetrics | null;
  loading: boolean;
}

export default function RiskMetricsPanel({ metrics, loading }: Props) {
  const { pnlColorDark, lossColor } = usePnlColor();
  return (
    <div>
      <Row gutter={[12, 12]}>
        <Col xs={12} sm={8} md={4}>
          <Card loading={loading} size="small">
            <Statistic
              title={
                <span>
                  日波动率{" "}
                  <Tooltip title="日收益率标准差">
                    <InfoCircleOutlined style={{ fontSize: 11, color: "#999" }} />
                  </Tooltip>
                </span>
              }
              value={metrics?.daily_volatility ?? 0}
              precision={3}
              suffix="%"
              valueStyle={{ fontSize: 12 }}
            />
          </Card>
        </Col>
        <Col xs={12} sm={8} md={4}>
          <Card loading={loading} size="small">
            <Statistic
              title={
                <span>
                  年化波动率{" "}
                  <Tooltip title="日波动率 × √252">
                    <InfoCircleOutlined style={{ fontSize: 11, color: "#999" }} />
                  </Tooltip>
                </span>
              }
              value={metrics?.annualized_volatility ?? 0}
              precision={2}
              suffix="%"
              valueStyle={{ fontSize: 12 }}
            />
          </Card>
        </Col>
        <Col xs={12} sm={8} md={4}>
          <Card loading={loading} size="small">
            <Statistic
              title={
                <span>
                  夏普比率{" "}
                  <Tooltip title={`(年化收益 − 无风险利率 ${metrics?.risk_free_rate?.toFixed(1) ?? 4.5}%) / 年化波动率`}>
                    <InfoCircleOutlined style={{ fontSize: 11, color: "#999" }} />
                  </Tooltip>
                </span>
              }
              value={metrics?.sharpe_ratio ?? 0}
              precision={2}
              valueStyle={{
                fontSize: 12,
                color: (metrics?.sharpe_ratio ?? 0) >= 1 ? pnlColorDark(1) : (metrics?.sharpe_ratio ?? 0) >= 0 ? "#d46b08" : pnlColorDark(-1),
              }}
            />
          </Card>
        </Col>
        <Col xs={12} sm={8} md={4}>
          <Card loading={loading} size="small">
            <Statistic
              title="最大回撤"
              value={Math.abs(metrics?.max_drawdown ?? 0)}
              precision={2}
              suffix="%"
              valueStyle={{ fontSize: 12, color: lossColor }}
            />
          </Card>
        </Col>
        <Col xs={12} sm={8} md={4}>
          <Card loading={loading} size="small">
            <Statistic
              title={
                <span>
                  卡玛比率{" "}
                  <Tooltip title="年化收益率 / 最大回撤（绝对值）">
                    <InfoCircleOutlined style={{ fontSize: 11, color: "#999" }} />
                  </Tooltip>
                </span>
              }
              value={metrics?.calmar_ratio ?? 0}
              precision={2}
              valueStyle={{
                fontSize: 12,
                color: (metrics?.calmar_ratio ?? 0) >= 1 ? pnlColorDark(1) : "#d46b08",
              }}
            />
          </Card>
        </Col>
        <Col xs={12} sm={8} md={4}>
          <Card loading={loading} size="small">
            <Statistic
              title="无风险利率"
              value={metrics?.risk_free_rate ?? 4.5}
              precision={1}
              suffix="%"
              valueStyle={{ fontSize: 12 }}
            />
          </Card>
        </Col>
      </Row>
    </div>
  );
}
