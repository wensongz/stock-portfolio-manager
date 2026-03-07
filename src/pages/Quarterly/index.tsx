import { useEffect, useState } from "react";
import {
  Button,
  Card,
  Col,
  Popconfirm,
  Row,
  Space,
  Statistic,
  Table,
  Tag,
  Typography,
  message,
} from "antd";
import {
  CalendarOutlined,
  DeleteOutlined,
  EyeOutlined,
  LineChartOutlined,
  PlusOutlined,
  ReloadOutlined,
  SwapOutlined,
} from "@ant-design/icons";
import { useNavigate } from "react-router-dom";
import { useQuarterlyStore } from "../../stores/quarterlyStore";
import type { QuarterlySnapshot } from "../../types";

const { Title, Text } = Typography;

function fmt(val: number) {
  return val.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

export default function QuarterlyPage() {
  const navigate = useNavigate();
  const {
    snapshots,
    missingQuarters,
    loading,
    fetchSnapshots,
    fetchMissingQuarters,
    createSnapshot,
    deleteSnapshot,
  } = useQuarterlyStore();

  const [creating, setCreating] = useState(false);

  useEffect(() => {
    fetchSnapshots();
    fetchMissingQuarters();
  }, []);

  const handleCreateCurrent = async () => {
    setCreating(true);
    const snap = await createSnapshot();
    setCreating(false);
    if (snap) {
      message.success(`已创建季度快照 ${snap.quarter}`);
    }
  };

  const handleCreateMissing = async (quarter: string) => {
    setCreating(true);
    const snap = await createSnapshot(quarter);
    setCreating(false);
    if (snap) {
      message.success(`已补录季度快照 ${snap.quarter}`);
      fetchMissingQuarters();
    }
  };

  const handleDelete = async (id: string) => {
    await deleteSnapshot(id);
    message.success("快照已删除");
    fetchMissingQuarters();
  };

  const columns = [
    {
      title: "季度",
      dataIndex: "quarter",
      key: "quarter",
      render: (q: string) => (
        <Tag color="blue" icon={<CalendarOutlined />}>
          {q}
        </Tag>
      ),
    },
    {
      title: "快照日期",
      dataIndex: "snapshot_date",
      key: "snapshot_date",
    },
    {
      title: "总市值 (USD)",
      dataIndex: "total_value",
      key: "total_value",
      render: (v: number) => <Text strong>${fmt(v)}</Text>,
    },
    {
      title: "总盈亏 (USD)",
      dataIndex: "total_pnl",
      key: "total_pnl",
      render: (v: number) => (
        <Text style={{ color: v >= 0 ? "#3f8600" : "#cf1322" }}>
          {v >= 0 ? "+" : ""}${fmt(v)}
        </Text>
      ),
    },
    {
      title: "持仓数",
      dataIndex: "holding_count",
      key: "holding_count",
    },
    {
      title: "季度总结",
      dataIndex: "overall_notes",
      key: "overall_notes",
      render: (notes: string | null) =>
        notes ? (
          <Tag color="green">已填写</Tag>
        ) : (
          <Tag color="default">未填写</Tag>
        ),
    },
    {
      title: "操作",
      key: "actions",
      render: (_: unknown, record: QuarterlySnapshot) => (
        <Space>
          <Button
            size="small"
            icon={<EyeOutlined />}
            onClick={() => navigate(`/quarterly/${record.id}`)}
          >
            详情
          </Button>
          <Popconfirm
            title="确认删除此季度快照？"
            onConfirm={() => handleDelete(record.id)}
            okText="删除"
            cancelText="取消"
          >
            <Button size="small" danger icon={<DeleteOutlined />}>
              删除
            </Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <div>
      {/* Header */}
      <div className="flex justify-between items-center mb-4">
        <Title level={2} className="!mb-0">
          📅 季度分析
        </Title>
        <Space>
          <Button
            icon={<ReloadOutlined />}
            onClick={() => { fetchSnapshots(); fetchMissingQuarters(); }}
            loading={loading}
            size="small"
          >
            刷新
          </Button>
          <Button
            type="primary"
            icon={<PlusOutlined />}
            onClick={handleCreateCurrent}
            loading={creating}
          >
            创建当前季度快照
          </Button>
          <Button
            icon={<SwapOutlined />}
            onClick={() => navigate("/quarterly/compare")}
            disabled={snapshots.length < 2}
          >
            季度对比
          </Button>
          <Button
            icon={<LineChartOutlined />}
            onClick={() => navigate("/quarterly/trends")}
            disabled={snapshots.length === 0}
          >
            趋势图表
          </Button>
        </Space>
      </div>

      {/* Summary Cards */}
      <Row gutter={[16, 16]} className="mb-4">
        <Col xs={24} sm={8}>
          <Card size="small">
            <Statistic title="已有快照" value={snapshots.length} suffix="个" />
          </Card>
        </Col>
        <Col xs={24} sm={8}>
          <Card size="small">
            <Statistic title="缺失快照" value={missingQuarters.length} suffix="个" />
          </Card>
        </Col>
        <Col xs={24} sm={8}>
          <Card size="small">
            <Statistic
              title="最新季度"
              value={snapshots.length > 0 ? snapshots[0].quarter : "—"}
            />
          </Card>
        </Col>
      </Row>

      {/* Missing quarters alert */}
      {missingQuarters.length > 0 && (
        <Card
          size="small"
          className="mb-4"
          title={<Text type="warning">⚠️ 以下季度缺少快照，可点击补录</Text>}
        >
          <Space wrap>
            {missingQuarters.map((q) => (
              <Button
                key={q}
                size="small"
                icon={<PlusOutlined />}
                onClick={() => handleCreateMissing(q)}
                loading={creating}
              >
                {q}
              </Button>
            ))}
          </Space>
        </Card>
      )}

      {/* Snapshots Table */}
      <Card size="small">
        <Table
          dataSource={snapshots}
          columns={columns}
          rowKey="id"
          loading={loading}
          pagination={{ pageSize: 20 }}
          size="small"
        />
      </Card>
    </div>
  );
}
