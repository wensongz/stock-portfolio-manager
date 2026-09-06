import { Card, Flex, Select, Space, Table, Typography } from "antd";
import { useMemo, useState } from "react";
import type { ColumnsType } from "antd/es/table";
import type { Currency, StockOperationSecuritySummary } from "../../types";
import { useTablePageSize } from "../../hooks/tablePageSize";
import {
  buildStockOperationIdentityDisplay,
  formatOperationCurrency,
  formatOperationPercent,
  formatStockOperationIdentity,
  sortStockOperationSecurities,
  type StockOperationSecuritySortKey,
} from "./stockOperationReviewViewModel";

const { Text } = Typography;

function number(value: number) {
  return value.toLocaleString("zh-CN", { maximumFractionDigits: 4 });
}

export default function StockOperationSecurityTable({
  rows,
  baseCurrency,
  reportAccountId,
}: {
  rows: StockOperationSecuritySummary[];
  baseCurrency: Currency;
  reportAccountId: string | null;
}) {
  const [sortKey, setSortKey] = useState<StockOperationSecuritySortKey>("effect");
  const sorted = useMemo(() => sortStockOperationSecurities(rows, sortKey), [rows, sortKey]);
  const { pageSize, onShowSizeChange } = useTablePageSize();
  const identityColumnTitle = buildStockOperationIdentityDisplay(
    reportAccountId,
    "",
    "",
  ).columnTitle;
  const columns: ColumnsType<StockOperationSecuritySummary> = [
    {
      title: identityColumnTitle,
      fixed: "left",
      ellipsis: true,
      width: 160,
      render: (_, row) => {
        const identity = buildStockOperationIdentityDisplay(
          reportAccountId,
          row.market,
          row.account_name,
        );
        return (
          <Space orientation="vertical" size={0}>
            <Text strong>{formatStockOperationIdentity(row.symbol, row.name)}</Text>
            {identity.securitySecondary && (
              <Text type="secondary">{identity.securitySecondary}</Text>
            )}
          </Space>
        );
      },
    },
    { title: "建 / 加 / 减 / 清", width: 90, render: (_, row) => `${row.open_count} / ${row.add_count} / ${row.reduce_count} / ${row.close_count}` },
    { title: "净增减股数", dataIndex: "net_shares", align: "right", width: 80, render: number },
    {
      title: "买入 / 卖出金额",
      align: "right",
      width: 160,
      render: (_, row) => `${formatOperationCurrency(row.buy_notional_local, row.currency)} / ${formatOperationCurrency(row.sell_notional_local, row.currency)}`,
    },
    { title: `期末价格效果（${baseCurrency}）`, dataIndex: "price_effect_base", align: "right", width: 140, render: (value) => formatOperationCurrency(value, baseCurrency) },
    { title: "相对市场", dataIndex: "weighted_excess_return", align: "right", width: 80, render: formatOperationPercent },
    { title: "最大权重变化（估算）", dataIndex: "largest_absolute_weight_change", align: "right", width: 140, render: formatOperationPercent },
    { title: "正向 / 负向 / 缺数据", width: 120, render: (_, row) => `${row.positive_count} / ${row.negative_count} / ${row.missing_effect_count}` },
  ];
  return (
    <Card title="股票操作效果排行">
      <Flex justify="space-between" align="center" wrap gap={8} style={{ marginBottom: 12 }}>
        <Text type="secondary">默认按截至期末的价格效果排序；缺少该字段的股票排在末尾。</Text>
        <Select
          aria-label="股票操作排行排序"
          value={sortKey}
          onChange={setSortKey}
          style={{ minWidth: 160 }}
          options={[
            { value: "effect", label: "按价格效果" },
            { value: "notional", label: "按操作金额" },
            { value: "benchmark", label: "按相对市场" },
            { value: "weight", label: "按权重变化" },
          ]}
        />
      </Flex>
      <Table
        rowKey={(row) => `${row.account_id}:${row.market}:${row.symbol}`}
        size="small"
        columns={columns}
        dataSource={sorted}
        scroll={{ x: 1250 }}
        pagination={{ pageSize, showSizeChanger: true, onShowSizeChange, showTotal: (total) => `共 ${total} 只股票` }}
        locale={{ emptyText: "所选区间没有可评价的股票买卖操作" }}
      />
    </Card>
  );
}
