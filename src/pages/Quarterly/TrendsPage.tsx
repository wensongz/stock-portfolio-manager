import { useEffect } from "react";
import { Button, Card, Col, Row, Space, Spin, Statistic, Typography } from "antd";
import { ArrowLeftOutlined, ReloadOutlined } from "@ant-design/icons";
import { useNavigate } from "react-router-dom";
import { useQuarterlyStore } from "../../stores/quarterlyStore";
import TrendCharts from "./TrendCharts";

const { Title, Text } = Typography;

export default function TrendsPage() {
  const navigate = useNavigate();
  const { trends, loading, fetchTrends } = useQuarterlyStore();

  useEffect(() => {
    fetchTrends();
  }, []);

  const lastIdx = (trends?.quarters.length ?? 0) - 1;
  const latestValue = trends?.total_values[lastIdx] ?? 0;
  const latestPnl = trends?.total_pnls[lastIdx] ?? 0;

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <Space>
          <Button icon={<ArrowLeftOutlined />} onClick={() => navigate("/quarterly")}>
            返回
          </Button>
          <Title level={3} className="!mb-0">
            📈 多季度趋势
          </Title>
        </Space>
        <Button icon={<ReloadOutlined />} onClick={fetchTrends} loading={loading} size="small">
          刷新
        </Button>
      </div>

      {loading && (
        <div className="flex justify-center py-10">
          <Spin size="large" />
        </div>
      )}

      {trends && !loading && (
        <>
          {trends.quarters.length === 0 ? (
            <Text type="secondary">暂无季度快照数据，请先创建季度快照</Text>
          ) : (
            <>
              <Row gutter={[16, 16]} className="mb-4">
                <Col xs={12} sm={6}>
                  <Card size="small">
                    <Statistic title="季度数量" value={trends.quarters.length} suffix="个" />
                  </Card>
                </Col>
                <Col xs={12} sm={6}>
                  <Card size="small">
                    <Statistic
                      title="最新总市值"
                      value={latestValue}
                      precision={2}
                      prefix="$"
                    />
                  </Card>
                </Col>
                <Col xs={12} sm={6}>
                  <Card size="small">
                    <Statistic
                      title="最新总盈亏"
                      value={latestPnl}
                      precision={2}
                      prefix="$"
                      valueStyle={{ color: latestPnl >= 0 ? "#3f8600" : "#cf1322" }}
                    />
                  </Card>
                </Col>
                <Col xs={12} sm={6}>
                  <Card size="small">
                    <Statistic
                      title="最新持仓数"
                      value={trends.holding_counts[lastIdx] ?? 0}
                      suffix="只"
                    />
                  </Card>
                </Col>
              </Row>

              <TrendCharts trends={trends} />
            </>
          )}
        </>
      )}
    </div>
  );
}
