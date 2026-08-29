import { Card, Collapse, Descriptions, List, Space, Tag, Typography } from "antd";
import type { StockReviewReport } from "../../types";
import { formatStockReviewPercent, getStockReviewStatusDisplay } from "./stockReviewViewModel";

const { Text } = Typography;

export default function RiskStructurePanel({ report }: { report: StockReviewReport }) {
  const risk = report.risk_structure;
  const status = getStockReviewStatusDisplay(risk.availability.status);
  const concentrationStatus = getStockReviewStatusDisplay(risk.concentration_availability.status);
  const turnoverStatus = getStockReviewStatusDisplay(risk.turnover_availability.status);
  const feeStatus = getStockReviewStatusDisplay(risk.fee_availability.status);
  return (
    <Card title={<Space><Text strong>风险结构变化</Text><Tag color={status.color}>{status.label}</Tag></Space>}>
      <Text type="secondary">最大单股权重、CR5 与 HHI 均以股票资产为分母；现金比例单独展示。</Text>
      <Space wrap style={{ marginTop: 10 }}>
        <Tag color={concentrationStatus.color}>集中度：{concentrationStatus.label}</Tag>
        <Tag color={turnoverStatus.color}>换手：{turnoverStatus.label}</Tag>
        <Tag color={feeStatus.color}>费用：{feeStatus.label}</Tag>
      </Space>
      <Descriptions bordered size="small" column={{ xs: 1, sm: 2, lg: 3 }} style={{ marginTop: 12 }}>
        <Descriptions.Item label="最大单股权重（期初 → 期末）">{formatStockReviewPercent(risk.opening.max_stock_weight)} → {formatStockReviewPercent(risk.ending.max_stock_weight)}</Descriptions.Item>
        <Descriptions.Item label="CR5（期初 → 期末）">{formatStockReviewPercent(risk.opening.cr5)} → {formatStockReviewPercent(risk.ending.cr5)}</Descriptions.Item>
        <Descriptions.Item label="现金比例（期初 → 期末）">{formatStockReviewPercent(risk.opening.cash_ratio)} → {formatStockReviewPercent(risk.ending.cash_ratio)}</Descriptions.Item>
        <Descriptions.Item label="单边换手率">{formatStockReviewPercent(risk.one_way_turnover)}</Descriptions.Item>
        <Descriptions.Item label="费用拖累">{formatStockReviewPercent(risk.fee_drag)}</Descriptions.Item>
        <Descriptions.Item label="期间峰值单股权重">{formatStockReviewPercent(risk.peak.max_stock_weight)}</Descriptions.Item>
      </Descriptions>
      <Collapse
        ghost
        style={{ marginTop: 8 }}
        items={[{
          key: "risk-detail",
          label: "展开 HHI、权重与数据提示",
          children: (
            <Space orientation="vertical" size="middle" style={{ width: "100%" }}>
              <Descriptions size="small" column={{ xs: 1, sm: 3 }}>
                <Descriptions.Item label="期初 HHI">{risk.opening.hhi ?? "—"}</Descriptions.Item>
                <Descriptions.Item label="期末 HHI">{risk.ending.hhi ?? "—"}</Descriptions.Item>
                <Descriptions.Item label="峰值 HHI">{risk.peak.hhi ?? "—"}</Descriptions.Item>
              </Descriptions>
              <Space wrap>{risk.fact_labels.map((label) => <Tag key={label}>{label}</Tag>)}</Space>
              {risk.top_position_weights.length > 0 && <List size="small" header="期末主要持仓权重" dataSource={risk.top_position_weights} renderItem={(item) => <List.Item><Space style={{ width: "100%", justifyContent: "space-between" }}><Text>{item.key}</Text><Text>{formatStockReviewPercent(item.weight)}</Text></Space></List.Item>} />}
              {risk.data_hints.map((hint) => <Text type="secondary" key={hint}>{hint}</Text>)}
            </Space>
          ),
        }]}
      />
    </Card>
  );
}
