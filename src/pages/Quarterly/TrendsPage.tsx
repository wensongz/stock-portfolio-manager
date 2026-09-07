import { useEffect } from "react";
import { Button, Card, Col, Row, Space, Spin, Statistic, Typography } from "antd";
import { ArrowLeftOutlined, ReloadOutlined } from "@ant-design/icons";
import { useNavigate } from "react-router-dom";
import { useQuarterlyStore } from "../../stores/quarterlyStore";
import TrendCharts from "./TrendCharts";
import { usePnlColor } from "../../hooks/usePnlColor";
import { formatQuarterlyMoney } from "./formatMoney";

const { Title, Text } = Typography;

export default function TrendsPage() {
  const navigate = useNavigate();
  const { trends, trendsLoading, fetchTrends } = useQuarterlyStore();
  const { pnlColorDark } = usePnlColor();

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
        <Button icon={<ReloadOutlined />} onClick={fetchTrends} loading={trendsLoading} size="small">
          刷新
        </Button>
      </div>

      {trendsLoading && (
        <div className="flex justify-center py-10">
          <Spin size="large" />
        </div>
      )}

      {trends && !trendsLoading && (
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
                      title="最新总市值 (USD)"
                      value={latestValue}
                      precision={2}
                      formatter={(value) => formatQuarterlyMoney(Number(value), "USD")}
                    />
                  </Card>
                </Col>
                <Col xs={12} sm={6}>
                  <Card size="small">
                    <Statistic
                      title="最新持仓盈亏 (USD)"
                      value={latestPnl}
                      precision={2}
                      formatter={(value) => formatQuarterlyMoney(Number(value), "USD")}
                      styles={{ content: {  color: pnlColorDark(latestPnl)  } }}
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
