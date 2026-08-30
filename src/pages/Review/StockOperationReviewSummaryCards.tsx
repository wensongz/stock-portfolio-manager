import { Card, Col, Row, Space, Statistic, Typography } from "antd";
import type { Currency, StockOperationReviewSummary } from "../../types";
import { buildStockOperationSummaryCards } from "./stockOperationReviewViewModel";

const { Text } = Typography;

export default function StockOperationReviewSummaryCards({
  summary,
  currency,
  loading,
}: {
  summary: StockOperationReviewSummary;
  currency: Currency;
  loading: boolean;
}) {
  const cards = buildStockOperationSummaryCards(summary, currency);
  return (
    <Row gutter={[12, 12]}>
      {cards.map((card) => (
        <Col key={card.title} xs={24} md={12} xl={6}>
          <Card loading={loading} size="small" style={{ height: "100%" }} title={card.title}>
            <Statistic value={card.primary} valueStyle={{ fontSize: 24 }} />
            <Space orientation="vertical" size={5} style={{ width: "100%", marginTop: 10 }}>
              {card.metrics.map((metric) => (
                <Space key={metric.label} style={{ width: "100%", justifyContent: "space-between" }}>
                  <Text type="secondary">{metric.label}</Text>
                  <Text>{metric.value}</Text>
                </Space>
              ))}
            </Space>
            <Text type="secondary" style={{ display: "block", fontSize: 12, marginTop: 10 }}>
              {card.description}
            </Text>
          </Card>
        </Col>
      ))}
    </Row>
  );
}
