import { useEffect, useState, useCallback, useMemo, useRef } from "react";
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
  Tooltip,
  Switch,
  Spin,
  AutoComplete,
} from "antd";
import { PlusOutlined, ReloadOutlined, SyncOutlined, FilterOutlined, DollarOutlined } from "@ant-design/icons";
import { invoke } from "@tauri-apps/api/core";
import { useHoldingStore } from "../../stores/holdingStore";
import { useAccountStore } from "../../stores/accountStore";
import { useCategoryStore } from "../../stores/categoryStore";
import { useQuoteStore } from "../../stores/quoteStore";
import type { Holding, HoldingWithQuote, Market, Currency, StockQuote } from "../../types";
import dayjs from "dayjs";

const { Title, Text } = Typography;

/** Cash symbol prefix – must match the backend constant. */
const CASH_SYMBOL_PREFIX = "$CASH-";

/** Map market → cash symbol */
const MARKET_CASH_SYMBOL: Record<Market, string> = {
  US: "$CASH-USD",
  CN: "$CASH-CNY",
  HK: "$CASH-HKD",
};

/** Returns true if the symbol represents a cash holding. */
function isCashSymbol(symbol: string): boolean {
  return symbol.startsWith(CASH_SYMBOL_PREFIX);
}

/** Shared formatting options for displaying currency amounts. */
const CURRENCY_FORMAT_OPTIONS: Intl.NumberFormatOptions = {
  minimumFractionDigits: 2,
  maximumFractionDigits: 2,
};

const marketColors: Record<Market, string> = {
  US: "blue",
  CN: "red",
  HK: "green",
};

function PnlText({ value, percent }: { value: number | null; percent: number | null }) {
  if (value === null || value === undefined) return <span>—</span>;
  const isPositive = value >= 0;
  const color = isPositive ? "#3f8600" : "#cf1322";
  const sign = isPositive ? "+" : "";
  return (
    <span style={{ color }}>
      {sign}{value.toFixed(2)}
      {percent !== null && (
        <> ({sign}{percent.toFixed(2)}%)</>
      )}
    </span>
  );
}

export default function HoldingsPage() {
  const { holdings, loading: holdingsLoading, fetchHoldings, createHolding, updateHolding, deleteHolding } =
    useHoldingStore();
  const { accounts, fetchAccounts } = useAccountStore();
  const { categories, fetchCategories } = useCategoryStore();
  const { holdingQuotes, loading: quotesLoading, lastUpdatedAt, refreshIntervalMs, fetchHoldingQuotes } = useQuoteStore();

  const [modalOpen, setModalOpen] = useState(false);
  const [cashModalOpen, setCashModalOpen] = useState(false);
  const [editingHolding, setEditingHolding] = useState<Holding | null>(null);
  const [showRealtime, setShowRealtime] = useState(true);
  const [form] = Form.useForm();
  const [cashForm] = Form.useForm();
  const [fetchingName, setFetchingName] = useState(false);
  const [filterAccountId, setFilterAccountId] = useState<string | undefined>(undefined);
  const [filterMarket, setFilterMarket] = useState<Market | undefined>(undefined);
  const [symbolSearch, setSymbolSearch] = useState("");
  const displayDataRef = useRef<HoldingWithQuote[]>([]);

  // Derive unique stock symbols from existing holdings for autocomplete
  const symbolOptions = useMemo(() => {
    const seen = new Set<string>();
    const options: { symbol: string; name: string; market: Market; currency: Currency }[] = [];
    for (const h of holdings) {
      if (!seen.has(h.symbol)) {
        seen.add(h.symbol);
        options.push({ symbol: h.symbol, name: h.name, market: h.market, currency: h.currency });
      }
    }
    return options;
  }, [holdings]);

  // Filter autocomplete options: show only when input >= 3 characters
  const filteredSymbolOptions = useMemo(() => {
    if (symbolSearch.length < 3) return [];
    const search = symbolSearch.toLowerCase();
    return symbolOptions
      .filter((o) => o.symbol.toLowerCase().includes(search))
      .map((o) => ({
        value: o.symbol,
        label: `${o.symbol} - ${o.name}`,
      }));
  }, [symbolSearch, symbolOptions]);

  const marketToCurrency: Record<Market, Currency> = {
    US: "USD",
    CN: "CNY",
    HK: "HKD",
  };

  const handleAccountChange = useCallback(
    (accountId: string) => {
      const account = accounts.find((a) => a.id === accountId);
      if (account) {
        form.setFieldsValue({
          market: account.market,
          currency: marketToCurrency[account.market],
        });
      }
    },
    [accounts, form],
  );

  const fetchStockName = useCallback(
    async (symbol: string) => {
      if (!symbol || !symbol.trim()) return;
      // Cash symbols don't need an API call for the name.
      if (isCashSymbol(symbol.trim())) return;
      const market: Market | undefined = form.getFieldValue("market");
      if (!market) return;

      setFetchingName(true);
      try {
        const commandMap: Record<Market, string> = {
          US: "get_us_quote",
          CN: "get_cn_quote",
          HK: "get_hk_quote",
        };
        const quote = await invoke<StockQuote>(commandMap[market], {
          symbol: symbol.trim(),
        });
        if (quote && quote.name) {
          form.setFieldsValue({ name: quote.name });
        }
      } catch {
        // Silently ignore - user can still type the name manually
      } finally {
        setFetchingName(false);
      }
    },
    [form],
  );

  const handleSymbolBlur = useCallback(
    () => {
      const symbol = form.getFieldValue("symbol");
      if (symbol) fetchStockName(symbol);
    },
    [fetchStockName, form],
  );

  const handleSymbolSelect = useCallback(
    (symbol: string) => {
      const match = symbolOptions.find((o) => o.symbol === symbol);
      if (match) {
        form.setFieldsValue({
          name: match.name,
          market: match.market,
          currency: match.currency,
        });
      }
    },
    [symbolOptions, form],
  );

  useEffect(() => {
    fetchHoldings();
    fetchAccounts();
    fetchCategories();
  }, [fetchHoldings, fetchAccounts, fetchCategories]);

  // Auto-refresh quotes at configured interval when realtime is enabled
  useEffect(() => {
    if (!showRealtime) return;
    const { startAutoRefresh } = useQuoteStore.getState();
    return startAutoRefresh(() => {
      const seen = new Set<string>();
      const symbols: [string, string][] = [];
      for (const h of displayDataRef.current) {
        if (!seen.has(h.symbol)) {
          seen.add(h.symbol);
          symbols.push([h.symbol, h.market]);
        }
      }
      return symbols;
    });
  }, [showRealtime, refreshIntervalMs]);

  const handleSubmit = async (values: {
    accountId: string;
    symbol: string;
    name: string;
    market: Market;
    categoryId?: string;
    shares: number;
    avgCost: number;
    currency: Currency;
  }) => {
    try {
      if (editingHolding) {
        await updateHolding({ id: editingHolding.id, ...values });
        message.success("持仓更新成功");
      } else {
        await createHolding(values);
        message.success("持仓创建成功");
      }
      setModalOpen(false);
      form.resetFields();
      setEditingHolding(null);
      setSymbolSearch("");
    } catch (err) {
      message.error(`操作失败: ${err}`);
    }
  };

  /** Submit handler for the "Add Cash" modal. */
  const handleCashSubmit = async (values: { accountId: string; amount: number }) => {
    try {
      const account = accounts.find((a) => a.id === values.accountId);
      if (!account) {
        message.error("账户不存在");
        return;
      }
      const market = account.market as Market;
      const currency = marketToCurrency[market];
      const cashSymbol = MARKET_CASH_SYMBOL[market];
      // Find the "现金类" category if available
      const cashCategory = categories.find((c) => c.name === "现金类");
      await createHolding({
        accountId: values.accountId,
        symbol: cashSymbol,
        name: `现金 (${currency})`,
        market,
        categoryId: cashCategory?.id,
        shares: values.amount,
        avgCost: 1,
        currency,
      });
      message.success("现金持仓创建成功");
      setCashModalOpen(false);
      cashForm.resetFields();
    } catch (err) {
      message.error(`操作失败: ${err}`);
    }
  };

  const handleEdit = (holding: Holding) => {
    setEditingHolding(holding);
    form.setFieldsValue({
      accountId: holding.account_id,
      symbol: holding.symbol,
      name: holding.name,
      market: holding.market,
      categoryId: holding.category_id,
      shares: holding.shares,
      avgCost: holding.avg_cost,
      currency: holding.currency,
    });
    setModalOpen(true);
  };

  const handleDelete = async (id: string) => {
    try {
      await deleteHolding(id);
      message.success("持仓删除成功");
    } catch (err) {
      message.error(`删除失败: ${err}`);
    }
  };

  const accountMap = Object.fromEntries(accounts.map((a) => [a.id, a.name]));
  const categoryMap = Object.fromEntries(categories.map((c) => [c.id, c]));

  // Merge holdings with realtime quotes
  const quoteMap = Object.fromEntries(holdingQuotes.map((h) => [h.id, h as HoldingWithQuote]));
  const allDisplayData: HoldingWithQuote[] = holdings.map((h) => quoteMap[h.id] ?? {
    ...h,
    quote: null,
    market_value: null,
    total_cost: null,
    unrealized_pnl: null,
    unrealized_pnl_percent: null,
  });

  // Apply filters
  const displayData = allDisplayData.filter((h) => {
    if (filterAccountId && h.account_id !== filterAccountId) return false;
    if (filterMarket && h.market !== filterMarket) return false;
    return true;
  });
  displayDataRef.current = displayData;

  // Extract unique (symbol, market) pairs from the visible holdings for
  // targeted refresh – only these symbols will be force-refreshed from the API.
  const handleRefreshQuotes = useCallback(() => {
    const seen = new Set<string>();
    const symbols: [string, string][] = [];
    for (const h of displayData) {
      if (!seen.has(h.symbol)) {
        seen.add(h.symbol);
        symbols.push([h.symbol, h.market]);
      }
    }
    fetchHoldingQuotes(symbols);
  }, [displayData, fetchHoldingQuotes]);

  const staticColumns = [
    {
      title: "股票代码",
      dataIndex: "symbol",
      key: "symbol",
      render: (symbol: string, record: HoldingWithQuote) => (
        <Space>
          <Tag color={isCashSymbol(symbol) ? "gold" : marketColors[record.market as Market]}>
            {isCashSymbol(symbol) ? "💵" : record.market}
          </Tag>
          <strong>{symbol}</strong>
        </Space>
      ),
    },
    { title: "股票名称", dataIndex: "name", key: "name" },
    {
      title: "所属账户",
      dataIndex: "account_id",
      key: "account_id",
      render: (id: string) => accountMap[id] || id,
    },
    {
      title: "投资类别",
      dataIndex: "category_id",
      key: "category_id",
      render: (id: string | null) => {
        if (!id) return "—";
        const cat = categoryMap[id];
        return cat ? (
          <Tag color={cat.color}>
            {cat.icon} {cat.name}
          </Tag>
        ) : "—";
      },
    },
    {
      title: "持仓数量/金额",
      dataIndex: "shares",
      key: "shares",
      render: (v: number, record: HoldingWithQuote) =>
        isCashSymbol(record.symbol)
          ? `${record.currency} ${v.toLocaleString(undefined, CURRENCY_FORMAT_OPTIONS)}`
          : v.toLocaleString(),
    },
    {
      title: "平均成本",
      dataIndex: "avg_cost",
      key: "avg_cost",
      render: (v: number, record: HoldingWithQuote) =>
        isCashSymbol(record.symbol) ? "—" : `${record.currency} ${v.toFixed(3)}`,
    },
  ];

  const realtimeColumns = [
    {
      title: "实时价格",
      key: "current_price",
      render: (_: unknown, record: HoldingWithQuote) => {
        if (!record.quote) return quotesLoading ? <Spin size="small" /> : <span>—</span>;
        return (
          <span>
            {record.currency} {record.quote.current_price.toFixed(2)}
          </span>
        );
      },
    },
    {
      title: "涨跌幅",
      key: "change_percent",
      render: (_: unknown, record: HoldingWithQuote) => {
        if (!record.quote) return <span>—</span>;
        const { change, change_percent } = record.quote;
        const isPositive = change >= 0;
        const color = isPositive ? "#3f8600" : "#cf1322";
        const sign = isPositive ? "+" : "";
        return (
          <span style={{ color }}>
            {sign}{change.toFixed(2)} ({sign}{change_percent.toFixed(2)}%)
          </span>
        );
      },
    },
    {
      title: "当前市值",
      key: "market_value",
      sorter: (a: HoldingWithQuote, b: HoldingWithQuote) =>
        (a.market_value ?? 0) - (b.market_value ?? 0),
      defaultSortOrder: "descend" as const,
      render: (_: unknown, record: HoldingWithQuote) => {
        if (record.market_value === null || record.market_value === undefined)
          return <span>—</span>;
        return `${record.currency} ${record.market_value.toFixed(2)}`;
      },
    },
    {
      title: "盈亏",
      key: "unrealized_pnl",
      render: (_: unknown, record: HoldingWithQuote) => (
        <PnlText value={record.unrealized_pnl ?? null} percent={record.unrealized_pnl_percent ?? null} />
      ),
    },
  ];

  const actionColumn = {
    title: "操作",
    key: "action",
    render: (_: unknown, record: HoldingWithQuote) => (
      <Space>
        <Button type="link" size="small" onClick={() => handleEdit(record)}>
          编辑
        </Button>
        <Popconfirm
          title="确认删除该持仓？"
          onConfirm={() => handleDelete(record.id)}
          okText="确认"
          cancelText="取消"
        >
          <Button type="link" size="small" danger>
            删除
          </Button>
        </Popconfirm>
      </Space>
    ),
  };

  const columns = showRealtime
    ? [...staticColumns, ...realtimeColumns, actionColumn]
    : [...staticColumns, actionColumn];

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <Title level={2} className="!mb-0">
          持仓管理
        </Title>
        <Space>
          <Space>
            <Text type="secondary">实时行情</Text>
            <Switch
              checked={showRealtime}
              onChange={setShowRealtime}
              size="small"
            />
          </Space>
          {showRealtime && (
            <Tooltip title={lastUpdatedAt ? `上次更新: ${dayjs(lastUpdatedAt).format("HH:mm:ss")}` : "点击刷新"}>
              <Button
                icon={quotesLoading ? <SyncOutlined spin /> : <ReloadOutlined />}
                onClick={handleRefreshQuotes}
                size="small"
                disabled={quotesLoading}
              >
                刷新行情
              </Button>
            </Tooltip>
          )}
          <Button
            type="primary"
            icon={<PlusOutlined />}
            onClick={() => {
              setEditingHolding(null);
              form.resetFields();
              // Pre-populate form fields based on active filters
              if (filterAccountId) {
                form.setFieldsValue({ accountId: filterAccountId });
                handleAccountChange(filterAccountId);
              }
              if (filterMarket) {
                form.setFieldsValue({
                  market: filterMarket,
                  currency: marketToCurrency[filterMarket],
                });
              }
              setModalOpen(true);
            }}
          >
            新增持仓
          </Button>
          <Button
            icon={<DollarOutlined />}
            onClick={() => {
              cashForm.resetFields();
              if (filterAccountId) {
                cashForm.setFieldsValue({ accountId: filterAccountId });
              }
              setCashModalOpen(true);
            }}
          >
            添加现金
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
          <Space>
            <Text type="secondary">按市场:</Text>
            <Select
              value={filterMarket}
              onChange={setFilterMarket}
              placeholder="全部市场"
              allowClear
              style={{ width: 140 }}
            >
              <Select.Option value="US">🇺🇸 美股</Select.Option>
              <Select.Option value="CN">🇨🇳 A股</Select.Option>
              <Select.Option value="HK">🇭🇰 港股</Select.Option>
            </Select>
          </Space>
        </Space>
      </div>

      <Table
        dataSource={displayData}
        columns={columns}
        rowKey="id"
        loading={holdingsLoading}
        pagination={{ pageSize: 20 }}
        scroll={{ x: showRealtime ? 1200 : undefined }}
      />

      <Modal
        title={editingHolding ? "编辑持仓" : "新增持仓"}
        open={modalOpen}
        onOk={() => form.submit()}
        onCancel={() => {
          setModalOpen(false);
          setEditingHolding(null);
          setSymbolSearch("");
          form.resetFields();
        }}
        okText="确认"
        cancelText="取消"
        width={600}
      >
        <Form form={form} layout="vertical" onFinish={handleSubmit}>
          <Form.Item
            name="accountId"
            label="所属账户"
            rules={[{ required: true, message: "请选择账户" }]}
          >
            <Select placeholder="选择证券账户" onChange={handleAccountChange}>
              {accounts.map((a) => (
                <Select.Option key={a.id} value={a.id}>
                  [{a.market}] {a.name}
                </Select.Option>
              ))}
            </Select>
          </Form.Item>
          <Form.Item
            name="symbol"
            label="股票代码"
            rules={[{ required: true, message: "请输入股票代码" }]}
          >
            <AutoComplete
              options={filteredSymbolOptions}
              onSearch={setSymbolSearch}
              onSelect={handleSymbolSelect}
              onBlur={handleSymbolBlur}
              placeholder="如：AAPL, sh600519, 0700.HK"
            />
          </Form.Item>
          <Form.Item
            name="name"
            label="股票名称"
            rules={[{ required: true, message: "请输入股票名称" }]}
          >
            <Input
              placeholder="如：苹果, 贵州茅台, 腾讯控股"
              suffix={fetchingName ? <Spin size="small" /> : undefined}
            />
          </Form.Item>
          <Form.Item
            name="market"
            label="市场"
            rules={[{ required: true, message: "请选择市场" }]}
          >
            <Select placeholder="选择市场">
              <Select.Option value="US">🇺🇸 美股</Select.Option>
              <Select.Option value="CN">🇨🇳 A股</Select.Option>
              <Select.Option value="HK">🇭🇰 港股</Select.Option>
            </Select>
          </Form.Item>
          <Form.Item name="categoryId" label="投资类别">
            <Select placeholder="选择投资类别（可选）" allowClear>
              {categories.map((c) => (
                <Select.Option key={c.id} value={c.id}>
                  {c.icon} {c.name}
                </Select.Option>
              ))}
            </Select>
          </Form.Item>
          <Form.Item
            name="shares"
            label="持仓股数"
            rules={[{ required: true, message: "请输入持仓股数" }]}
          >
            <InputNumber min={0} precision={0} style={{ width: "100%" }} placeholder="持有股数" />
          </Form.Item>
          <Form.Item
            name="avgCost"
            label="平均成本价"
            rules={[{ required: true, message: "请输入平均成本价" }]}
          >
            <InputNumber precision={4} style={{ width: "100%" }} placeholder="买入均价" />
          </Form.Item>
          <Form.Item
            name="currency"
            label="币种"
            rules={[{ required: true, message: "请选择币种" }]}
          >
            <Select placeholder="选择币种">
              <Select.Option value="USD">USD 美元</Select.Option>
              <Select.Option value="CNY">CNY 人民币</Select.Option>
              <Select.Option value="HKD">HKD 港元</Select.Option>
            </Select>
          </Form.Item>
        </Form>
      </Modal>

      {/* Add Cash Modal */}
      <Modal
        title="添加现金"
        open={cashModalOpen}
        onOk={() => cashForm.submit()}
        onCancel={() => {
          setCashModalOpen(false);
          cashForm.resetFields();
        }}
        okText="确认"
        cancelText="取消"
        width={480}
      >
        <Form form={cashForm} layout="vertical" onFinish={handleCashSubmit}>
          <Form.Item
            name="accountId"
            label="所属账户"
            rules={[{ required: true, message: "请选择账户" }]}
          >
            <Select placeholder="选择证券账户">
              {accounts.map((a) => (
                <Select.Option key={a.id} value={a.id}>
                  [{a.market}] {a.name}
                </Select.Option>
              ))}
            </Select>
          </Form.Item>
          <Form.Item
            name="amount"
            label="现金金额"
            rules={[{ required: true, message: "请输入现金金额" }]}
          >
            <InputNumber min={0} precision={2} style={{ width: "100%" }} placeholder="如：10000" />
          </Form.Item>
        </Form>
      </Modal>
    </div>
  );
}
