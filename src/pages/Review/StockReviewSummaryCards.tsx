import { Card, Col, Row, Space, Statistic, Tag, Typography } from "antd";
import type { Currency, MetricAvailability, StockReviewMethodology, StockReviewSummary } from "../../types";
import { formatStockReviewForwardWindowNote, formatStockReviewPercent, formatStockReviewRecovery, getRebalanceValueAddTitle, getStockReviewStatusDisplay } from "./stockReviewViewModel";

const { Text } = Typography;

function amount(value: number | null, currency: Currency) {
  return value == null || !Number.isFinite(value)
    ? "—"
    : new Intl.NumberFormat("zh-CN", { style: "currency", currency, maximumFractionDigits: 2 }).format(value);
}

function Status({ availability }: { availability: MetricAvailability }) {
  const display = getStockReviewStatusDisplay(availability.status);
  return <Tag color={display.color}>{display.label}</Tag>;
}

function MetricLine({ label, value }: { label: string; value: string }) {
  return <Space style={{ width: "100%", justifyContent: "space-between" }}><Text type="secondary">{label}</Text><Text>{value}</Text></Space>;
}

function ReviewCard({ title, availability, main, loading, children }: { title: string; availability: MetricAvailability; main: string; loading: boolean; children: React.ReactNode }) {
  return (
    <Card loading={loading} size="small" style={{ height: "100%" }} title={<Space><Text strong>{title}</Text><Status availability={availability} /></Space>}>
      <Statistic value={main} valueStyle={{ fontSize: 24 }} />
      <Space orientation="vertical" size={4} style={{ width: "100%", marginTop: 10 }}>{children}</Space>
      {availability.note && <Text type="secondary" style={{ display: "block", fontSize: 12, marginTop: 10 }}>{availability.note}</Text>}
    </Card>
  );
}

export default function StockReviewSummaryCards({ summary, methodology, currency, loading }: { summary: StockReviewSummary; methodology: StockReviewMethodology; currency: Currency; loading: boolean }) {
  const result = summary.result_quality;
  const drawdown = summary.max_drawdown;
  const valueAdd = summary.rebalance_value_add;
  const forward = summary.forward_effect;
  const risk = summary.risk_structure;
  const forward60Status = getStockReviewStatusDisplay(forward.day_60.status.status);
  const forward120Status = getStockReviewStatusDisplay(forward.day_120.status.status);
  return (
    <Row gutter={[12, 12]}>
      <Col xs={24} md={12} xl={5}>
        <ReviewCard loading={loading} title="结果质量" availability={result.availability} main={formatStockReviewPercent(result.portfolio_return)}>
          <MetricLine label="基准收益" value={formatStockReviewPercent(result.benchmark_return)} />
          <MetricLine label="超额收益" value={formatStockReviewPercent(result.excess_return)} />
        </ReviewCard>
      </Col>
      <Col xs={24} md={12} xl={5}>
        <ReviewCard loading={loading} title="最大回撤" availability={drawdown.availability} main={formatStockReviewPercent(drawdown.max_drawdown)}>
          <MetricLine label="峰值 / 谷值" value={`${drawdown.peak_date ?? "—"} / ${drawdown.trough_date ?? "—"}`} />
          <MetricLine label="持续 / 恢复" value={`${drawdown.duration_days == null ? "—" : `${drawdown.duration_days} 天`} / ${formatStockReviewRecovery(drawdown)}`} />
        </ReviewCard>
      </Col>
      <Col xs={24} md={12} xl={5}>
        <ReviewCard loading={loading} title={getRebalanceValueAddTitle(methodology.shadow_return_method)} availability={valueAdd.availability} main={formatStockReviewPercent(valueAdd.value_add)}>
          <MetricLine label="实际 / 影子" value={`${formatStockReviewPercent(valueAdd.actual_return)} / ${formatStockReviewPercent(valueAdd.shadow_return)}`} />
          <MetricLine label="期末价值差" value={amount(valueAdd.ending_value_difference_base, currency)} />
        </ReviewCard>
      </Col>
      <Col xs={24} md={12} xl={5}>
        <ReviewCard loading={loading} title="调仓后续效果" availability={forward.availability} main={formatStockReviewPercent(forward.day_60.amount_weighted_excess_return)}>
          <MetricLine label="60 日状态" value={`${forward60Status.label}${forward.day_60.status.note ? ` · ${forward.day_60.status.note}` : ""}`} />
          <MetricLine label="60 日成熟 / 观察中" value={`${forward.day_60.matured_actions} / ${forward.day_60.pending_actions}`} />
          <MetricLine label="60 日正向金额占比" value={formatStockReviewPercent(forward.day_60.positive_notional_ratio)} />
          <MetricLine label={`120 日验证 · ${forward120Status.label}`} value={`${formatStockReviewPercent(forward.day_120.amount_weighted_excess_return)}（${formatStockReviewForwardWindowNote(forward.day_120)}）`} />
        </ReviewCard>
      </Col>
      <Col xs={24} md={12} xl={4}>
        <ReviewCard loading={loading} title="风险结构" availability={risk.availability} main={formatStockReviewPercent(risk.ending_max_stock_weight)}>
          <MetricLine label="CR5 / 现金" value={`${formatStockReviewPercent(risk.ending_cr5)} / ${formatStockReviewPercent(risk.ending_cash_ratio)}`} />
          <MetricLine label="单边换手率" value={formatStockReviewPercent(risk.one_way_turnover)} />
          <MetricLine label="费用拖累" value={formatStockReviewPercent(risk.fee_drag)} />
        </ReviewCard>
      </Col>
    </Row>
  );
}
