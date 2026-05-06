import { useEffect, useState, useMemo, useCallback } from "react";
import {
  Typography,
  Button,
  Table,
  Space,
  Modal,
  Form,
  Input,
  Select,
  InputNumber,
  Tag,
  Popconfirm,
  message,
  DatePicker,
  AutoComplete,
} from "antd";
import { PlusOutlined, EditOutlined, FilterOutlined, CameraOutlined, FileTextOutlined } from "@ant-design/icons";
import dayjs from "dayjs";
import { invoke } from "@tauri-apps/api/core";
import { useTransactionStore } from "../../stores/transactionStore";
import { useAccountStore } from "../../stores/accountStore";
import type { Transaction, Market, Currency, TransactionType, Holding, StockQuote } from "../../types";
import ImportFromImageModal from "./ImportFromImageModal";
import ImportFromIbCsvModal from "./ImportFromIbCsvModal";

const { Title, Text } = Typography;

const marketColors: Record<Market, string> = {
  US: "blue",
  CN: "red",
  HK: "green",
};

const marketCurrencyMap: Record<Market, Currency> = {
  US: "USD",
  CN: "CNY",
  HK: "HKD",
};

// Default traded time: 1 hour after market open
// US: 9:30 ET → 10:30 ET (use local hour 10, min 30)
// CN: 9:30 CST → 10:30 CST (use local hour 10, min 30)
// HK: 9:30 HKT → 10:30 HKT (use local hour 10, min 30)
const marketDefaultTime: Record<Market, { hour: number; minute: number }> = {
  US: { hour: 10, minute: 30 },
  CN: { hour: 10, minute: 30 },
  HK: { hour: 10, minute: 30 },
};

function getDefaultTradedAt(market?: Market): dayjs.Dayjs {
  const time = market ? marketDefaultTime[market] : { hour: 10, minute: 30 };
  return dayjs().hour(time.hour).minute(time.minute).second(0);
}

export default function TransactionsPage() {
  const { transactions, loading, fetchTransactions, createTransaction, updateTransaction, deleteTransaction } =
    useTransactionStore();
  const { accounts, fetchAccounts } = useAccountStore();
  const [modalOpen, setModalOpen] = useState(false);
  const [editingTransaction, setEditingTransaction] = useState<Transaction | null>(null);
  const [form] = Form.useForm();
  const [accountHoldings, setAccountHoldings] = useState<Holding[]>([]);
  const [symbolSearching, setSymbolSearching] = useState(false);
  const [filterAccountId, setFilterAccountId] = useState<string | undefined>(undefined);
  const [importModalOpen, setImportModalOpen] = useState(false);
  const [csvImportModalOpen, setCsvImportModalOpen] = useState(false);

  useEffect(() => {
    fetchTransactions();
    fetchAccounts();
  }, [fetchTransactions, fetchAccounts]);

  // When account changes, set default market/currency/time and load holdings
  const handleAccountChange = useCallback(async (accountId: string) => {
    const account = accounts.find((a) => a.id === accountId);
    if (account) {
      form.setFieldsValue({
        market: account.market,
        currency: marketCurrencyMap[account.market],
        tradedAt: getDefaultTradedAt(account.market),
      });
      try {
        const holdings = await invoke<Holding[]>("get_holdings", { accountId });
        setAccountHoldings(holdings);
      } catch {
        setAccountHoldings([]);
      }
    }
  }, [accounts, form]);

  // Build AutoComplete options from holdings
  const symbolOptions = useMemo(() => {
    return accountHoldings
      .filter((h) => h.shares > 0)
      .map((h) => ({
        value: h.symbol,
        label: `${h.symbol} - ${h.name} (持仓: ${h.shares})`,
      }));
  }, [accountHoldings]);

  // When a symbol is selected from dropdown, auto-fill name/market/currency
  const handleSymbolSelect = useCallback((value: string) => {
    const holding = accountHoldings.find((h) => h.symbol === value);
    if (holding) {
      form.setFieldsValue({
        name: holding.name,
        market: holding.market,
        currency: holding.currency,
      });
    }
  }, [accountHoldings, form]);

  // When user finishes typing a symbol (on blur), try to look up the stock name
  const handleSymbolBlur = useCallback(async () => {
    const symbol = form.getFieldValue("symbol");
    const name = form.getFieldValue("name");
    const market = form.getFieldValue("market") as Market | undefined;
    if (!symbol || name || !market) return;

    // Check holdings first
    const holding = accountHoldings.find(
      (h) => h.symbol.toUpperCase() === symbol.toUpperCase()
    );
    if (holding) {
      form.setFieldsValue({ name: holding.name });
      return;
    }

    // Try to fetch quote from backend
    setSymbolSearching(true);
    try {
      const quotes = await invoke<StockQuote[]>("get_real_time_quotes", {
        symbols: [[symbol, market]],
        forceRefresh: false,
      });
      if (quotes.length > 0 && quotes[0].name) {
        form.setFieldsValue({ name: quotes[0].name });
      }
    } catch {
      // silently ignore - user can enter name manually
    } finally {
      setSymbolSearching(false);
    }
  }, [accountHoldings, form]);

  // Auto-calculate total amount when shares or price changes
  const handleAmountFieldChange = useCallback(() => {
    const shares = form.getFieldValue("shares");
    const price = form.getFieldValue("price");
    if (typeof shares === "number" && typeof price === "number" && shares > 0 && price > 0) {
      form.setFieldsValue({
        totalAmount: Math.round(shares * price * 100) / 100,
      });
    }
  }, [form]);

  const handleSubmit = async (values: {
    accountId: string;
    symbol: string;
    name: string;
    market: Market;
    transactionType: TransactionType;
    shares: number;
    price: number;
    totalAmount: number;
    commission: number;
    currency: Currency;
    tradedAt: dayjs.Dayjs;
    notes?: string;
  }) => {
    try {
      if (editingTransaction) {
        await updateTransaction({
          id: editingTransaction.id,
          ...values,
          tradedAt: values.tradedAt.toISOString(),
        });
        message.success("交易记录更新成功");
      } else {
        await createTransaction({
          ...values,
          tradedAt: values.tradedAt.toISOString(),
        });
        message.success("交易记录添加成功");
      }
      setModalOpen(false);
      setEditingTransaction(null);
      form.resetFields();
    } catch (err) {
      message.error(`操作失败: ${err}`);
    }
  };

  const handleEdit = async (record: Transaction) => {
    setEditingTransaction(record);
    // Load holdings for the account
    try {
      const holdings = await invoke<Holding[]>("get_holdings", { accountId: record.account_id });
      setAccountHoldings(holdings);
    } catch {
      setAccountHoldings([]);
    }
    form.setFieldsValue({
      accountId: record.account_id,
      symbol: record.symbol,
      name: record.name,
      market: record.market,
      transactionType: record.transaction_type,
      shares: record.shares,
      price: record.price,
      totalAmount: record.total_amount,
      commission: record.commission,
      currency: record.currency,
      tradedAt: dayjs(record.traded_at),
      notes: record.notes,
    });
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

  // Apply account filter
  const displayData = useMemo(() => {
    if (!filterAccountId) return transactions;
    return transactions.filter((t) => t.account_id === filterAccountId);
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
      render: (_: unknown, record: Transaction) => (
        <Space>
          <Tag color={marketColors[record.market]}>{record.market}</Tag>
          <strong>{record.symbol}</strong>
          <span className="text-gray-500 text-sm">{record.name}</span>
        </Space>
      ),
    },
    {
      title: "账户",
      dataIndex: "account_id",
      key: "account_id",
      render: (id: string) => accountMap[id] || id,
    },
    {
      title: "类型",
      dataIndex: "transaction_type",
      key: "transaction_type",
      render: (type: TransactionType) => (
        <Tag color={type === "BUY" ? "green" : type === "OPEN" ? "blue" : "red"}>
          {type === "BUY" ? "买入" : type === "OPEN" ? "建仓" : "卖出"}
        </Tag>
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
      render: (v: number, record: Transaction) => `${record.currency} ${v.toFixed(2)}`,
    },
    {
      title: "总金额",
      dataIndex: "total_amount",
      key: "total_amount",
      render: (v: number, record: Transaction) => `${record.currency} ${v.toFixed(2)}`,
    },
    {
      title: "操作",
      key: "action",
      render: (_: unknown, record: Transaction) => (
        <Space>
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
          交易记录
        </Title>
        <Space>
          {filterAccountId && (() => {
            const acct = accounts.find((a) => a.id === filterAccountId);
            if (!acct) return null;
            if (acct.market === "CN") {
              return (
                <Button
                  icon={<CameraOutlined />}
                  onClick={() => setImportModalOpen(true)}
                >
                  从截图导入
                </Button>
              );
            }
            return (
              <Button
                icon={<FileTextOutlined />}
                onClick={() => setCsvImportModalOpen(true)}
              >
                从CSV导入
              </Button>
            );
          })()}
          <Button
            type="primary"
            icon={<PlusOutlined />}
            onClick={() => {
              setEditingTransaction(null);
              form.resetFields();
              setAccountHoldings([]);
              if (filterAccountId) {
                form.setFieldsValue({ accountId: filterAccountId });
                handleAccountChange(filterAccountId);
              }
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
              onChange={setFilterAccountId}
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
        pagination={{ pageSize: 20 }}
      />

      <Modal
        title={editingTransaction ? "编辑交易记录" : "录入交易记录"}
        open={modalOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setModalOpen(false);
          setEditingTransaction(null);
          form.resetFields();
          setAccountHoldings([]);
        }}
        okText="确认"
        cancelText="取消"
        width={640}
      >
        <Form form={form} layout="vertical" onFinish={handleSubmit}
          initialValues={{ tradedAt: getDefaultTradedAt(), commission: 0 }}>
          <Form.Item name="accountId" label="证券账户"
            rules={[{ required: true, message: "请选择账户" }]}>
            <Select placeholder="选择证券账户" onChange={handleAccountChange}>
              {accounts.map((a) => (
                <Select.Option key={a.id} value={a.id}>
                  [{a.market}] {a.name}
                </Select.Option>
              ))}
            </Select>
          </Form.Item>
          <Form.Item name="symbol" label="股票代码"
            rules={[{ required: true, message: "请输入股票代码" }]}>
            <AutoComplete
              options={symbolOptions}
              placeholder="输入或选择股票代码"
              onSelect={handleSymbolSelect}
              onBlur={handleSymbolBlur}
              filterOption={(inputValue, option) =>
                (option?.value?.toString().toUpperCase().indexOf(inputValue.toUpperCase()) ?? -1) >= 0 ||
                (option?.label?.toString().toUpperCase().indexOf(inputValue.toUpperCase()) ?? -1) >= 0
              }
            />
          </Form.Item>
          <Form.Item name="name" label="股票名称"
            rules={[{ required: true, message: "请输入股票名称" }]}>
            <Input placeholder="如：苹果" disabled={symbolSearching} />
          </Form.Item>
          <Form.Item name="market" label="市场"
            rules={[{ required: true, message: "请选择市场" }]}>
            <Select placeholder="选择市场">
              <Select.Option value="US">🇺🇸 美股</Select.Option>
              <Select.Option value="CN">🇨🇳 A股</Select.Option>
              <Select.Option value="HK">🇭🇰 港股</Select.Option>
            </Select>
          </Form.Item>
          <Form.Item name="transactionType" label="交易类型"
            rules={[{ required: true, message: "请选择交易类型" }]}>
            <Select placeholder="买入 / 卖出">
              <Select.Option value="BUY">买入</Select.Option>
              <Select.Option value="SELL">卖出</Select.Option>
            </Select>
          </Form.Item>
          <Form.Item name="shares" label="交易股数"
            rules={[{ required: true, message: "请输入交易股数" }]}>
            <InputNumber min={1} precision={0} style={{ width: "100%" }}
              onChange={handleAmountFieldChange} />
          </Form.Item>
          <Form.Item name="price" label="成交价格"
            rules={[{ required: true, message: "请输入成交价格" }]}>
            <InputNumber min={0} precision={4} style={{ width: "100%" }}
              onChange={handleAmountFieldChange} />
          </Form.Item>
          <Form.Item name="totalAmount" label="成交总额"
            rules={[{ required: true, message: "请输入成交总额" }]}>
            <InputNumber min={0} precision={2} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="commission" label="手续费">
            <InputNumber min={0} precision={2} style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="currency" label="币种"
            rules={[{ required: true, message: "请选择币种" }]}>
            <Select placeholder="选择币种">
              <Select.Option value="USD">USD 美元</Select.Option>
              <Select.Option value="CNY">CNY 人民币</Select.Option>
              <Select.Option value="HKD">HKD 港元</Select.Option>
            </Select>
          </Form.Item>
          <Form.Item name="tradedAt" label="成交时间"
            rules={[{ required: true, message: "请选择成交时间" }]}>
            <DatePicker showTime style={{ width: "100%" }} />
          </Form.Item>
          <Form.Item name="notes" label="备注（可选）">
            <Input.TextArea rows={3} placeholder="交易备注" />
          </Form.Item>
        </Form>
      </Modal>

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

      {/* Import from CSV modal – only for US/HK accounts */}
      {filterAccountId && (() => {
        const account = accounts.find((a) => a.id === filterAccountId);
        return account && (account.market === "US" || account.market === "HK") ? (
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
    </div>
  );
}
