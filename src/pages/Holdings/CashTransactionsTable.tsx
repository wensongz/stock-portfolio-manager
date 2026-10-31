import { Button, Space, Table, Tag } from "antd";
import dayjs from "dayjs";
import type { CashBalanceReconciliationRow, Transaction, TransactionType } from "../../types";
import { formatMoney } from "../../lib/formatMoney";
import { formatCashDelta } from "./cashBalanceEditing";

/** Shared cash ledger view for holdings and account statistics. */
export default function CashTransactionsTable({ rows, loading, onEdit }: {
  rows: CashBalanceReconciliationRow[];
  loading: boolean;
  onEdit: (transaction: Transaction) => void;
}) {
  return (
    <Table<CashBalanceReconciliationRow>
      dataSource={rows}
      rowKey="id"
      loading={loading}
      pagination={false}
      scroll={{ x: "max-content", y: 400 }}
      columns={[
        {
          title: "日期",
          dataIndex: "traded_at",
          key: "traded_at",
          width: 160,
          render: (date: string) => dayjs(date).format("YYYY-MM-DD HH:mm"),
        },
        {
          title: "类型",
          dataIndex: "transaction_type",
          key: "transaction_type",
          width: 80,
          render: (type: TransactionType, record: CashBalanceReconciliationRow) => {
            if (type === "OPEN") return <Tag color="blue">期初</Tag>;
            if (record.symbol.startsWith("$CASH-") && (type === "BUY" || type === "SELL")) {
              return type === "BUY" ? <Tag color="blue">存入</Tag> : <Tag color="red">提取</Tag>;
            }
            // Cash flow colors reflect the cash impact of a stock trade.
            if (type === "BUY") return <Tag color="red">买入</Tag>;
            if (type === "SELL") return <Tag color="green">卖出</Tag>;
            if (type === "PAY") return <Tag color="orange">分红</Tag>;
            return <Tag>{type}</Tag>;
          },
        },
        {
          title: "股票",
          key: "stock",
          render: (_: unknown, record: CashBalanceReconciliationRow) => record.symbol.startsWith("$CASH-")
            ? <span style={{ color: "var(--color-text-secondary)" }}>—</span>
            : <Space>
                <strong>{record.symbol}</strong>
                <span className="text-sm" style={{ color: "var(--color-text-secondary)" }}>{record.name}</span>
              </Space>,
        },
        {
          title: "现金变动",
          key: "cashDelta",
          width: 160,
          align: "right",
          render: (_: unknown, record: CashBalanceReconciliationRow) => (
            <span style={{ color: record.cash_delta >= 0 ? "#16a34a" : "#dc2626", fontWeight: 500 }}>
              {formatCashDelta(record.cash_delta, record.currency)}
            </span>
          ),
        },
        {
          title: "余额",
          key: "runningBalance",
          width: 160,
          align: "right",
          render: (_: unknown, record: CashBalanceReconciliationRow) => (
            <span style={{ fontWeight: 500 }}>{formatMoney(record.running_balance, record.currency)}</span>
          ),
        },
        {
          title: "备注",
          dataIndex: "notes",
          key: "notes",
          ellipsis: true,
          render: (value: string | null) => value || "—",
        },
        {
          title: "操作",
          key: "action",
          width: 70,
          fixed: "right",
          align: "right",
          render: (_: unknown, record: CashBalanceReconciliationRow) => (
            <Button type="link" size="small" disabled={record.transaction_type === "OPEN"} onClick={() => onEdit(record)}>
              编辑
            </Button>
          ),
        },
      ]}
    />
  );
}
