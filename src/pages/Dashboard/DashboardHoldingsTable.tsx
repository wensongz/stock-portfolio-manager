import { useMemo, useState } from "react";
import { Button, Card, Select, Table, Tag, Tooltip, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import type { HoldingDetail } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";
import { useTablePageSize } from "../../hooks/tablePageSize";
import { formatHoldingShares, formatMoney } from "../../lib/formatMoney";
import AccountStockTransactionsModal from "../Statistics/AccountStockTransactionsModal";
import {
  aggregateDashboardHoldings,
  filterDashboardHoldings,
  type DashboardAccountHolding,
  type DashboardStockHolding,
} from "./aggregateDashboardHoldings";

const { Text } = Typography;

interface Props {
  holdings: HoldingDetail[];
  loading: boolean;
}

const marketOptions = [
  { label: "🇺🇸 美股", value: "US" },
  { label: "🇨🇳 A股", value: "CN" },
  { label: "🇭🇰 港股", value: "HK" },
];

const accountValueColumnOrder = ["shares", "market_value", "position_pct", "avg_cost", "pnl", "pnl_percent"];

function valueColumns<T extends DashboardAccountHolding | DashboardStockHolding>(
  pnlColor: (value: number) => string,
): ColumnsType<T> {
  return [
    {
      title: "持仓数量", dataIndex: "shares", key: "shares", width: 90, align: "right",
      sorter: (a, b) => a.shares - b.shares,
      render: (shares: number, record: T) => formatHoldingShares(shares, record.symbol),
    },
    {
      title: "市值", dataIndex: "market_value", key: "market_value", width: 145, align: "right",
      sorter: (a, b) => a.market_value_usd - b.market_value_usd,
      defaultSortOrder: "descend",
      render: (value: number, record: T) => formatMoney(value, record.currency),
    },
    {
      title: <Tooltip title="占当前筛选范围内全部持仓市值的比例，按统一币种计算">仓位%</Tooltip>,
      dataIndex: "position_pct", key: "position_pct", width: 90, align: "right",
      sorter: (a, b) => a.position_pct - b.position_pct,
      render: (value: number) => `${value.toFixed(2)}%`,
    },
    {
      title: "均价", dataIndex: "avg_cost", key: "avg_cost", width: 90, align: "right",
      sorter: (a, b) => a.avg_cost - b.avg_cost,
      render: (value: number) => value.toLocaleString("en-US", {
        minimumFractionDigits: 3, maximumFractionDigits: 3,
      }),
    },
    {
      title: "盈亏金额", dataIndex: "pnl", key: "pnl", width: 145, align: "right",
      sorter: (a, b) => a.pnl - b.pnl,
      render: (value: number, record: T) => (
        <span style={{ color: pnlColor(value) }}>
          {value >= 0 ? "+" : "-"}{formatMoney(Math.abs(value), record.currency)}
        </span>
      ),
    },
    {
      title: "盈亏比例", dataIndex: "pnl_percent", key: "pnl_percent", width: 95, align: "right",
      render: (value: number | null) => value == null ? <Text type="secondary">—</Text> : (
        <span style={{ color: pnlColor(value) }}>
          {value >= 0 ? "+" : ""}{value.toFixed(2)}%
        </span>
      ),
    },
  ];
}

export default function DashboardHoldingsTable({ holdings, loading }: Props) {
  const { pnlColor } = usePnlColor();
  const { pageSize, onShowSizeChange } = useTablePageSize();
  const [accountIds, setAccountIds] = useState<string[]>([]);
  const [markets, setMarkets] = useState<string[]>([]);
  const [page, setPage] = useState(1);
  const [transactionHolding, setTransactionHolding] = useState<DashboardAccountHolding | null>(null);

  const accountOptions = useMemo(() => Array.from(
    new Map(holdings.map((holding) => [holding.account_id, holding.account_name])),
    ([value, label]) => ({ value, label }),
  ), [holdings]);
  const stocks = useMemo(() => aggregateDashboardHoldings(
    filterDashboardHoldings(holdings, accountIds, markets),
  ), [holdings, accountIds, markets]);
  const stockCount = stocks.filter((stock) => !stock.symbol.startsWith("$CASH-")).length;

  const columns: ColumnsType<DashboardStockHolding> = [
    {
      title: "代码", dataIndex: "symbol", key: "symbol", width: 105, fixed: "left",
      sorter: (a, b) => a.symbol.localeCompare(b.symbol),
      render: (symbol: string) => <Text strong>{symbol}</Text>,
    },
    { title: "名称", dataIndex: "name", key: "name", width: 140, ellipsis: true },
    {
      title: "类别", dataIndex: "category_name", key: "category_name", width: 70,
      sorter: (a, b) => a.category_name.localeCompare(b.category_name),
      render: (name: string, record: DashboardStockHolding) => <Tag color={record.category_color}>{name}</Tag>,
    },
    {
      title: "现价", dataIndex: "current_price", key: "current_price", width: 100, align: "right",
      sorter: (a, b) => a.current_price - b.current_price,
      render: (value: number, record: DashboardStockHolding) => formatMoney(value, record.currency),
    },
    {
      title: <Tooltip title="行情相对上一交易日收盘价的涨跌幅">涨跌幅</Tooltip>,
      dataIndex: "daily_change_percent", key: "daily_change_percent", width: 90, align: "right",
      sorter: (a, b) => (a.daily_change_percent ?? -Infinity) - (b.daily_change_percent ?? -Infinity),
      render: (value: number | null) => value == null ? <Text type="secondary">—</Text> : (
        <Text strong style={{ color: value === 0 ? undefined : pnlColor(value) }}>
          {value > 0 ? "+" : ""}{value.toFixed(2)}%
        </Text>
      ),
    },
    ...valueColumns<DashboardStockHolding>(pnlColor),
  ];
  const accountColumns: ColumnsType<DashboardAccountHolding> = [
    { title: "账户", dataIndex: "account_name", key: "account_name", width: 160, ellipsis: true },
    ...valueColumns<DashboardAccountHolding>(pnlColor)
      .sort((a, b) => accountValueColumnOrder.indexOf(String(a.key)) - accountValueColumnOrder.indexOf(String(b.key)))
      .map((column) => ({
        ...column,
        title: column.key === "position_pct" ? "仓位" : column.title,
        sorter: undefined,
        defaultSortOrder: undefined,
      })),
    {
      title: "交易", key: "transactions", width: 80, align: "center",
      render: (_: unknown, record: DashboardAccountHolding) => (
        <Button type="link" size="small" onClick={() => setTransactionHolding(record)}>
          明细
        </Button>
      ),
    },
  ];

  return (
    <Card
      title="持仓概览"
      style={{ marginTop: 16 }}
      extra={(
        <div className="flex flex-wrap items-center justify-end gap-2">
          <Select
            aria-label="筛选市场"
            mode="multiple"
            size="small"
            allowClear
            placeholder="全部市场"
            maxTagCount="responsive"
            style={{ width: 120 }}
            options={marketOptions}
            value={markets}
            onChange={(values) => { setMarkets(values); setPage(1); }}
          />
          <Select
            aria-label="筛选账户"
            mode="multiple"
            size="small"
            allowClear
            placeholder="全部账户"
            maxTagCount="responsive"
            optionFilterProp="label"
            style={{ width: 160 }}
            options={accountOptions}
            value={accountIds}
            onChange={(values) => { setAccountIds(values); setPage(1); }}
          />
          <Text type="secondary">{stockCount} 个标的 · 展开查看分账户持仓</Text>
        </div>
      )}
    >
      <Table<DashboardStockHolding>
        columns={columns}
        dataSource={stocks}
        rowKey="key"
        loading={loading}
        size="small"
        className="account-detail-table"
        scroll={{ x: 1200 }}
        pagination={{
          current: page, pageSize, showSizeChanger: true,
          onChange: setPage, onShowSizeChange,
        }}
        expandable={{
          rowExpandable: (record) => record.accountRows.length > 0,
          expandedRowRender: (record) => (
            <Table<DashboardAccountHolding>
              columns={accountColumns}
              dataSource={record.accountRows}
              rowKey="key"
              size="small"
              pagination={false}
              className="ml-8 account-sub-table"
            />
          ),
        }}
      />
      {transactionHolding && (
        <AccountStockTransactionsModal
          open
          accountId={transactionHolding.account_id}
          accountName={transactionHolding.account_name}
          symbol={transactionHolding.symbol}
          stockName={transactionHolding.name}
          onClose={() => setTransactionHolding(null)}
        />
      )}
    </Card>
  );
}
