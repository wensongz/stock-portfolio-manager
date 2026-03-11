import { Table, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import type { HoldingDetail } from "../../types";

const { Text } = Typography;

interface Props {
  holdings: HoldingDetail[];
  loading: boolean;
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

export default function HoldingsTable({ holdings, loading }: Props) {
  const columns: ColumnsType<HoldingDetail> = [
    {
      title: "代码",
      dataIndex: "symbol",
      key: "symbol",
      sorter: (a, b) => a.symbol.localeCompare(b.symbol),
      render: (symbol: string) => <Text strong>{symbol}</Text>,
      fixed: "left",
      width: 90,
    },
    {
      title: "名称",
      dataIndex: "name",
      key: "name",
      sorter: (a, b) => a.name.localeCompare(b.name),
      ellipsis: true,
    },
    {
      title: "账户",
      dataIndex: "account_name",
      key: "account_name",
      sorter: (a, b) => a.account_name.localeCompare(b.account_name),
      ellipsis: true,
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
      width: 90,
    },
    {
      title: "类别",
      dataIndex: "category_name",
      key: "category_name",
      render: (name: string, record: HoldingDetail) => (
        <Tag color={record.category_color}>{name}</Tag>
      ),
      width: 90,
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
      width: 110,
    },
    {
      title: "市值",
      dataIndex: "market_value",
      key: "market_value",
      sorter: (a, b) => a.market_value - b.market_value,
      defaultSortOrder: "descend" as const,
      render: (value: number, record: HoldingDetail) =>
        fmtMoney(value, record.currency),
      align: "right",
      width: 130,
    },
    {
      title: "盈亏金额",
      dataIndex: "pnl",
      key: "pnl",
      sorter: (a, b) => a.pnl - b.pnl,
      render: (pnl: number, record: HoldingDetail) => (
        <span style={{ color: pnl >= 0 ? "#22C55E" : "#EF4444" }}>
          {pnl >= 0 ? "+" : ""}
          {fmtMoney(pnl, record.currency)}
        </span>
      ),
      align: "right",
      width: 130,
    },
    {
      title: "盈亏比例",
      dataIndex: "pnl_percent",
      key: "pnl_percent",
      sorter: (a, b) => a.pnl_percent - b.pnl_percent,
      render: (pnl: number) => (
        <span style={{ color: pnl >= 0 ? "#22C55E" : "#EF4444" }}>
          {pnl >= 0 ? "+" : ""}
          {pnl.toFixed(2)}%
        </span>
      ),
      align: "right",
      width: 100,
    },
  ];

  return (
    <Table<HoldingDetail>
      columns={columns}
      dataSource={holdings}
      rowKey="id"
      loading={loading}
      scroll={{ x: 1200 }}
      size="small"
      pagination={{ pageSize: 20, showSizeChanger: true }}
      bordered
    />
  );
}
