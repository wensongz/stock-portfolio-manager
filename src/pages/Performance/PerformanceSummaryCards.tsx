import { Card, Col, Row, Statistic, Tooltip } from "antd";
import { ArrowUpOutlined, ArrowDownOutlined, InfoCircleOutlined } from "@ant-design/icons";
import type { PerformanceSummary } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";
import { formatMoney } from "../../lib/formatMoney";

interface Props {
  summary: PerformanceSummary | null;
  loading: boolean;
  currency?: string;
}

function PctStat({
  title,
  value,
  suffix = "%",
  tooltip,
  colorFor,
}: {
  title: string;
  value: number | null;
  suffix?: string;
  tooltip?: string;
  colorFor: (v: number) => string;
}) {
  const color = value == null ? undefined : colorFor(value);
  const prefix = value == null ? undefined
    : value >= 0 ? <ArrowUpOutlined /> : <ArrowDownOutlined />;
  return (
    <Statistic
      title={
        tooltip ? (
          <span>
            {title}{" "}
            <Tooltip title={tooltip}>
              <InfoCircleOutlined style={{ fontSize: 12, color: "var(--color-text-tertiary)" }} />
            </Tooltip>
          </span>
        ) : (
          title
        )
      }
      value={value == null ? "—" : Math.abs(value)}
      precision={2}
      styles={{ content: { color, fontSize: 20 } }}
      prefix={prefix}
      suffix={value == null ? undefined : suffix}
    />
  );
}

export default function PerformanceSummaryCards({ summary, loading, currency = "CNY" }: Props) {
  const { pnlColorDark } = usePnlColor();
  const sharpe = summary?.sharpe_ratio ?? null;
  return (
    <Row gutter={[16, 16]}>
      <Col xs={12} sm={8} md={6} lg={5}>
        <Card loading={loading} size="small">
          <Statistic
            title="总盈亏"
            value={Math.abs(summary?.total_pnl ?? 0)}
            precision={2}
            styles={{ content: { color: pnlColorDark(summary?.total_pnl ?? 0), fontSize: 20 } }}
            prefix={(summary?.total_pnl ?? 0) >= 0 ? <ArrowUpOutlined /> : <ArrowDownOutlined />}
            formatter={(value) => formatMoney(Number(value), currency)}
          />
        </Card>
      </Col>
      <Col xs={12} sm={8} md={6} lg={4}>
        <Card loading={loading} size="small">
          <PctStat
            title="总收益率"
            value={summary?.total_return ?? 0}
            tooltip="Time-Weighted Return（时间加权收益率）"
            colorFor={pnlColorDark}
          />
        </Card>
      </Col>
      <Col xs={12} sm={8} md={6} lg={4}>
        <Card loading={loading} size="small">
          <PctStat
            title="年化收益率"
            value={summary?.annualized_return ?? null}
            tooltip="基于 TWR 年化到 365 天的收益率"
            colorFor={pnlColorDark}
          />
        </Card>
      </Col>
      <Col xs={12} sm={8} md={6} lg={4}>
        <Card loading={loading} size="small">
          <PctStat
            title="最大回撤"
            value={summary?.max_drawdown ?? 0}
            tooltip="从历史最高点到最低点的最大跌幅"
            colorFor={pnlColorDark}
          />
        </Card>
      </Col>
      <Col xs={12} sm={8} md={6} lg={4}>
        <Card loading={loading} size="small">
          <PctStat
            title="年化波动率"
            value={summary?.volatility ?? 0}
            tooltip="日收益率标准差 × √252（年化）"
            colorFor={pnlColorDark}
          />
        </Card>
      </Col>
      <Col xs={12} sm={8} md={6} lg={3}>
        <Card loading={loading} size="small">
          <Statistic
            title={
              <span>
                夏普比率{" "}
                <Tooltip title="日均超额收益 / 日收益标准差 × √252，无风险利率固定假设为 4.5%">
                  <InfoCircleOutlined style={{ fontSize: 12, color: "var(--color-text-tertiary)" }} />
                </Tooltip>
              </span>
            }
            value={sharpe ?? "—"}
            precision={2}
            styles={{ content: { color: sharpe == null ? undefined : pnlColorDark(sharpe), fontSize: 20 } }}
          />
        </Card>
      </Col>
    </Row>
  );
}
