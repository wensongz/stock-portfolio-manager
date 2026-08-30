import { Card, Space, Table, Tag, Tooltip, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import type { Currency, StockOperationEffect } from "../../types";
import { useTablePageSize } from "../../hooks/tablePageSize";
import {
  buildStockOperationIdentityDisplay,
  formatOperationCurrency,
  formatOperationPercent,
  formatStockOperationIdentity,
} from "./stockOperationReviewViewModel";

const { Text } = Typography;

const ACTION_LABELS: Record<StockOperationEffect["action_type"], string> = {
  open: "建仓",
  add: "加仓",
  reduce: "减仓",
  close: "清仓",
};

function number(value: number | null) {
  return value == null ? "—" : value.toLocaleString("zh-CN", { maximumFractionDigits: 4 });
}

export default function StockOperationActionsTable({
  actions,
  baseCurrency,
  reportAccountId,
}: {
  actions: StockOperationEffect[];
  baseCurrency: Currency;
  reportAccountId: string | null;
}) {
  const { pageSize, onShowSizeChange } = useTablePageSize();
  const identityColumnTitle = buildStockOperationIdentityDisplay(
    reportAccountId,
    "",
    "",
  ).columnTitle;
  const columns: ColumnsType<StockOperationEffect> = [
    { title: "日期", dataIndex: "trade_date", fixed: "left", width: 100 },
    {
      title: identityColumnTitle,
      fixed: "left",
      width: 175,
      render: (_, row) => {
        const identity = buildStockOperationIdentityDisplay(
          reportAccountId,
          row.market,
          row.account_name,
        );
        return (
          <Space orientation="vertical" size={0}>
            <Text strong>{formatStockOperationIdentity(row.symbol, row.name)}</Text>
            {identity.actionSecondary && (
              <Text type="secondary">{identity.actionSecondary}</Text>
            )}
          </Space>
        );
      },
    },
    { title: "操作", dataIndex: "action_type", width: 60, render: (value: StockOperationEffect["action_type"]) => ACTION_LABELS[value] },
    { title: "股数", dataIndex: "quantity", align: "right", width: 60, render: number },
    { title: "成交均价", dataIndex: "trade_price", align: "right", width: 80, render: number },
    { title: "成交金额", dataIndex: "trade_notional_local", align: "right", width: 90, render: (value, row) => formatOperationCurrency(value, row.currency) },
    { title: "费用", dataIndex: "fee_local", align: "right", width: 60, render: (value, row) => formatOperationCurrency(value, row.currency) },
    { title: "股数（前 → 后）", width: 120, render: (_, row) => `${number(row.shares_before)} → ${number(row.shares_after)}` },
    { title: "权重（前 → 后）估算", width: 130, render: (_, row) => `${formatOperationPercent(row.weight_before)} → ${formatOperationPercent(row.weight_after)}` },
    { title: "权重变化", dataIndex: "weight_change", align: "right", width: 80, render: formatOperationPercent },
    { title: "评价日", dataIndex: "evaluation_date", width: 100, render: (value) => value ?? "—" },
    { title: "期末价", dataIndex: "end_price", align: "right", width: 80, render: number },
    { title: "本币价格效果", dataIndex: "price_effect_local", align: "right", width: 100, render: (value, row) => formatOperationCurrency(value, row.currency) },
    { title: `价格效果（${baseCurrency}）`, dataIndex: "price_effect_base", align: "right", width: 110, render: (value) => formatOperationCurrency(value, baseCurrency) },
    { title: "价格效果率", dataIndex: "price_effect_percent", align: "right", width: 100, render: formatOperationPercent },
    { title: "自动基准", dataIndex: "benchmark_symbol", width: 80, render: (value) => value ?? "—" },
    { title: "基准收益", dataIndex: "benchmark_return", align: "right", width: 80, render: formatOperationPercent },
    { title: "方向调整相对效果", dataIndex: "directional_excess_return", align: "right", width: 120, render: formatOperationPercent },
    {
      title: "事实标签 / 数据说明",
      width: 200,
      render: (_, row) => (
        <Space wrap>
          {row.fact_labels.map((label) => <Tag key={label}>{label}</Tag>)}
          {row.issues.length > 0 && (
            <Tooltip title={<Space orientation="vertical" size={2}>{row.issues.map((issue) => <span key={`${issue.code}:${issue.field}`}>{issue.message}</span>)}</Space>}>
              <Tag color="gold">{row.issues.length} 项字段说明</Tag>
            </Tooltip>
          )}
        </Space>
      ),
    },
  ];
  return (
    <Card title="操作明细">
      <Table
        rowKey="action_id"
        size="small"
        columns={columns}
        dataSource={actions}
        scroll={{ x: 2500 }}
        pagination={{ pageSize, showSizeChanger: true, onShowSizeChange, showTotal: (total) => `共 ${total} 项操作` }}
        locale={{ emptyText: "所选区间没有可评价的股票买卖操作" }}
      />
      <Text type="secondary">
        价格效果比较“执行该操作”和“不执行该操作”的期末价格差，不是组合收益归因；权重按操作前最近有效总资产估算。
      </Text>
    </Card>
  );
}
