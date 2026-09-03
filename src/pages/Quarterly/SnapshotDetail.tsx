import { useEffect } from "react";
import {
  Button,
  Card,
  Col,
  Divider,
  Row,
  Space,
  Statistic,
  Typography,
} from "antd";
import PieChart from "../../components/charts/PieChart";
import type { PieSlice } from "../../types";
import { ArrowLeftOutlined, EditOutlined, ReloadOutlined } from "@ant-design/icons";
import { useNavigate, useParams } from "react-router-dom";
import { useQuarterlyStore } from "../../stores/quarterlyStore";
import { useAccountStore } from "../../stores/accountStore";
import SnapshotHoldingsTable from "./SnapshotHoldingsTable";
import QuarterlyNotesEditor from "./QuarterlyNotesEditor";
import HoldingChangesTable from "./HoldingChangesTable";
import QuarterlyTransactionsSection from "./QuarterlyTransactionsSection";
import { usePnlColor } from "../../hooks/usePnlColor";

const { Title, Text } = Typography;

export default function SnapshotDetail() {
  const { snapshotId } = useParams<{ snapshotId: string }>();
  const navigate = useNavigate();
  const {
    detail,
    detailLoading,
    mutationLoading,
    quarterlyTransactions,
    fetchDetail,
    refreshSnapshot,
    clearDetail,
  } = useQuarterlyStore();
  const loading = detailLoading || mutationLoading;

  const { pnlColorDark } = usePnlColor();

  /** Build category distribution pie data for a subset of holdings */
  function categorySlices(hdgs: { market: string; category_name: string; category_color: string; market_value: number }[], market?: string): PieSlice[] {
    const subset = market ? hdgs.filter((h) => h.market === market) : hdgs;
    const map = new Map<string, { value: number; color: string }>();
    subset.forEach((h) => {
      const key = h.category_name || "未分类";
      const color = h.category_color || "#999";
      const prev = map.get(key);
      map.set(key, { value: (prev?.value ?? 0) + h.market_value, color });
    });
    const CATEGORY_ORDER = ["现金类", "分红股", "成长股", "套利"];
    return [...map.entries()]
      .map(([name, { value, color }]) => ({ name, value, color }))
      .sort((a, b) => {
        const ai = CATEGORY_ORDER.indexOf(a.name);
        const bi = CATEGORY_ORDER.indexOf(b.name);
        return (ai === -1 ? 999 : ai) - (bi === -1 ? 999 : bi);
      });
  }

  const holdings = detail?.holdings ?? [];

  const { fetchAccounts } = useAccountStore();

  useEffect(() => {
    if (snapshotId) {
      void fetchDetail(snapshotId);
    }
    fetchAccounts();
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
  const pnlColor = pnlColorDark(snap?.total_pnl ?? 0);

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
          onClick={async () => {
            if (snapshotId) {
              await refreshSnapshot(snapshotId);
            }
          }}
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
              styles={{ content: {  color: pnlColor  } }}
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
        <Row gutter={[12, 8]}>
          {[
            { label: "🇨🇳 A股", currency: "CNY", value: snap?.cn_value ?? 0, cost: snap?.cn_cost ?? 0 },
            { label: "🇭🇰 港股", currency: "HKD", value: snap?.hk_value ?? 0, cost: snap?.hk_cost ?? 0 },
            { label: "🇺🇸 美股", currency: "USD", value: snap?.us_value ?? 0, cost: snap?.us_cost ?? 0 },
          ].map(({ label, currency, value, cost }) => (
            <Col key={label} xs={8} style={{ textAlign: "center" }}>
              <Text type="secondary">{label}</Text>
              <br />
              <Text strong style={{ fontSize: 16 }}>
                {currency === "CNY" ? "¥" : currency === "HKD" ? "HK$" : "$"}
                {value.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
              </Text>
              <br />
              <Text type="secondary" style={{ fontSize: 12 }}>
                成本 {currency === "CNY" ? "¥" : currency === "HKD" ? "HK$" : "$"}
                {cost.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
              </Text>
            </Col>
          ))}
        </Row>
      </Card>

      {/* Category distribution */}
      {holdings.length > 0 && (
        <Card size="small" className="mb-4" title="类别分布">
          <Row gutter={[8, 8]}>
            {[
              { label: "整体", market: undefined, currency: "USD" },
              { label: "🇨🇳 A股", market: "CN", currency: "CNY" },
              { label: "🇭🇰 港股", market: "HK", currency: "HKD" },
              { label: "🇺🇸 美股", market: "US", currency: "USD" },
            ].map(({ label, market, currency }) => {
              const slices = categorySlices(holdings, market);
              if (slices.length === 0) return null;
              return (
                <Col key={label} xs={24} sm={12} lg={6}>
                  <PieChart data={slices} title={label} height={200} currencyCode={currency} hideLegend />
                </Col>
              );
            })}
          </Row>
          {/* Shared legend — built from all holdings so every category appears */}
          <div className="flex flex-wrap justify-start gap-x-4 gap-y-1 mt-2">
            {categorySlices(holdings).map(({ name, color }) => (
              <span key={name} className="flex items-center gap-1 text-sm">
                <span
                  style={{ display: "inline-block", width: 12, height: 12, borderRadius: 2, background: color ?? "var(--color-text-tertiary)", flexShrink: 0 }}
                />
                {name}
              </span>
            ))}
          </div>
        </Card>
      )}

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

      {/* Quarterly Transactions */}
      <QuarterlyTransactionsSection groups={quarterlyTransactions} loading={loading} />

      <Divider />

      {/* Holdings Table */}
      {snapshotId && (
        <Card size="small" title="持仓明细">
          <SnapshotHoldingsTable
            holdings={detail?.holdings ?? []}
            snapshotId={snapshotId}
            loading={loading}
            snap={snap}
          />
        </Card>
      )}
    </div>
  );
}
