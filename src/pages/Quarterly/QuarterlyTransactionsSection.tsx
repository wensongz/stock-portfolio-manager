import { Card, Table, Tag, Typography } from "antd";
import { SwapOutlined } from "@ant-design/icons";
import dayjs from "dayjs";
import type { StockTransactionGroup, Transaction } from "../../types";
import { useAccountStore } from "../../stores/accountStore";
import type { Market } from "../../types";
import { formatQuarterlyMoney } from "./formatMoney";

const { Text } = Typography;

const MARKET_COLORS: Record<string, string> = {
  US: "blue",
  CN: "red",
  HK: "green",
};

/** Expanded view: individual transactions for one stock within the quarter. */
function TransactionDetailTable({ transactions }: { transactions: Transaction[] }) {
  const { accounts } = useAccountStore();
  const accountNameById = Object.fromEntries(accounts.map((a) => [a.id, a.name]));

  const columns = [
    {
      title: "账户",
      dataIndex: "account_id",
      key: "account_id",
      render: (id: string) => accountNameById[id] ?? id,
    },
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
      render: (t: string) => {
        switch (t) {
          case "STOCK_IN": return <Tag color="cyan">存入股票</Tag>;
          case "STOCK_OUT": return <Tag color="purple">提取股票</Tag>;
          case "BUY": return <Tag color="green">买入</Tag>;
          case "SELL": return <Tag color="red">卖出</Tag>;
          case "PAY": return <Tag color="orange">分红</Tag>;
          case "OPEN": return <Tag color="blue">建仓</Tag>;
          default: return <Tag>{t}</Tag>;
        }
      },
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
      render: (v: number, r: Transaction) => formatQuarterlyMoney(v, r.currency, 4),
    },
    {
      title: "成交总额",
      dataIndex: "total_amount",
      key: "total_amount",
      render: (v: number, r: Transaction) => formatQuarterlyMoney(v, r.currency),
    },
    {
      title: "手续费",
      dataIndex: "commission",
      key: "commission",
      render: (v: number, r: Transaction) => (v > 0 ? formatQuarterlyMoney(v, r.currency) : "—"),
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

const MARKET_ORDER: Market[] = ["CN", "HK", "US"];

function buildColumns() {
  return [
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
              {formatQuarterlyMoney(r.total_buy_amount, r.currency)}
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
              {formatQuarterlyMoney(r.total_sell_amount, r.currency)}
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
}

/** Per-market table with a summary footer row. */
function MarketTable({
  market,
  rows,
}: {
  market: Market;
  rows: StockTransactionGroup[];
}) {
  const columns = buildColumns();

  // Group totals by currency in case rows ever have mixed currencies.
  const totalsByCurrency = new Map<string, { buy: number; sell: number }>();
  for (const r of rows) {
    const cur = r.currency;
    const existing = totalsByCurrency.get(cur) ?? { buy: 0, sell: 0 };
    totalsByCurrency.set(cur, {
      buy: existing.buy + r.total_buy_amount,
      sell: existing.sell + r.total_sell_amount,
    });
  }

  return (
    <>
      <div style={{ marginBottom: 4 }}>
        <Tag color={MARKET_COLORS[market] ?? "default"}>{market}</Tag>
      </div>
      <Table
        dataSource={rows}
        columns={columns}
        rowKey="symbol"
        size="small"
        pagination={false}
        expandable={{
          expandedRowRender: (record) => (
            <TransactionDetailTable transactions={record.transactions} />
          ),
          rowExpandable: (record) => record.transactions.length > 0,
        }}
        summary={() => {
          const buyEntries = [...totalsByCurrency.entries()].filter(([, t]) => t.buy > 0);
          const sellEntries = [...totalsByCurrency.entries()].filter(([, t]) => t.sell > 0);
          return (
            <Table.Summary.Row>
              {/* colSpan=3: expand-toggle(0) + 代码(1) + 名称(2) */}
              <Table.Summary.Cell index={0} colSpan={3}>
                <Text strong>合计</Text>
              </Table.Summary.Cell>
              {/* index=3 aligns with 买入 column */}
              <Table.Summary.Cell index={3}>
                {buyEntries.length > 0 ? (
                  buyEntries.map(([cur, t]) => (
                    <div key={cur}>
                      <Text type="success" strong>
                        {formatQuarterlyMoney(t.buy, cur)}
                      </Text>
                    </div>
                  ))
                ) : (
                  <Text type="secondary">—</Text>
                )}
              </Table.Summary.Cell>
              {/* index=4 aligns with 卖出 column */}
              <Table.Summary.Cell index={4}>
                {sellEntries.length > 0 ? (
                  sellEntries.map(([cur, t]) => (
                    <div key={cur}>
                      <Text type="danger" strong>
                        {formatQuarterlyMoney(t.sell, cur)}
                      </Text>
                    </div>
                  ))
                ) : (
                  <Text type="secondary">—</Text>
                )}
              </Table.Summary.Cell>
              {/* index=5: 净交易股数 column — intentionally empty */}
              <Table.Summary.Cell index={5} />
            </Table.Summary.Row>
          );
        }}
      />
    </>
  );
}

/** Summary table: one row per stock grouped by market, each market with a totals footer. */
export default function QuarterlyTransactionsSection({ groups, loading }: Props) {
  const byMarket = new Map<Market, StockTransactionGroup[]>();
  for (const g of groups) {
    if (!byMarket.has(g.market)) byMarket.set(g.market, []);
    byMarket.get(g.market)!.push(g);
  }

  const markets = MARKET_ORDER.flatMap((m) => {
    const rows = byMarket.get(m);
    return rows ? [{ market: m, rows }] : [];
  });

  return (
    <Card
      size="small"
      loading={loading}
      title={
        <span>
          <SwapOutlined className="mr-1" />
          季度交易{" "}
          <Tag color="blue">{groups.length} 只</Tag>
        </span>
      }
    >
      {groups.length === 0 ? (
        <div style={{ textAlign: "center", color: "var(--color-text-tertiary)", padding: "16px 0" }}>
          本季度暂无交易记录
        </div>
      ) : (
        markets.map(({ market, rows }, idx) => (
          <div key={market} style={idx > 0 ? { marginTop: 16 } : undefined}>
            <MarketTable market={market} rows={rows} />
          </div>
        ))
      )}
    </Card>
  );
}
