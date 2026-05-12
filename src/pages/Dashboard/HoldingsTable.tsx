import { useMemo, useState } from "react";
import { Table, Tag, Typography } from "antd";
import type { ColumnsType, TableProps } from "antd/es/table";
import type { HoldingDetail } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";

const { Text } = Typography;

interface Props {
  holdings: HoldingDetail[];
  loading: boolean;
  hideAccountMarket?: boolean;
}

const marketLabel: Record<string, string> = {
  US: "🇺🇸 US",
  CN: "🇨🇳 CN",
  HK: "🇭🇰 HK",
};

const currencySymbol: Record<string, string> = { USD: "$", CNY: "¥", HKD: "HK$" };

function fmtMoney(value: number, currency: string) {
  return `${currencySymbol[currency] ?? ""}${value.toLocaleString("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`;
}

export default function HoldingsTable({ holdings, loading, hideAccountMarket = false }: Props) {
  const { pnlColor } = usePnlColor();

  // Track which filter values are currently active for account and market columns.
  // This lets us recompute the denominator whenever holdings or filters change.
  const [activeAccountFilter, setActiveAccountFilter] = useState<string[] | null>(null);
  const [activeMarketFilter, setActiveMarketFilter] = useState<string[] | null>(null);

  const filteredTotalMvUsd = useMemo(() => {
    const visible = holdings.filter((h) => {
      if (activeAccountFilter && activeAccountFilter.length > 0 && !activeAccountFilter.includes(h.account_name))
        return false;
      if (activeMarketFilter && activeMarketFilter.length > 0 && !activeMarketFilter.includes(h.market))
        return false;
      return true;
    });
    return visible.reduce((sum, h) => sum + h.market_value_usd, 0);
  }, [holdings, activeAccountFilter, activeMarketFilter]);

  const handleTableChange: TableProps<HoldingDetail>["onChange"] = (_pagination, filters) => {
    const accountVals = filters["account_name"];
    const marketVals = filters["market"];
    setActiveAccountFilter(accountVals ? (accountVals as string[]) : null);
    setActiveMarketFilter(marketVals ? (marketVals as string[]) : null);
  };

  const accountFilters = useMemo(
    () =>
      Array.from(new Set(holdings.map((h) => h.account_name))).map((name) => ({
        text: name,
        value: name,
      })),
    [holdings]
  );

  const columns: ColumnsType<HoldingDetail> = useMemo(() => {
    const allColumns: ColumnsType<HoldingDetail> = [
    {
      title: "代码",
      dataIndex: "symbol",
      key: "symbol",
      sorter: (a, b) => a.symbol.localeCompare(b.symbol),
      render: (symbol: string) => <Text strong>{symbol}</Text>,
      fixed: "left",
      width: 110,
    },
    {
      title: "名称",
      dataIndex: "name",
      key: "name",
      ellipsis: true,
      width: 140,
    },
    {
      title: "账户",
      dataIndex: "account_name",
      key: "account_name",
      filters: accountFilters,
      onFilter: (value, record) => record.account_name === value,
      ellipsis: true,
      width: 120,
    },
    {
      title: "市场",
      dataIndex: "market",
      key: "market",
      render: (market: string) => marketLabel[market] ?? market,
      filters: [
        { text: "🇺🇸 美股", value: "US" },
        { text: "🇨🇳 A股", value: "CN" },
        { text: "🇭🇰 港股", value: "HK" },
      ],
      onFilter: (value, record) => record.market === value,
      width: 80,
    },
    {
      title: "类别",
      dataIndex: "category_name",
      key: "category_name",
      sorter: (a, b) => a.category_name.localeCompare(b.category_name),
      render: (name: string, record: HoldingDetail) => (
        <Tag color={record.category_color}>{name}</Tag>
      ),
      width: 80,
    },
    {
      title: "持仓数量",
      dataIndex: "shares",
      key: "shares",
      sorter: (a, b) => a.shares - b.shares,
      render: (shares: number) => shares.toLocaleString(),
      align: "right",
      width: 100,
    },
    {
      title: "均价",
      dataIndex: "avg_cost",
      key: "avg_cost",
      sorter: (a, b) => a.avg_cost - b.avg_cost,
      render: (price: number, record: HoldingDetail) =>
        `${currencySymbol[record.currency] ?? ""}${price.toLocaleString("en-US", {
          minimumFractionDigits: 3,
          maximumFractionDigits: 3,
        })}`,
      align: "right",
      width: 110,
    },
    {
      title: "现价",
      dataIndex: "current_price",
      key: "current_price",
      sorter: (a, b) => a.current_price - b.current_price,
      render: (price: number, record: HoldingDetail) =>
        fmtMoney(price, record.currency),
      align: "right",
      width: 100,
    },
    {
      title: "市值",
      dataIndex: "market_value",
      key: "market_value",
      sorter: (a, b) => a.market_value_usd - b.market_value_usd,
      defaultSortOrder: "descend" as const,
      render: (value: number, record: HoldingDetail) =>
        fmtMoney(value, record.currency),
      align: "right",
      width: 140,
    },
    {
      title: "仓位%",
      key: "position_pct",
      sorter: (a, b) => a.market_value_usd - b.market_value_usd,
      render: (_: unknown, record: HoldingDetail) => {
        const pct = filteredTotalMvUsd > 0 ? (record.market_value_usd / filteredTotalMvUsd) * 100 : 0;
        return `${pct.toFixed(2)}%`;
      },
      align: "right",
      width: 90,
    },
    {
      title: "盈亏金额",
      dataIndex: "pnl",
      key: "pnl",
      sorter: (a, b) => a.pnl - b.pnl,
      render: (pnl: number, record: HoldingDetail) => (
        <span style={{ color: pnlColor(pnl) }}>
          {pnl >= 0 ? "+" : ""}
          {fmtMoney(pnl, record.currency)}
        </span>
      ),
      align: "right",
      width: 140,
    },
    {
      title: "盈亏比例",
      dataIndex: "pnl_percent",
      key: "pnl_percent",
      sorter: (a, b) => a.pnl_percent - b.pnl_percent,
      render: (pnl: number) => (
        <span style={{ color: pnlColor(pnl) }}>
          {pnl >= 0 ? "+" : ""}
          {pnl.toFixed(2)}%
        </span>
      ),
      align: "right",
      width: 100,
    },
    ];
    return hideAccountMarket
      ? allColumns.filter((c) => c.key !== "account_name" && c.key !== "market")
      : allColumns;
  }, [accountFilters, filteredTotalMvUsd, pnlColor, hideAccountMarket]);

  return (
    <Table<HoldingDetail>
      columns={columns}
      dataSource={holdings}
      rowKey="id"
      loading={loading}
      scroll={{ x: hideAccountMarket ? 1100 : 1310 }}
      size="small"
      pagination={{ pageSize: 20, showSizeChanger: true }}
      bordered
      onChange={handleTableChange}
    />
  );
}
