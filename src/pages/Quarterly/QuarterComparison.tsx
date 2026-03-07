import { useEffect, useState } from "react";
import {
  Button,
  Card,
  Col,
  Divider,
  Row,
  Select,
  Space,
  Spin,
  Statistic,
  Table,
  Tag,
  Typography,
} from "antd";
import { ArrowLeftOutlined, SwapOutlined } from "@ant-design/icons";
import { useNavigate } from "react-router-dom";
import { useQuarterlyStore } from "../../stores/quarterlyStore";
import ComparisonCharts from "./ComparisonCharts";
import HoldingChangesTable from "./HoldingChangesTable";

const { Title, Text } = Typography;

function fmt(v: number) {
  return v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

export default function QuarterComparisonPage() {
  const navigate = useNavigate();
  const { snapshots, comparison, loading, fetchSnapshots, compareQuarters, clearComparison } =
    useQuarterlyStore();

  const [q1, setQ1] = useState<string | undefined>();
  const [q2, setQ2] = useState<string | undefined>();

  useEffect(() => {
    fetchSnapshots();
    return () => clearComparison();
  }, []);

  useEffect(() => {
    if (snapshots.length >= 2 && !q1 && !q2) {
      setQ2(snapshots[0].quarter);
      setQ1(snapshots[1].quarter);
    }
  }, [snapshots]);

  const handleCompare = () => {
    if (q1 && q2) {
      compareQuarters(q1, q2);
    }
  };

  const quarterOptions = snapshots.map((s) => ({ label: s.quarter, value: s.quarter }));

  const ov = comparison?.overview;

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <Space>
          <Button icon={<ArrowLeftOutlined />} onClick={() => navigate("/quarterly")}>
            返回
          </Button>
          <Title level={3} className="!mb-0">
            🔄 季度对比分析
          </Title>
        </Space>
      </div>

      {/* Quarter Selector */}
      <Card size="small" className="mb-4">
        <Space align="center" wrap>
          <Text>对比季度：</Text>
          <Select
            style={{ width: 140 }}
            options={quarterOptions}
            value={q1}
            onChange={setQ1}
            placeholder="选择季度 1"
          />
          <SwapOutlined />
          <Select
            style={{ width: 140 }}
            options={quarterOptions}
            value={q2}
            onChange={setQ2}
            placeholder="选择季度 2"
          />
          <Button type="primary" onClick={handleCompare} loading={loading} disabled={!q1 || !q2}>
            开始对比
          </Button>
        </Space>
      </Card>

      {loading && (
        <div className="flex justify-center py-10">
          <Spin size="large" />
        </div>
      )}

      {comparison && !loading && (
        <>
          {/* Overview */}
          <Row gutter={[16, 16]} className="mb-4">
            <Col xs={12} sm={6}>
              <Card size="small">
                <Statistic
                  title={`${comparison.quarter1} 市值`}
                  value={ov?.q1_total_value ?? 0}
                  precision={2}
                  prefix="$"
                />
              </Card>
            </Col>
            <Col xs={12} sm={6}>
              <Card size="small">
                <Statistic
                  title={`${comparison.quarter2} 市值`}
                  value={ov?.q2_total_value ?? 0}
                  precision={2}
                  prefix="$"
                />
              </Card>
            </Col>
            <Col xs={12} sm={6}>
              <Card size="small">
                <Statistic
                  title="市值变化"
                  value={ov?.value_change ?? 0}
                  precision={2}
                  prefix={`${(ov?.value_change ?? 0) >= 0 ? "+" : ""}$`}
                  valueStyle={{
                    color: (ov?.value_change ?? 0) >= 0 ? "#3f8600" : "#cf1322",
                  }}
                />
              </Card>
            </Col>
            <Col xs={12} sm={6}>
              <Card size="small">
                <Statistic
                  title="市值变化%"
                  value={Math.abs(ov?.value_change_percent ?? 0)}
                  precision={2}
                  suffix="%"
                  prefix={(ov?.value_change_percent ?? 0) >= 0 ? "+" : "-"}
                  valueStyle={{
                    color: (ov?.value_change_percent ?? 0) >= 0 ? "#3f8600" : "#cf1322",
                  }}
                />
              </Card>
            </Col>
          </Row>

          {/* Charts */}
          <ComparisonCharts comparison={comparison} />

          <Divider />

          {/* Market comparison table */}
          <Card size="small" className="mb-4" title="分市场对比">
            <Table
              dataSource={comparison.by_market}
              rowKey="market"
              size="small"
              pagination={false}
              columns={[
                {
                  title: "市场",
                  dataIndex: "market",
                  render: (m: string) => {
                    const labels: Record<string, string> = { US: "🇺🇸 美股", CN: "🇨🇳 A股", HK: "🇭🇰 港股" };
                    return <Tag>{labels[m] ?? m}</Tag>;
                  },
                },
                { title: `${comparison.quarter1} 市值`, dataIndex: "q1_value", render: fmt },
                { title: `${comparison.quarter2} 市值`, dataIndex: "q2_value", render: fmt },
                {
                  title: "变化",
                  dataIndex: "value_change",
                  render: (v: number) => (
                    <Text style={{ color: v >= 0 ? "#3f8600" : "#cf1322" }}>
                      {v >= 0 ? "+" : ""}{fmt(v)}
                    </Text>
                  ),
                },
                { title: `${comparison.quarter1} 盈亏`, dataIndex: "q1_pnl", render: fmt },
                { title: `${comparison.quarter2} 盈亏`, dataIndex: "q2_pnl", render: fmt },
              ]}
            />
          </Card>

          {/* Category comparison table */}
          <Card size="small" className="mb-4" title="分类别对比">
            <Table
              dataSource={comparison.by_category}
              rowKey="category_name"
              size="small"
              pagination={false}
              columns={[
                {
                  title: "类别",
                  dataIndex: "category_name",
                  render: (name: string, record: { category_color: string }) => (
                    <Tag color={record.category_color}>{name}</Tag>
                  ),
                },
                { title: `${comparison.quarter1} 市值`, dataIndex: "q1_value", render: fmt },
                { title: `${comparison.quarter2} 市值`, dataIndex: "q2_value", render: fmt },
                {
                  title: "变化",
                  dataIndex: "value_change",
                  render: (v: number) => (
                    <Text style={{ color: v >= 0 ? "#3f8600" : "#cf1322" }}>
                      {v >= 0 ? "+" : ""}{fmt(v)}
                    </Text>
                  ),
                },
              ]}
            />
          </Card>

          <Divider />

          {/* Holding changes */}
          <HoldingChangesTable
            changes={comparison.holding_changes}
            quarter1={comparison.quarter1}
            quarter2={comparison.quarter2}
          />
        </>
      )}
    </div>
  );
}
