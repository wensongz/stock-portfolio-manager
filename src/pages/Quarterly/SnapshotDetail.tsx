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
import { ArrowLeftOutlined, EditOutlined, ReloadOutlined } from "@ant-design/icons";
import { useNavigate, useParams } from "react-router-dom";
import { useQuarterlyStore } from "../../stores/quarterlyStore";
import { useAccountStore } from "../../stores/accountStore";
import SnapshotHoldingsTable from "./SnapshotHoldingsTable";
import QuarterlyNotesEditor from "./QuarterlyNotesEditor";
import HoldingChangesTable from "./HoldingChangesTable";
import QuarterlyTransactionsSection from "./QuarterlyTransactionsSection";
import { usePnlColor } from "../../hooks/usePnlColor";
import { buildSnapshotComposition, parseSnapshotExchangeRates } from "./aggregateSnapshotHoldings";
import { formatQuarterlyMoney } from "./formatMoney";

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
  const snapshotRates = parseSnapshotExchangeRates(snap?.exchange_rates);
  const categoryLegend = [...new Map(holdings.map((holding) => [holding.category_name || "未分类", { name: holding.category_name || "未分类", color: holding.category_color }])).values()];
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
              formatter={(value) => formatQuarterlyMoney(Number(value), "USD")}
            />
          </Card>
        </Col>
        <Col xs={12} sm={6}>
          <Card size="small">
            <Statistic
              title="总成本 (USD)"
              value={snap?.total_cost ?? 0}
              formatter={(value) => formatQuarterlyMoney(Number(value), "USD")}
            />
          </Card>
        </Col>
        <Col xs={12} sm={6}>
          <Card size="small">
            <Statistic
              title="持仓盈亏 (USD)"
              value={snap?.total_pnl ?? 0}
              formatter={(value) => formatQuarterlyMoney(Number(value), "USD")}
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
                {formatQuarterlyMoney(value, currency)}
              </Text>
              <br />
              <Text type="secondary" style={{ fontSize: 12 }}>
                成本 {formatQuarterlyMoney(cost, currency)}
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
              if (!holdings.some((holding) => !market || holding.market === market)) return null;
              const composition = buildSnapshotComposition(holdings, snapshotRates, market);
              return (
                <Col key={label} xs={24} sm={12} lg={6}>
                  {composition.pieSlices.length > 0 ? (
                    <PieChart data={composition.pieSlices} title={`${label} (${currency})`} height={200} currencyCode={currency} formatValue={(value) => formatQuarterlyMoney(value, currency)} hideLegend />
                  ) : (
                    <div style={{ minHeight: 200, display: "flex", flexDirection: "column", justifyContent: "center", textAlign: "center", gap: 8 }}>
                      <Text strong>{label} ({currency})</Text>
                      <Text type="secondary">{composition.hasMissingRates ? "缺少有效快照汇率，无法折算分布" : composition.hasNegativeValues ? "含负余额，不展示饼图；金额见持仓明细" : "暂无正余额可展示"}</Text>
                      {composition.total !== null && <Text>净市值 {formatQuarterlyMoney(composition.total, currency)}</Text>}
                    </div>
                  )}
                </Col>
              );
            })}
          </Row>
          {/* Shared legend — built from all holdings so every category appears */}
          <div className="flex flex-wrap justify-start gap-x-4 gap-y-1 mt-2">
            {categoryLegend.map(({ name, color }) => (
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
