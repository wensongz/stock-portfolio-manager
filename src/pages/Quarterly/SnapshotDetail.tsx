import { useEffect } from "react";
import {
  Button,
  Card,
  Col,
  Descriptions,
  Divider,
  Row,
  Space,
  Statistic,
  Typography,
} from "antd";
import { ArrowLeftOutlined, EditOutlined, ReloadOutlined } from "@ant-design/icons";
import { useNavigate, useParams } from "react-router-dom";
import { useQuarterlyStore } from "../../stores/quarterlyStore";
import SnapshotHoldingsTable from "./SnapshotHoldingsTable";
import QuarterlyNotesEditor from "./QuarterlyNotesEditor";
import HoldingChangesTable from "./HoldingChangesTable";

const { Title, Text } = Typography;

function fmt(val: number) {
  return val.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

export default function SnapshotDetail() {
  const { snapshotId } = useParams<{ snapshotId: string }>();
  const navigate = useNavigate();
  const { detail, loading, fetchDetail, refreshSnapshot, clearDetail } = useQuarterlyStore();

  useEffect(() => {
    if (snapshotId) {
      fetchDetail(snapshotId);
    }
    return () => clearDetail();
  }, [snapshotId]);

  if (!detail && !loading) {
    return (
      <div>
        <Button icon={<ArrowLeftOutlined />} onClick={() => navigate("/quarterly")}>
          返回
        </Button>
        <div className="mt-4">快照不存在或已删除</div>
      </div>
    );
  }

  const snap = detail?.snapshot;
  const pnlColor = (snap?.total_pnl ?? 0) >= 0 ? "#3f8600" : "#cf1322";

  return (
    <div>
      {/* Header */}
      <div className="flex justify-between items-center mb-4">
        <Space>
          <Button icon={<ArrowLeftOutlined />} onClick={() => navigate("/quarterly")}>
            返回
          </Button>
          <Title level={3} className="!mb-0">
            📅 {snap?.quarter} 季度快照
          </Title>
        </Space>
        <Button
          icon={<ReloadOutlined />}
          onClick={() => snapshotId && refreshSnapshot(snapshotId)}
          loading={loading}
          size="small"
        >
          刷新
        </Button>
      </div>

      {/* Overview Cards */}
      <Row gutter={[16, 16]} className="mb-4">
        <Col xs={12} sm={6}>
          <Card size="small">
            <Statistic
              title="总市值 (USD)"
              value={snap?.total_value ?? 0}
              precision={2}
              prefix="$"
            />
          </Card>
        </Col>
        <Col xs={12} sm={6}>
          <Card size="small">
            <Statistic
              title="总成本 (USD)"
              value={snap?.total_cost ?? 0}
              precision={2}
              prefix="$"
            />
          </Card>
        </Col>
        <Col xs={12} sm={6}>
          <Card size="small">
            <Statistic
              title="总盈亏 (USD)"
              value={snap?.total_pnl ?? 0}
              precision={2}
              prefix="$"
              valueStyle={{ color: pnlColor }}
            />
          </Card>
        </Col>
        <Col xs={12} sm={6}>
          <Card size="small">
            <Statistic title="持仓数量" value={snap?.holding_count ?? 0} suffix="只" />
          </Card>
        </Col>
      </Row>

      {/* Market breakdown */}
      <Card size="small" className="mb-4" title="分市场市值">
        <Descriptions size="small" column={{ xs: 1, sm: 3 }}>
          <Descriptions.Item label="🇺🇸 美股 (USD)">
            <Text strong>${fmt(snap?.us_value ?? 0)}</Text>
            <Text type="secondary" className="ml-2">
              成本 ${fmt(snap?.us_cost ?? 0)}
            </Text>
          </Descriptions.Item>
          <Descriptions.Item label="🇨🇳 A股 (CNY)">
            <Text strong>¥{fmt(snap?.cn_value ?? 0)}</Text>
            <Text type="secondary" className="ml-2">
              成本 ¥{fmt(snap?.cn_cost ?? 0)}
            </Text>
          </Descriptions.Item>
          <Descriptions.Item label="🇭🇰 港股 (HKD)">
            <Text strong>HK${fmt(snap?.hk_value ?? 0)}</Text>
            <Text type="secondary" className="ml-2">
              成本 HK${fmt(snap?.hk_cost ?? 0)}
            </Text>
          </Descriptions.Item>
        </Descriptions>
      </Card>

      <Divider />

      {/* Quarterly Notes */}
      {snapshotId && snap && (
        <Card
          size="small"
          className="mb-4"
          title={
            <Space>
              <EditOutlined />
              <span>季度总结</span>
            </Space>
          }
        >
          <QuarterlyNotesEditor
            snapshotId={snapshotId}
            initialNotes={snap.overall_notes ?? ""}
          />
        </Card>
      )}

      <Divider />

      {/* Quarterly Operations - Holding Changes vs Previous Quarter */}
      {detail?.holding_changes && detail?.previous_quarter && (
        <>
          <HoldingChangesTable
            changes={detail.holding_changes}
            quarter1={detail.previous_quarter}
            quarter2={snap?.quarter ?? ""}
            title={`季度操作 (${detail.previous_quarter} → ${snap?.quarter})`}
          />
          <Divider />
        </>
      )}

      {/* Holdings Table */}
      {snapshotId && (
        <Card size="small" title="持仓明细">
          <SnapshotHoldingsTable
            holdings={detail?.holdings ?? []}
            snapshotId={snapshotId}
            loading={loading}
          />
        </Card>
      )}
    </div>
  );
}
