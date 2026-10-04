import { useEffect, useMemo } from "react";
import { Alert, Button, Card, Col, Row, Space, Spin, Statistic, Typography } from "antd";
import { ArrowLeftOutlined, ReloadOutlined } from "@ant-design/icons";
import { useNavigate } from "react-router-dom";
import { useQuarterlyStore } from "../../stores/quarterlyStore";
import TrendCharts from "./TrendCharts";
import { usePnlColor } from "../../hooks/usePnlColor";
import { formatQuarterlyMoney } from "./formatMoney";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import BaseCurrencySelect from "../../components/BaseCurrencySelect";
import { convertQuarterlyTrends } from "./quarterlyCurrency";

const { Title, Text } = Typography;

export default function TrendsPage() {
  const navigate = useNavigate();
  const { trends, trendsLoading, trendsError, snapshots, listLoading, listError, fetchTrends, fetchSnapshots } = useQuarterlyStore();
  const baseCurrency = useExchangeRateStore((state) => state.baseCurrency);
  const { pnlColorDark } = usePnlColor();

  useEffect(() => {
    fetchTrends();
    fetchSnapshots();
  }, []);

  const displayedTrends = useMemo(() => trends ? convertQuarterlyTrends(trends, snapshots, baseCurrency) : null, [trends, snapshots, baseCurrency]);
  const lastIdx = (displayedTrends?.quarters.length ?? 0) - 1;
  const latestValue = displayedTrends?.total_values[lastIdx] ?? 0;
  const latestPnl = displayedTrends?.total_pnls[lastIdx] ?? 0;

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
        <Space>
          <BaseCurrencySelect />
          <Button icon={<ReloadOutlined />} onClick={() => { fetchTrends(); fetchSnapshots(); }} loading={trendsLoading || listLoading} size="small">刷新</Button>
        </Space>
      </div>

      {(trendsLoading || listLoading) && (
        <div className="flex justify-center py-10">
          <Spin size="large" />
        </div>
      )}

      {listError && <Alert type="error" showIcon title="季度快照列表加载失败" description={listError} action={<Button size="small" onClick={() => void fetchSnapshots()}>重试</Button>} className="mb-4" />}
      {trendsError && <Alert type="error" showIcon title="季度趋势加载失败" description={trendsError} action={<Button size="small" onClick={() => void fetchTrends()}>重试</Button>} className="mb-4" />}

      {trends && !trendsLoading && !listLoading && !listError && !trendsError && !displayedTrends && (
        <Alert type="warning" showIcon title={`缺少部分季度的有效快照汇率，无法以 ${baseCurrency} 显示趋势金额`} className="mb-4" />
      )}

      {displayedTrends && !trendsLoading && !listLoading && !listError && !trendsError && (
        <>
          {displayedTrends.quarters.length === 0 ? (
            <Text type="secondary">暂无季度快照数据，请先创建季度快照</Text>
          ) : (
            <>
              <Row gutter={[16, 16]} className="mb-4">
                <Col xs={12} sm={6}>
                  <Card size="small">
                    <Statistic title="季度数量" value={displayedTrends.quarters.length} suffix="个" />
                  </Card>
                </Col>
                <Col xs={12} sm={6}>
                  <Card size="small">
                    <Statistic
                      title={`最新总市值 (${baseCurrency})`}
                      value={latestValue}
                      precision={2}
                      formatter={(value) => formatQuarterlyMoney(Number(value), baseCurrency)}
                    />
                  </Card>
                </Col>
                <Col xs={12} sm={6}>
                  <Card size="small">
                    <Statistic
                      title={`最新持仓盈亏 (${baseCurrency})`}
                      value={latestPnl}
                      precision={2}
                      formatter={(value) => formatQuarterlyMoney(Number(value), baseCurrency)}
                      styles={{ content: {  color: pnlColorDark(latestPnl)  } }}
                    />
                  </Card>
                </Col>
                <Col xs={12} sm={6}>
                  <Card size="small">
                    <Statistic
                      title="最新持仓数"
                      value={displayedTrends.holding_counts[lastIdx] ?? 0}
                      suffix="只"
                    />
                  </Card>
                </Col>
              </Row>

              <TrendCharts trends={displayedTrends} baseCurrency={baseCurrency} />
            </>
          )}
        </>
      )}
    </div>
  );
}
