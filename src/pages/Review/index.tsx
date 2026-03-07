import { useEffect, useState } from "react";
import {
  Card,
  Select,
  Space,
  Typography,
  Tag,
  Timeline,
  Statistic,
  Row,
  Col,
  Empty,
  Progress,
} from "antd";
import {
  CheckCircleOutlined,
  CloseCircleOutlined,
  HistoryOutlined,
} from "@ant-design/icons";
import { useReviewStore } from "../../stores/reviewStore";
import type { QuarterlyHoldingStatus } from "../../types";

const { Title, Text } = Typography;

function DecisionQualityTag({
  quality,
  snapshotId,
  symbol,
}: {
  quality: string | null;
  snapshotId: string;
  symbol: string;
}) {
  const { updateDecisionQuality } = useReviewStore();

  const handleChange = (v: string) => {
    updateDecisionQuality(snapshotId, symbol, v);
  };

  return (
    <Select
      size="small"
      allowClear
      placeholder="设置决策质量"
      value={quality || undefined}
      onChange={handleChange}
      style={{ width: 140 }}
      options={[
        { value: "correct", label: "✅ 正确决策" },
        { value: "wrong", label: "❌ 错误决策" },
        { value: "pending", label: "⚠️ 待定" },
      ]}
    />
  );
}

function HoldingTimeline({
  timeline,
  symbol,
}: {
  timeline: QuarterlyHoldingStatus[];
  symbol: string;
}) {
  if (!timeline.length) return <Empty description="暂无季度记录" />;

  return (
    <Timeline
      items={timeline.map((item) => ({
        dot:
          item.pnl_percent >= 0 ? (
            <CheckCircleOutlined style={{ color: "#52c41a" }} />
          ) : (
            <CloseCircleOutlined style={{ color: "#ff4d4f" }} />
          ),
        children: (
          <Card size="small" style={{ marginBottom: 8 }}>
            <Space direction="vertical" style={{ width: "100%" }}>
              <Space>
                <Tag color="blue">{item.quarter}</Tag>
                <Text>持仓：{item.shares} 股</Text>
                <Text>均价：{item.avg_cost.toFixed(2)}</Text>
                <Text>现价：{item.close_price.toFixed(2)}</Text>
                <Tag color={item.pnl_percent >= 0 ? "green" : "red"}>
                  {item.pnl_percent >= 0 ? "+" : ""}
                  {item.pnl_percent.toFixed(2)}%
                </Tag>
              </Space>
              {item.notes && (
                <Text type="secondary" style={{ fontSize: 12 }}>
                  思考：{item.notes}
                </Text>
              )}
              <DecisionQualityTag
                quality={item.decision_quality}
                snapshotId={item.snapshot_id}
                symbol={symbol}
              />
            </Space>
          </Card>
        ),
      }))}
    />
  );
}

export default function ReviewPage() {
  const {
    reviewedSymbols,
    currentReview,
    decisionStats,
    loading,
    fetchReviewedSymbols,
    fetchHoldingReview,
    fetchDecisionStatistics,
  } = useReviewStore();

  const [selectedSymbol, setSelectedSymbol] = useState<string | null>(null);

  useEffect(() => {
    fetchReviewedSymbols();
    fetchDecisionStatistics();
  }, [fetchReviewedSymbols, fetchDecisionStatistics]);

  const handleSelectSymbol = (symbol: string) => {
    setSelectedSymbol(symbol);
    fetchHoldingReview(symbol);
  };

  return (
    <div className="space-y-6">
      <Title level={2}>
        <HistoryOutlined /> 历史操作复盘
      </Title>

      {/* Decision Statistics */}
      {decisionStats && (
        <Card title="决策统计">
          <Row gutter={16}>
            <Col span={6}>
              <Statistic
                title="总决策数"
                value={decisionStats.total_decisions}
              />
            </Col>
            <Col span={6}>
              <Statistic
                title="正确决策"
                value={decisionStats.correct_count}
                valueStyle={{ color: "#52c41a" }}
              />
            </Col>
            <Col span={6}>
              <Statistic
                title="错误决策"
                value={decisionStats.wrong_count}
                valueStyle={{ color: "#ff4d4f" }}
              />
            </Col>
            <Col span={6}>
              <Statistic
                title="决策准确率"
                value={(decisionStats.accuracy_rate * 100).toFixed(1)}
                suffix="%"
                valueStyle={{
                  color:
                    decisionStats.accuracy_rate >= 0.6 ? "#52c41a" : "#ff4d4f",
                }}
              />
            </Col>
          </Row>
          {decisionStats.total_decisions > 0 && (
            <Progress
              percent={Math.round(decisionStats.accuracy_rate * 100)}
              style={{ marginTop: 16 }}
            />
          )}
        </Card>
      )}

      <Row gutter={16}>
        {/* Symbol Selector */}
        <Col span={6}>
          <Card title="选择股票">
            <Select
              showSearch
              placeholder="搜索或选择股票"
              style={{ width: "100%", marginBottom: 16 }}
              value={selectedSymbol || undefined}
              onChange={handleSelectSymbol}
              optionFilterProp="label"
              options={reviewedSymbols.map(([sym, name]) => ({
                value: sym,
                label: `${sym} ${name}`,
              }))}
            />
            {reviewedSymbols.map(([sym, name, market]) => (
              <div
                key={sym}
                className="cursor-pointer p-2 rounded hover:bg-gray-100"
                style={{
                  background: selectedSymbol === sym ? "#e6f7ff" : undefined,
                  borderLeft:
                    selectedSymbol === sym ? "3px solid #1890ff" : "3px solid transparent",
                }}
                onClick={() => handleSelectSymbol(sym)}
              >
                <Space>
                  <Text strong>{sym}</Text>
                  <Text type="secondary" style={{ fontSize: 12 }}>
                    {name}
                  </Text>
                  <Tag color="default" style={{ fontSize: 10 }}>
                    {market}
                  </Tag>
                </Space>
              </div>
            ))}
            {!reviewedSymbols.length && (
              <Empty
                description="暂无复盘数据，请先创建季度快照"
                image={Empty.PRESENTED_IMAGE_SIMPLE}
              />
            )}
          </Card>
        </Col>

        {/* Review Timeline */}
        <Col span={18}>
          {currentReview ? (
            <Card
              title={
                <Space>
                  <Text strong>{currentReview.symbol}</Text>
                  <Text>{currentReview.name}</Text>
                  <Tag color="blue">{currentReview.market}</Tag>
                  {currentReview.is_current_holding ? (
                    <Tag color="green">持仓中</Tag>
                  ) : (
                    <Tag color="default">已清仓</Tag>
                  )}
                </Space>
              }
              loading={loading}
            >
              <HoldingTimeline
                timeline={currentReview.quarterly_timeline}
                symbol={currentReview.symbol}
              />
            </Card>
          ) : (
            <Card>
              <Empty
                description="请从左侧选择一只股票查看复盘"
                image={Empty.PRESENTED_IMAGE_SIMPLE}
              />
            </Card>
          )}
        </Col>
      </Row>
    </div>
  );
}
