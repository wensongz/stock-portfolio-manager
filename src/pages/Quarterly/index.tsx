import { useEffect, useState } from "react";
import {
  Alert,
  Button,
  Card,
  Popconfirm,
  Space,
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
import { usePnlColor } from "../../hooks/usePnlColor";

const { Title, Text } = Typography;

function fmt(val: number) {
  return val.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

export default function QuarterlyPage() {
  const navigate = useNavigate();
  const { pnlColorDark } = usePnlColor();
  const {
    snapshots,
    listLoading,
    listError,
    mutationLoading,
    mutationError,
    initializationLoading,
    initializationError,
    initializeSnapshots,
    createSnapshot,
    deleteSnapshot,
  } = useQuarterlyStore();
  const loading = listLoading || mutationLoading;
  const mutationDisabled = loading || initializationLoading;

  const [creating, setCreating] = useState(false);

  useEffect(() => {
    void initializeSnapshots();
  }, [initializeSnapshots]);

  const handleCreateCurrent = async () => {
    setCreating(true);
    const snap = await createSnapshot();
    setCreating(false);
    if (snap) {
      message.success(`已创建季度快照 ${snap.quarter}`);
    }
  };

  const handleDelete = async (id: string) => {
    await deleteSnapshot(id);
    message.success("快照已删除");
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
      title: "持仓盈亏 (USD)",
      dataIndex: "total_pnl",
      key: "total_pnl",
      render: (v: number) => (
        <Text style={{ color: pnlColorDark(v) }}>
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
            <Button size="small" danger icon={<DeleteOutlined />} disabled={mutationDisabled}>
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
          <CalendarOutlined style={{ color: "#722ed1" }} /> 季度分析
        </Title>
        <Space>
          <Button
            icon={<ReloadOutlined />}
            onClick={() => void initializeSnapshots()}
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
            disabled={mutationDisabled}
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

      {initializationError && (
        <Alert
          type="warning"
          showIcon
          title="部分季度快照未能自动生成"
          description={<div style={{ whiteSpace: "pre-wrap" }}>{initializationError}</div>}
          action={<Button size="small" onClick={() => void initializeSnapshots()} disabled={mutationDisabled}>重试</Button>}
          className="mb-4"
        />
      )}
      {(listError || mutationError) && (
        <Alert type="error" showIcon title={listError || mutationError} className="mb-4" />
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
