import { Button, Card, Flex, Select, Space, Table, Tag, Typography } from "antd";
import { SortAscendingOutlined, SortDescendingOutlined } from "@ant-design/icons";
import { useMemo, useState } from "react";
import type { ColumnsType } from "antd/es/table";
import type { Currency, StockActionReview, StockCampaignSummary } from "../../types";
import {
  formatStockReviewPercent,
  getStockActionTypeDisplay,
  getStockReviewStatusDisplay,
  sortStockReviewActions,
  type StockReviewActionSortKey,
  type StockReviewSortOrder,
} from "./stockReviewViewModel";

const { Text } = Typography;

function number(value: number | null) { return value == null ? "—" : value.toLocaleString("zh-CN", { maximumFractionDigits: 4 }); }
function money(value: number | null, currency: string | null) { return value == null ? "—" : `${currency ?? ""} ${value.toLocaleString("zh-CN", { maximumFractionDigits: 2 })}`.trim(); }
function windowValue(action: StockActionReview, days: number) { return action.observation_windows.find((window) => window.trading_days === days) ?? null; }

export default function StockActionsTable({
  actions,
  campaigns,
  baseCurrency,
  onOpenCampaign,
}: {
  actions: StockActionReview[];
  campaigns: StockCampaignSummary[];
  baseCurrency: Currency;
  onOpenCampaign: (campaignId: string) => void;
}) {
  const [symbol, setSymbol] = useState<string | null>(null);
  const [actionType, setActionType] = useState<string | null>(null);
  const [campaignId, setCampaignId] = useState<string | null>(null);
  const [status, setStatus] = useState<string | null>(null);
  const [sortKey, setSortKey] = useState<StockReviewActionSortKey>("date");
  const [sortOrder, setSortOrder] = useState<StockReviewSortOrder>("descend");
  const campaignByAction = useMemo(() => new Map(campaigns.flatMap((campaign) => campaign.action_ids.map((actionId) => [actionId, campaign.campaign_id] as const))), [campaigns]);
  const filtered = useMemo(() => sortStockReviewActions(actions.filter((action) =>
    (!symbol || action.symbol === symbol) &&
    (!actionType || action.action_type === actionType) &&
    (!campaignId || campaignByAction.get(action.action_id) === campaignId) &&
    (!status || action.status === status)
  ), sortKey, sortOrder), [actions, symbol, actionType, campaignId, status, sortKey, sortOrder, campaignByAction]);

  const columns: ColumnsType<StockActionReview> = [
    { title: "日期", dataIndex: "traded_at", width: 112, render: (value: string) => value.slice(0, 10) },
    { title: "账户", dataIndex: "account_id", width: 120 },
    { title: "股票", dataIndex: "symbol", width: 90, fixed: "left" },
    { title: "动作", dataIndex: "action_type", width: 80, render: (value: StockActionReview["action_type"]) => getStockActionTypeDisplay(value) },
    { title: "加权价格", dataIndex: "weighted_average_price", align: "right", width: 110, render: (value) => number(value) },
    { title: "金额", dataIndex: "gross_amount", align: "right", width: 130, render: (value, row) => money(value, row.currency) },
    { title: "费用", dataIndex: "fees", align: "right", width: 110, render: (value, row) => money(value, row.currency) },
    { title: "股数（前 → 后）", width: 145, render: (_, row) => `${number(row.shares_before)} → ${number(row.shares_after)}` },
    { title: "组合权重（前 → 后）", width: 170, render: (_, row) => `${formatStockReviewPercent(row.portfolio_weight_before)} → ${formatStockReviewPercent(row.portfolio_weight_after)}` },
    { title: "60 日效果", width: 110, render: (_, row) => formatStockReviewPercent(windowValue(row, 60)?.amount_weighted_excess_return ?? null) },
    { title: "120 日效果", width: 110, render: (_, row) => formatStockReviewPercent(windowValue(row, 120)?.amount_weighted_excess_return ?? null) },
    { title: `调仓贡献（${baseCurrency}）`, dataIndex: "contribution", align: "right", width: 150, render: (value) => money(value, baseCurrency) },
    { title: "状态", dataIndex: "status", width: 95, render: (value) => { const item = getStockReviewStatusDisplay(value); return <Tag color={item.color}>{item.label}</Tag>; } },
    { title: "事实标签", dataIndex: "fact_labels", width: 220, render: (labels: string[]) => <Space wrap>{labels.map((label) => <Tag key={label}>{label}</Tag>)}</Space> },
  ];

  return (
    <Card title="全部调仓动作">
      <Flex wrap gap={8} style={{ marginBottom: 12 }}>
        <Select allowClear placeholder="股票" aria-label="按股票筛选动作" style={{ minWidth: 110 }} value={symbol} onChange={setSymbol} options={[...new Set(actions.map((action) => action.symbol))].sort().map((value) => ({ value, label: value }))} />
        <Select allowClear placeholder="动作类型" aria-label="按动作类型筛选" style={{ minWidth: 120 }} value={actionType} onChange={setActionType} options={["open", "add", "reduce", "close"].map((value) => ({ value, label: getStockActionTypeDisplay(value as StockActionReview["action_type"]) }))} />
        <Select allowClear placeholder="Campaign" aria-label="按 Campaign 筛选动作" style={{ minWidth: 180 }} value={campaignId} onChange={setCampaignId} options={campaigns.map((campaign) => ({ value: campaign.campaign_id, label: `${campaign.symbol} · ${campaign.campaign_id}` }))} />
        <Select allowClear placeholder="数据状态" aria-label="按数据状态筛选动作" style={{ minWidth: 120 }} value={status} onChange={setStatus} options={["available", "degraded", "pending", "unavailable"].map((value) => ({ value, label: getStockReviewStatusDisplay(value as StockActionReview["status"]).label }))} />
        <Select aria-label="动作排序字段" value={sortKey} onChange={setSortKey} options={[{ value: "date", label: "按日期" }, { value: "amount", label: "按金额" }, { value: "contribution", label: "按调仓贡献" }, { value: "forward_effect", label: "按 60 日效果" }]} />
        <Button aria-label={sortOrder === "descend" ? "当前降序，点击改为升序" : "当前升序，点击改为降序"} icon={sortOrder === "descend" ? <SortDescendingOutlined /> : <SortAscendingOutlined />} onClick={() => setSortOrder((order) => order === "descend" ? "ascend" : "descend")}>{sortOrder === "descend" ? "降序" : "升序"}</Button>
      </Flex>
      <Table
        rowKey="action_id"
        size="small"
        columns={columns}
        dataSource={filtered}
        scroll={{ x: 1750 }}
        pagination={{ pageSize: 15, showSizeChanger: true, showTotal: (total) => `共 ${total} 项动作` }}
        locale={{ emptyText: "所选筛选条件下没有调仓动作" }}
        onRow={(record) => ({
          role: "button",
          tabIndex: 0,
          "aria-label": `打开 ${record.symbol} ${getStockActionTypeDisplay(record.action_type)} Campaign`,
          style: { cursor: campaignByAction.has(record.action_id) ? "pointer" : "default" },
          onClick: () => { const id = campaignByAction.get(record.action_id); if (id) onOpenCampaign(id); },
          onKeyDown: (event) => { if (event.key === "Enter" || event.key === " ") { event.preventDefault(); const id = campaignByAction.get(record.action_id); if (id) onOpenCampaign(id); } },
        })}
      />
      <Text type="secondary">所有金额、权重、后续效果、贡献、状态与事实标签均直接来自确定性报告。</Text>
    </Card>
  );
}
