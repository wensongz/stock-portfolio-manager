import { useCallback, useEffect, useRef, useState } from "react";
import { Button, Modal, Table, Tag, message } from "antd";
import { invoke } from "@tauri-apps/api/core";
import dayjs from "dayjs";
import type { Transaction, TransactionType } from "../../types";
import TransactionFormModal from "../Transactions/TransactionFormModal";

const currencySymbol: Record<string, string> = {
  USD: "$",
  CNY: "¥",
  HKD: "HK$",
};

interface Props {
  open: boolean;
  accountName: string;
  symbol: string;
  stockName: string;
  accountId: string;
  onClose: () => void;
  onUpdated?: () => void | Promise<void>;
}

/**
 * Modal showing one account's transaction history for one stock.
 * Mirrors the holdings page "明细" popup (date, type, shares, price, total,
 * commission, notes).
 */
export default function AccountStockTransactionsModal({
  open,
  accountName,
  symbol,
  stockName,
  accountId,
  onClose,
  onUpdated,
}: Props) {
  const [transactions, setTransactions] = useState<Transaction[]>([]);
  const [loading, setLoading] = useState(false);
  const [editingTransaction, setEditingTransaction] = useState<Transaction | null>(null);
  const loadRequest = useRef(0);

  const load = useCallback(async () => {
    const request = ++loadRequest.current;
    setLoading(true);
    try {
      const txns = await invoke<Transaction[]>("get_transactions", {
        accountId,
        symbol,
      });
      if (request === loadRequest.current) setTransactions(txns);
    } catch (err) {
      if (request === loadRequest.current) {
        message.error(`获取交易记录失败: ${err}`);
        setTransactions([]);
      }
    } finally {
      if (request === loadRequest.current) setLoading(false);
    }
  }, [accountId, symbol]);

  useEffect(() => {
    if (open) void load();
    return () => { loadRequest.current += 1; };
  }, [open, load]);

  return (
    <>
      <Modal
        title={`交易明细 — ${stockName} (${symbol}) · ${accountName}`}
        open={open}
        onCancel={() => {
          setEditingTransaction(null);
          onClose();
        }}
        footer={null}
        width={960}
      >
        <Table<Transaction>
          dataSource={transactions}
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
              render: (type: TransactionType) => (
                <Tag color={type === "STOCK_IN" ? "cyan" : type === "STOCK_OUT" ? "purple" : type === "BUY" ? "green" : type === "OPEN" ? "blue" : type === "PAY" ? "orange" : "red"}>
                  {type === "STOCK_IN" ? "存入股票" : type === "STOCK_OUT" ? "提取股票" : type === "BUY" ? "买入" : type === "OPEN" ? "建仓" : type === "PAY" ? "分红" : "卖出"}
                </Tag>
              ),
            },
            {
              title: "股数",
              dataIndex: "shares",
              key: "shares",
              width: 90,
              render: (v: number) => v.toLocaleString(),
            },
            {
              title: "价格",
              dataIndex: "price",
              key: "price",
              width: 100,
              render: (v: number, record: Transaction) =>
                `${currencySymbol[record.currency] ?? ""}${v.toFixed(2)}`,
            },
            {
              title: "总金额",
              dataIndex: "total_amount",
              key: "total_amount",
              width: 120,
              render: (v: number, record: Transaction) =>
                `${currencySymbol[record.currency] ?? ""}${v.toFixed(2)}`,
            },
            {
              title: "手续费",
              dataIndex: "commission",
              key: "commission",
              width: 90,
              render: (v: number, record: Transaction) =>
                `${currencySymbol[record.currency] ?? ""}${v.toFixed(2)}`,
            },
            {
              title: "备注",
              dataIndex: "notes",
              key: "notes",
              render: (v: string | null) => v || "—",
            },
            {
              title: "操作",
              key: "action",
              width: 70,
              fixed: "right",
              align: "right",
              render: (_: unknown, record: Transaction) => (
                <Button
                  type="link"
                  size="small"
                  disabled={record.transaction_type === "OPEN"}
                  onClick={() => setEditingTransaction(record)}
                >
                  编辑
                </Button>
              ),
            },
          ]}
        />
      </Modal>
      <TransactionFormModal
        open={open && editingTransaction !== null}
        transaction={editingTransaction}
        onClose={() => setEditingTransaction(null)}
        onSaved={async (saved) => {
          await Promise.all([load(), onUpdated?.()]);
          // Keep the committed note when the list reload still contains the old text.
          setTransactions((rows) => rows.map((row) => row.id === saved.id ? { ...row, notes: saved.notes } : row));
        }}
      />
    </>
  );
}
