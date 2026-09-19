import { useEffect, useState, useMemo, useCallback } from "react";
import {
  Typography,
  Button,
  Table,
  Space,
  Select,
  Tag,
  Popconfirm,
  message,
  Tooltip,
} from "antd";
import { PlusOutlined, EditOutlined, FilterOutlined, CameraOutlined, FileTextOutlined, SwapOutlined } from "@ant-design/icons";
import dayjs from "dayjs";
import { useTransactionStore } from "../../stores/transactionStore";
import { useAccountStore } from "../../stores/accountStore";
import type {
  Transaction,
  Market,
  TransactionType,
} from "../../types";
import ImportFromImageModal from "./ImportFromImageModal";
import ImportFromIbCsvModal from "./ImportFromIbCsvModal";
import ImportFromMoomooCsvModal from "./ImportFromMoomooCsvModal";
import ImportFromThsCsvModal from "./ImportFromThsCsvModal";
import ImportFromFirstradeCsvModal from "./ImportFromFirstradeCsvModal";
import { useTablePageSize } from "../../hooks/tablePageSize";
import TransactionFormModal from "./TransactionFormModal";

const { Title, Text } = Typography;

const marketColors: Record<Market, string> = {
  US: "blue",
  CN: "red",
  HK: "green",
};

const currencySymbol: Record<string, string> = { USD: "$", CNY: "¥", HKD: "HK$" };

export default function TransactionsPage() {
  const { transactions, loading, fetchTransactions, deleteTransaction } =
    useTransactionStore();
  const { accounts, fetchAccounts } = useAccountStore();
  const { pageSize, onShowSizeChange } = useTablePageSize();
  const [modalOpen, setModalOpen] = useState(false);
  const [editingTransaction, setEditingTransaction] = useState<Transaction | null>(null);
  const CASH_SYMBOL_PREFIX = "$CASH-";
  // Remember the last "按账户" filter selection across sessions.
  const [filterAccountId, setFilterAccountId] = useState<string | undefined>(() => {
    return localStorage.getItem("transactions_filter_account_id") || undefined;
  });
  const [stockColumnFilters, setStockColumnFilters] = useState<string[] | null>(null);
  const [importModalOpen, setImportModalOpen] = useState(false);
  const [excelImportModalOpen, setExcelImportModalOpen] = useState(false);
  const [csvImportModalOpen, setCsvImportModalOpen] = useState(false);
  const [moomooCsvImportModalOpen, setMoomooCsvImportModalOpen] = useState(false);
  const [firstradeCsvImportModalOpen, setFirstradeCsvImportModalOpen] = useState(false);

  useEffect(() => {
    fetchTransactions();
    fetchAccounts();
  }, [fetchTransactions, fetchAccounts]);

  // Clear a persisted filter that references a deleted account.
  useEffect(() => {
    if (filterAccountId && accounts.length > 0) {
      const exists = accounts.some((a) => a.id === filterAccountId);
      if (!exists) {
        setFilterAccountId(undefined);
        localStorage.removeItem("transactions_filter_account_id");
      }
    }
  }, [accounts, filterAccountId]);

  const handleFilterAccountChange = useCallback((v: string | undefined) => {
    setFilterAccountId(v);
    setStockColumnFilters(null);
    if (v) {
      localStorage.setItem("transactions_filter_account_id", v);
    } else {
      localStorage.removeItem("transactions_filter_account_id");
    }
  }, []);

  const handleEdit = (record: Transaction) => {
    setEditingTransaction(record);
    setModalOpen(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteTransaction(id);
      message.success("交易记录删除成功");
    } catch (err) {
      message.error(`删除失败: ${err}`);
    }
  };

  const accountMap = Object.fromEntries(accounts.map((a) => [a.id, a.name]));

  // Apply account filter (symbol filtering handled by Table column)
  const displayData = useMemo(() => {
    if (!filterAccountId) return transactions;
    return transactions.filter((t) => t.account_id === filterAccountId);
  }, [transactions, filterAccountId]);

  // Unique stock filters for the "股票" column header, scoped to account
  const symbolColumnFilters = useMemo(() => {
    if (!filterAccountId) return [];
    const base = transactions.filter((t) => t.account_id === filterAccountId);
    const seen = new Set<string>();
    const filters: { text: string; value: string }[] = [];
    for (const t of base) {
      if (!seen.has(t.symbol)) {
        seen.add(t.symbol);
        filters.push({ text: t.name ? `${t.symbol} ${t.name}` : t.symbol, value: t.symbol });
      }
    }
    return filters;
  }, [transactions, filterAccountId]);

  const columns = [
    {
      title: "日期",
      dataIndex: "traded_at",
      key: "traded_at",
      render: (date: string) => dayjs(date).format("YYYY-MM-DD HH:mm"),
    },
    {
      title: "股票",
      key: "stock",
      ...(filterAccountId ? {
        filters: symbolColumnFilters,
        filterSearch: true,
        filteredValue: stockColumnFilters,
        onFilter: (value: unknown, record: Transaction) => record.symbol === value,
      } : {}),
      render: (_: unknown, record: Transaction) => (
        <Space>
          <Tag color={marketColors[record.market]}>{record.market}</Tag>
          <strong>{record.symbol}</strong>
          <span className="text-sm" style={{ color: "var(--color-text-secondary)" }}>{record.name}</span>
        </Space>
      ),
    },
    ...(!filterAccountId ? [{
      title: "账户",
      dataIndex: "account_id",
      key: "account_id",
      render: (id: string) => accountMap[id] || id,
    }] : []),
    {
      title: "类型",
      dataIndex: "transaction_type",
      key: "transaction_type",
      render: (type: TransactionType, record: Transaction) => {
        const isCashRecord = record.symbol.startsWith(CASH_SYMBOL_PREFIX);
        if (isCashRecord) {
          return (
            <Tag color={record.transaction_type === "BUY" ? "green" : "red"}>
              {record.transaction_type === "BUY" ? "存入" : "提取"}
            </Tag>
          );
        }
        return (
          <Tag color={type === "STOCK_IN" ? "cyan" : type === "STOCK_OUT" ? "purple" : type === "BUY" ? "green" : type === "OPEN" ? "blue" : type === "PAY" ? "orange" : "red"}>
            {type === "STOCK_IN" ? "存入股票" : type === "STOCK_OUT" ? "提取股票" : type === "BUY" ? "买入" : type === "OPEN" ? "建仓" : type === "PAY" ? "分红" : "卖出"}
          </Tag>
        );
      },
    },
    {
      title: "股数",
      dataIndex: "shares",
      key: "shares",
      render: (v: number, record: Transaction) =>
        record.symbol.startsWith(CASH_SYMBOL_PREFIX) ? "—" : v.toLocaleString(),
    },
    {
      title: "价格",
      dataIndex: "price",
      key: "price",
      render: (v: number, record: Transaction) =>
        record.symbol.startsWith(CASH_SYMBOL_PREFIX) || record.transaction_type === "STOCK_OUT" ? "—" : `${currencySymbol[record.currency]}${v.toFixed(2)}`,
    },
    {
      title: "总金额",
      dataIndex: "total_amount",
      key: "total_amount",
      render: (v: number, record: Transaction) => record.transaction_type === "STOCK_OUT" ? "—" : `${currencySymbol[record.currency]}${v.toFixed(2)}`,
    },
    {
      title: "手续费",
      dataIndex: "commission",
      key: "commission",
      render: (v: number, record: Transaction) => `${currencySymbol[record.currency]}${v.toFixed(2)}`,
    },
    {
      title: "操作",
      key: "action",
      render: (_: unknown, record: Transaction) => (
        <Space size={2}>
          <Button
            type="link"
            size="small"
            icon={<EditOutlined />}
            onClick={() => handleEdit(record)}
            disabled={record.transaction_type === "OPEN"}
          >
            编辑
          </Button>
          <Popconfirm
            title="确认删除该交易记录？"
            onConfirm={() => handleDelete(record.id)}
            okText="确认"
            cancelText="取消"
          >
            <Button type="link" size="small" danger disabled={record.transaction_type === "OPEN"}>
              删除
            </Button>
          </Popconfirm>
        </Space>
      ),
    },
  ];

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <Title level={2} className="!mb-0">
          <SwapOutlined style={{ color: "#fa8c16" }} /> 交易记录
        </Title>
        <Space>
          {filterAccountId && (() => {
            const acct = accounts.find((a) => a.id === filterAccountId);
            if (!acct) return null;
            if (acct.market === "CN") {
              return (
                <>
                  <Button
                    icon={<FileTextOutlined />}
                    onClick={() => setExcelImportModalOpen(true)}
                  >
                    从CSV导入
                  </Button>
                  <Button
                    icon={<CameraOutlined />}
                    onClick={() => setImportModalOpen(true)}
                  >
                    从截图导入
                  </Button>
                </>
              );
            }
            if (acct.name.toLowerCase().includes("moomoo")) {
              return (
                <Button
                  icon={<FileTextOutlined />}
                  onClick={() => setMoomooCsvImportModalOpen(true)}
                >
                  从 CSV 导入
                </Button>
              );
            }
            if (acct.name.toLowerCase().includes("firstrade")) {
              return (
                <Button
                  icon={<FileTextOutlined />}
                  onClick={() => setFirstradeCsvImportModalOpen(true)}
                >
                  从 CSV 导入
                </Button>
              );
            }
            return (
              <Tooltip title="可以导入交易记录或分红记录">
                <Button
                  icon={<FileTextOutlined />}
                  onClick={() => setCsvImportModalOpen(true)}
                >
                  从CSV导入
                </Button>
              </Tooltip>
            );
          })()}
          <Button
            type="primary"
            icon={<PlusOutlined />}
            onClick={() => {
              setEditingTransaction(null);
              setModalOpen(true);
            }}
          >
            录入交易
          </Button>
        </Space>
      </div>

      <div className="mb-4">
        <Space size="middle">
          <Space>
            <FilterOutlined />
            <Text type="secondary">按账户:</Text>
            <Select
              value={filterAccountId}
              onChange={handleFilterAccountChange}
              placeholder="全部账户"
              allowClear
              style={{ width: 180 }}
            >
              {accounts.map((a) => (
                <Select.Option key={a.id} value={a.id}>
                  [{a.market}] {a.name}
                </Select.Option>
              ))}
            </Select>
          </Space>
        </Space>
      </div>

      <Table
        dataSource={displayData}
        columns={columns}
        rowKey="id"
        loading={loading}
        pagination={{ pageSize, showSizeChanger: true, onShowSizeChange }}
        scroll={{ x: "max-content" }}
        onChange={(_pagination, filters) => {
          const stockFilters = filters["stock"];
          setStockColumnFilters(stockFilters ? (stockFilters as string[]) : null);
        }}
      />

      <TransactionFormModal
        open={modalOpen}
        transaction={editingTransaction}
        initialAccountId={filterAccountId}
        onClose={() => {
          setModalOpen(false);
          setEditingTransaction(null);
        }}
      />

      {/* Import from screenshot modal – only for CN accounts */}
      {filterAccountId && (() => {
        const account = accounts.find((a) => a.id === filterAccountId);
        return account && account.market === "CN" ? (
          <ImportFromImageModal
            open={importModalOpen}
            account={account}
            onClose={() => setImportModalOpen(false)}
            onImported={() => {
              setImportModalOpen(false);
              fetchTransactions();
            }}
          />
        ) : null;
      })()}

      {/* Import from THS CSV modal – only for CN accounts */}
      {filterAccountId && (() => {
        const account = accounts.find((a) => a.id === filterAccountId);
        return account && account.market === "CN" ? (
          <ImportFromThsCsvModal
            open={excelImportModalOpen}
            account={account}
            onClose={() => setExcelImportModalOpen(false)}
            onImported={() => {
              setExcelImportModalOpen(false);
              fetchTransactions();
            }}
          />
        ) : null;
      })()}

      {/* Import from CSV modal – only for US/HK accounts (not moomoo, not firstrade) */}
      {filterAccountId && (() => {
        const account = accounts.find((a) => a.id === filterAccountId);
        return account &&
          (account.market === "US" || account.market === "HK") &&
          !account.name.toLowerCase().includes("moomoo") &&
          !account.name.toLowerCase().includes("firstrade") ? (
          <ImportFromIbCsvModal
            open={csvImportModalOpen}
            account={account}
            onClose={() => setCsvImportModalOpen(false)}
            onImported={() => {
              setCsvImportModalOpen(false);
              fetchTransactions();
            }}
          />
        ) : null;
      })()}

      {/* Import from moomoo CSV modal */}
      {filterAccountId && (() => {
        const account = accounts.find((a) => a.id === filterAccountId);
        return account && account.name.toLowerCase().includes("moomoo") ? (
          <ImportFromMoomooCsvModal
            open={moomooCsvImportModalOpen}
            account={account}
            onClose={() => setMoomooCsvImportModalOpen(false)}
            onImported={() => {
              setMoomooCsvImportModalOpen(false);
              fetchTransactions();
            }}
          />
        ) : null;
      })()}

      {/* Import from Firstrade CSV modal */}
      {filterAccountId && (() => {
        const account = accounts.find((a) => a.id === filterAccountId);
        return account && account.name.toLowerCase().includes("firstrade") ? (
          <ImportFromFirstradeCsvModal
            open={firstradeCsvImportModalOpen}
            account={account}
            onClose={() => setFirstradeCsvImportModalOpen(false)}
            onImported={() => {
              setFirstradeCsvImportModalOpen(false);
              fetchTransactions();
            }}
          />
        ) : null;
      })()}
    </div>
  );
}
