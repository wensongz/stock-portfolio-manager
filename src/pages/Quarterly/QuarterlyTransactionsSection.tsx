import { Card, Table, Tag, Typography } from "antd";
import { SwapOutlined } from "@ant-design/icons";
import dayjs from "dayjs";
import type { StockTransactionGroup, Transaction } from "../../types";

const { Text } = Typography;

const MARKET_COLORS: Record<string, string> = {
  US: "blue",
  CN: "red",
  HK: "green",
};

function fmt(v: number, decimals = 2) {
  return v.toLocaleString("en-US", {
    minimumFractionDigits: decimals,
    maximumFractionDigits: decimals,
  });
}

/** Expanded view: individual transactions for one stock within the quarter. */
function TransactionDetailTable({ transactions }: { transactions: Transaction[] }) {
  const columns = [
    {
      title: "日期",
      dataIndex: "traded_at",
      key: "traded_at",
      render: (d: string) => dayjs(d).format("YYYY-MM-DD HH:mm"),
    },
    {
      title: "类型",
      dataIndex: "transaction_type",
      key: "transaction_type",
      render: (t: string) => (
        <Tag color={t === "BUY" ? "green" : "red"}>{t === "BUY" ? "买入" : "卖出"}</Tag>
      ),
    },
    {
      title: "股数",
      dataIndex: "shares",
      key: "shares",
      render: (v: number) => v.toLocaleString(),
    },
    {
      title: "价格",
      dataIndex: "price",
      key: "price",
      render: (v: number, r: Transaction) => `${r.currency} ${fmt(v, 4)}`,
    },
    {
      title: "成交总额",
      dataIndex: "total_amount",
      key: "total_amount",
      render: (v: number, r: Transaction) => `${r.currency} ${fmt(v)}`,
    },
    {
      title: "手续费",
      dataIndex: "commission",
      key: "commission",
      render: (v: number, r: Transaction) => (v > 0 ? `${r.currency} ${fmt(v)}` : "—"),
    },
    {
      title: "备注",
      dataIndex: "notes",
      key: "notes",
      render: (v: string | null) => v ?? "—",
    },
  ];

  return (
    <Table
      dataSource={transactions}
      columns={columns}
      rowKey="id"
      size="small"
      pagination={false}
      className="ml-8"
    />
  );
}

interface Props {
  groups: StockTransactionGroup[];
  loading?: boolean;
}

/** Summary table: one row per stock, expandable to show individual transactions. */
export default function QuarterlyTransactionsSection({ groups, loading }: Props) {
  const columns = [
    {
      title: "市场",
      dataIndex: "market",
      key: "market",
      render: (m: string) => <Tag color={MARKET_COLORS[m] ?? "default"}>{m}</Tag>,
    },
    {
      title: "代码",
      dataIndex: "symbol",
      key: "symbol",
      render: (s: string) => <Text strong>{s}</Text>,
    },
    {
      title: "名称",
      dataIndex: "name",
      key: "name",
    },
    {
      title: "买入",
      key: "buy",
      render: (_: unknown, r: StockTransactionGroup) =>
        r.buy_count > 0 ? (
          <Text>
            {r.buy_count}笔 · {r.total_buy_shares.toLocaleString()}股 ·{" "}
            <Text type="success">
              {r.currency} {fmt(r.total_buy_amount)}
            </Text>
          </Text>
        ) : (
          <Text type="secondary">—</Text>
        ),
    },
    {
      title: "卖出",
      key: "sell",
      render: (_: unknown, r: StockTransactionGroup) =>
        r.sell_count > 0 ? (
          <Text>
            {r.sell_count}笔 · {r.total_sell_shares.toLocaleString()}股 ·{" "}
            <Text type="danger">
              {r.currency} {fmt(r.total_sell_amount)}
            </Text>
          </Text>
        ) : (
          <Text type="secondary">—</Text>
        ),
    },
    {
      title: "净交易股数",
      key: "net_shares",
      render: (_: unknown, r: StockTransactionGroup) => {
        const net = r.total_buy_shares - r.total_sell_shares;
        return (
          <Text style={{ color: net > 0 ? "#3f8600" : net < 0 ? "#cf1322" : undefined }}>
            {net > 0 ? "+" : ""}
            {net.toLocaleString()}
          </Text>
        );
      },
    },
  ];

  return (
    <Card
      size="small"
      title={
        <span>
          <SwapOutlined className="mr-1" />
          季度交易{" "}
          <Tag color="blue">{groups.length} 只</Tag>
        </span>
      }
    >
      <Table
        dataSource={groups}
        columns={columns}
        rowKey="symbol"
        size="small"
        loading={loading}
        pagination={false}
        expandable={{
          expandedRowRender: (record) => (
            <TransactionDetailTable transactions={record.transactions} />
          ),
          rowExpandable: (record) => record.transactions.length > 0,
        }}
        locale={{ emptyText: "本季度暂无交易记录" }}
      />
    </Card>
  );
}
