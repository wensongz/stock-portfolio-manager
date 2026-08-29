import { Card, Col, Descriptions, Empty, List, Row, Space, Tag, Typography } from "antd";
import type { Currency, StockActionReview, StockReviewReport } from "../../types";
import { getStockActionTypeDisplay, getStockReviewStatusDisplay } from "./stockReviewViewModel";

const { Text } = Typography;

function money(value: number | null, currency: Currency) {
  return value == null ? "—" : new Intl.NumberFormat("zh-CN", { style: "currency", currency, maximumFractionDigits: 2 }).format(value);
}

export default function RebalanceAttributionPanel({ report }: { report: StockReviewReport }) {
  const attribution = report.attribution;
  const status = getStockReviewStatusDisplay(attribution.availability.status);
  const actionTypes: StockActionReview["action_type"][] = ["open", "add", "reduce", "close"];
  return (
    <Card
      title={<Space><Text strong>调仓归因</Text><Tag color={status.color}>{status.label}</Tag></Space>}
      extra={<Text type="secondary">{attribution.percentage_basis_label}</Text>}
    >
      <Text type="secondary">以下百分比是基于平均净资产的解释性近似，不是 TWR 的精确百分点拆解。</Text>
      <Row gutter={[12, 12]} style={{ marginTop: 12 }}>
        {actionTypes.map((actionType) => {
          const actions = report.actions.filter((action) => action.action_type === actionType);
          const contributions = attribution.action_contributions.filter((item) => item.action_type === actionType);
          return (
            <Col xs={12} lg={6} key={actionType}>
              <Card size="small" title={getStockActionTypeDisplay(actionType)}>
                <Text>{actions.length} 项动作</Text>
                <div style={{ marginTop: 6 }}>
                  {contributions.length ? contributions.map((item) => (
                    <Tag key={item.action_id} style={{ marginBottom: 4 }}>{item.symbol} {money(item.amount, report.methodology.query.base_currency as Currency)}</Tag>
                  )) : <Text type="secondary">贡献明细 —</Text>}
                </div>
              </Card>
            </Col>
          );
        })}
      </Row>
      <Row gutter={[16, 16]} style={{ marginTop: 16 }}>
        <Col xs={24} lg={12}>
          <Card size="small" title="主要正贡献">
            {attribution.contributors.length ? <List size="small" dataSource={attribution.contributors} renderItem={(item) => <List.Item><Space style={{ width: "100%", justifyContent: "space-between" }}><Text>{item.symbol} · {getStockActionTypeDisplay(item.action_type)}</Text><Text>{money(item.amount, report.methodology.query.base_currency as Currency)}</Text></Space></List.Item>} /> : <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无正贡献明细" />}
          </Card>
        </Col>
        <Col xs={24} lg={12}>
          <Card size="small" title="主要机会损失 / 负贡献">
            {attribution.detractors.length ? <List size="small" dataSource={attribution.detractors} renderItem={(item) => <List.Item><Space style={{ width: "100%", justifyContent: "space-between" }}><Text>{item.symbol} · {getStockActionTypeDisplay(item.action_type)}</Text><Text>{money(item.amount, report.methodology.query.base_currency as Currency)}</Text></Space></List.Item>} /> : <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无负贡献明细" />}
          </Card>
        </Col>
      </Row>
      <Descriptions size="small" bordered column={{ xs: 1, sm: 2, lg: 4 }} style={{ marginTop: 16 }}>
        <Descriptions.Item label="分红影响">{money(attribution.dividend_contribution, report.methodology.query.base_currency as Currency)}</Descriptions.Item>
        <Descriptions.Item label="费用影响">{money(attribution.fee_contribution, report.methodology.query.base_currency as Currency)}</Descriptions.Item>
        <Descriptions.Item label="汇率影响">{money(attribution.currency_contribution, report.methodology.query.base_currency as Currency)}</Descriptions.Item>
        <Descriptions.Item label="现金影响">{money(attribution.cash_contribution, report.methodology.query.base_currency as Currency)}</Descriptions.Item>
        <Descriptions.Item label="解释金额">{money(attribution.explained_value_difference, report.methodology.query.base_currency as Currency)}</Descriptions.Item>
        <Descriptions.Item label="期末价值差">{money(attribution.ending_value_difference, report.methodology.query.base_currency as Currency)}</Descriptions.Item>
        <Descriptions.Item label="未解释残差">{money(attribution.residual, report.methodology.query.base_currency as Currency)}</Descriptions.Item>
        <Descriptions.Item label="残差 / 平均净资产">{attribution.residual_to_average_nav == null ? "—" : `${(attribution.residual_to_average_nav * 100).toFixed(3)}%`}</Descriptions.Item>
      </Descriptions>
      {attribution.availability.note && <Text type="secondary" style={{ display: "block", marginTop: 10 }}>{attribution.availability.note}</Text>}
    </Card>
  );
}
