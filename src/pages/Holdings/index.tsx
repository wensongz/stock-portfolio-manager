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
  Row,
  Col,
} from "antd";
import { PlusOutlined, ReloadOutlined, SyncOutlined, FilterOutlined, DollarOutlined, UploadOutlined } from "@ant-design/icons";
import ImportHoldingFromCsvModal from "./ImportHoldingFromCsvModal";
import ImportHoldingFromIbCsvModal from "./ImportHoldingFromIbCsvModal";
import ImportHoldingFromMoomooCsvModal from "./ImportHoldingFromMoomooCsvModal";
import ImportHoldingFromFirstradeCsvModal from "./ImportHoldingFromFirstradeCsvModal";
import { invoke } from "@tauri-apps/api/core";
import { useHoldingStore } from "../../stores/holdingStore";
import { useAccountStore } from "../../stores/accountStore";
import { useCategoryStore } from "../../stores/categoryStore";
import { useQuoteStore } from "../../stores/quoteStore";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import { usePnlColor } from "../../hooks/usePnlColor";
import type { Holding, HoldingWithQuote, Market, Currency, StockQuote, Transaction, TransactionType } from "../../types";
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

/** Returns true if a holding is a fully-cleared (fully-sold) stock position. */
function isClearedPosition(holding: { symbol: string; shares: number }): boolean {
  return !isCashSymbol(holding.symbol) && holding.shares === 0;
}

/**
 * Compute the cash flow delta caused by a single transaction.
 * Cash-symbol BUY/OPEN → deposit, positive.
 * Stock BUY → cash out (negative). Commission is added to the outflow because
 *   it further reduces available cash on top of the purchase cost.
 * Stock SELL → cash in (positive). Commission is subtracted from the inflow
 *   because it is deducted from the proceeds, reducing available cash.
 * PAY (dividend) → cash in (positive).
 * Stock OPEN → 0 (initial position entry, no real cash movement).
 */
function computeCashDelta(txn: Transaction): number {
  if (isCashSymbol(txn.symbol)) {
    // Cash deposit (BUY or OPEN for the cash symbol itself)
    return txn.total_amount;
  }
  switch (txn.transaction_type) {
    case "BUY": return -(txn.total_amount + txn.commission);
    case "SELL": return txn.total_amount - txn.commission;
    case "PAY": return txn.total_amount;
    default: return 0; // OPEN for stocks
  }
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
  const { pnlColorDark } = usePnlColor();
  if (value === null || value === undefined) return <span>—</span>;
  const isPositive = value >= 0;
  const color = pnlColorDark(value);
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
  const { holdingQuotes, loading: quotesLoading, lastUpdatedAt, fetchHoldingQuotes } = useQuoteStore();
  const { pnlColorDark: pnlColorDarkFn } = usePnlColor();
  const { convertWithCachedRates, fetchRates } = useExchangeRateStore();

  const [modalOpen, setModalOpen] = useState(false);
  const [cashModalOpen, setCashModalOpen] = useState(false);
  const [holdingCsvImportModalOpen, setHoldingCsvImportModalOpen] = useState(false);
  const [holdingIbCsvImportModalOpen, setHoldingIbCsvImportModalOpen] = useState(false);
  const [holdingMoomooCsvImportModalOpen, setHoldingMoomooCsvImportModalOpen] = useState(false);
  const [holdingFirstradeCsvImportModalOpen, setHoldingFirstradeCsvImportModalOpen] = useState(false);
  const [editingHolding, setEditingHolding] = useState<Holding | null>(null);
  const [detailModalOpen, setDetailModalOpen] = useState(false);
  const [detailHolding, setDetailHolding] = useState<HoldingWithQuote | null>(null);
  const [detailTransactions, setDetailTransactions] = useState<Transaction[]>([]);
  const [detailLoading, setDetailLoading] = useState(false);
  const [showRealtime, setShowRealtime] = useState(true);
  const [showCleared, setShowCleared] = useState(false);
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

  const currencySymbol: Record<string, string> = { USD: "$", CNY: "¥", HKD: "HK$" };

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

  useEffect(() => {
    fetchRates();
  }, [fetchRates]);

  // Load holdings with cached quotes when realtime display is enabled.
  // No periodic auto-refresh – the backend refreshes the cache on startup
  // and the user can click the refresh button for on-demand updates.
  useEffect(() => {
    if (!showRealtime) return;
    const { startAutoRefresh } = useQuoteStore.getState();
    return startAutoRefresh();
  }, [showRealtime]);

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
        // Reload holding quotes from DB cache (no API call) so the table
        // immediately reflects the updated holding metadata.
        fetchHoldingQuotes([]);
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

  const handleShowDetail = useCallback(async (holding: HoldingWithQuote) => {
    setDetailHolding(holding);
    setDetailModalOpen(true);
    setDetailLoading(true);
    try {
      let txns: Transaction[];
      if (isCashSymbol(holding.symbol)) {
        // For cash holdings, fetch all account transactions so we can show the
        // full cash flow history (deposits + stock buys/sells/dividends).
        txns = await invoke<Transaction[]>("get_transactions", {
          accountId: holding.account_id,
        });
        // Keep only transactions that match the cash currency, and skip OPEN
        // records for non-cash symbols (those have zero cash impact).
        txns = txns.filter(
          (t) =>
            t.currency === holding.currency &&
            !(t.transaction_type === "OPEN" && !isCashSymbol(t.symbol)),
        );
      } else {
        txns = await invoke<Transaction[]>("get_transactions", {
          accountId: holding.account_id,
          symbol: holding.symbol,
        });
      }
      setDetailTransactions(txns);
    } catch (err) {
      message.error(`获取交易记录失败: ${err}`);
      setDetailTransactions([]);
    } finally {
      setDetailLoading(false);
    }
  }, []);

  const accountMap = Object.fromEntries(accounts.map((a) => [a.id, a.name]));
  const categoryMap = Object.fromEntries(categories.map((c) => [c.id, c]));

  // Compute cash flow rows for the cash holding detail modal.
  // Transactions are sorted chronologically (ascending) to compute the running
  // balance, then reversed so the table shows the most recent entry first.
  const cashFlowRows = useMemo(() => {
    if (!detailHolding || !isCashSymbol(detailHolding.symbol)) return [];
    const sorted = [...detailTransactions].sort(
      (a, b) => new Date(a.traded_at).getTime() - new Date(b.traded_at).getTime(),
    );
    let balance = 0;
    const rows = sorted.map((txn) => {
      const delta = computeCashDelta(txn);
      balance += delta;
      return { ...txn, cashDelta: delta, runningBalance: balance };
    });
    return rows.reverse();
  }, [detailHolding, detailTransactions]);

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
    if (showCleared) {
      // Show only cleared (fully-sold) non-cash positions
      return isClearedPosition(h);
    } else {
      // Show active positions (shares > 0); cash is always active
      return isCashSymbol(h.symbol) || h.shares > 0;
    }
  });
  displayDataRef.current = displayData;

  // Determine if the "从CSV导入" button should be shown:
  // visible only when a single CN account is selected and it has no active holdings.
  const selectedAccount = filterAccountId
    ? accounts.find((a) => a.id === filterAccountId)
    : undefined;
  const isCnAccountSelected = selectedAccount?.market === "CN";
  const isUsOrHkAccountSelected =
    selectedAccount?.market === "US" || selectedAccount?.market === "HK";
  const isMoomooAccount = !!selectedAccount?.name.toLowerCase().includes("moomoo");
  const isFirstradeAccount = !!selectedAccount?.name.toLowerCase().includes("firstrade");
  const activeHoldingsForSelectedAccount =
    (isCnAccountSelected || isUsOrHkAccountSelected) && filterAccountId
      ? allDisplayData.filter(
          (h) => h.account_id === filterAccountId && (isCashSymbol(h.symbol) || h.shares > 0),
        )
      : [];
  const showCsvImportButton =
    isCnAccountSelected && !holdingsLoading && activeHoldingsForSelectedAccount.length === 0;
  const showMoomooCsvImportButton =
    isUsOrHkAccountSelected && isMoomooAccount && !holdingsLoading && activeHoldingsForSelectedAccount.length === 0;
  const showFirstradeCsvImportButton =
    isUsOrHkAccountSelected && isFirstradeAccount && !holdingsLoading && activeHoldingsForSelectedAccount.length === 0;
  const showIbCsvImportButton =
    isUsOrHkAccountSelected && !isMoomooAccount && !isFirstradeAccount && !holdingsLoading && activeHoldingsForSelectedAccount.length === 0;

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
      width: 140,
      sorter: (a: HoldingWithQuote, b: HoldingWithQuote) => a.symbol.localeCompare(b.symbol),
      render: (symbol: string, record: HoldingWithQuote) => (
        <Space>
          <Tag color={isCashSymbol(symbol) ? "gold" : marketColors[record.market as Market]}>
            {isCashSymbol(symbol) ? "💵" : record.market}
          </Tag>
          <strong>{symbol}</strong>
        </Space>
      ),
      fixed: "left" as const,
    },
    { title: "股票名称", dataIndex: "name", key: "name", width: 120, ellipsis: true, },
    ...(!filterAccountId ? [{
      title: "所属账户",
      dataIndex: "account_id",
      key: "account_id",
      width: 105,
      render: (id: string) => accountMap[id] || id,
    }] : []),
    {
      title: "投资类别",
      dataIndex: "category_id",
      key: "category_id",
      width: 105,
      sorter: (a: HoldingWithQuote, b: HoldingWithQuote) => {
        const nameA = (a.category_id && categoryMap[a.category_id]?.name) || "";
        const nameB = (b.category_id && categoryMap[b.category_id]?.name) || "";
        return nameA.localeCompare(nameB);
      },
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
      title: "持仓数量 / 金额",
      dataIndex: "shares",
      key: "shares",
      width: 100,
      ellipsis: true,
      render: (v: number, record: HoldingWithQuote) =>
        isCashSymbol(record.symbol)
          ? `${currencySymbol[record.currency]}${v.toLocaleString(undefined, CURRENCY_FORMAT_OPTIONS)}`
          : v.toLocaleString(),
    },
    {
      title: "平均成本",
      dataIndex: "avg_cost",
      key: "avg_cost",
      width: 120,
      ellipsis: true,
      render: (v: number, record: HoldingWithQuote) =>
        isCashSymbol(record.symbol) ? "—" : `${currencySymbol[record.currency]}${v.toFixed(3)}`,
    },
  ];

  const realtimeColumns = [
    {
      title: "实时价格",
      key: "current_price",
      width: 110,
      render: (_: unknown, record: HoldingWithQuote) => {
        if (!record.quote) return quotesLoading ? <Spin size="small" /> : <span>—</span>;
        return (
          <span>
            {currencySymbol[record.currency]}{record.quote.current_price.toFixed(2)}
          </span>
        );
      },
    },
    {
      title: "涨跌幅",
      key: "change_percent",
      width: 140,
      render: (_: unknown, record: HoldingWithQuote) => {
        if (!record.quote) return <span>—</span>;
        const { change, change_percent } = record.quote;
        const isPositive = change >= 0;
        const color = pnlColorDarkFn(change);
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
      width: 145,
      sorter: (a: HoldingWithQuote, b: HoldingWithQuote) => {
        const aUsd = convertWithCachedRates(a.market_value ?? 0, a.currency, "USD");
        const bUsd = convertWithCachedRates(b.market_value ?? 0, b.currency, "USD");
        return aUsd - bUsd;
      },
      defaultSortOrder: "descend" as const,
      render: (_: unknown, record: HoldingWithQuote) => {
        if (record.market_value === null || record.market_value === undefined)
          return <span>—</span>;
        return `${currencySymbol[record.currency]}${record.market_value.toFixed(2)}`;
      },
    },
    {
      title: "盈亏",
      key: "unrealized_pnl",
      width: 200,
      sorter: (a: HoldingWithQuote, b: HoldingWithQuote) => {
        const aUsd = convertWithCachedRates(a.unrealized_pnl ?? 0, a.currency, "USD");
        const bUsd = convertWithCachedRates(b.unrealized_pnl ?? 0, b.currency, "USD");
        return aUsd - bUsd;
      },
      render: (_: unknown, record: HoldingWithQuote) => {
        const isCleared = isClearedPosition(record);
        return (
          <span>
            <PnlText value={record.unrealized_pnl ?? null} percent={record.unrealized_pnl_percent ?? null} />
            {isCleared && (
              <Tag color="default" style={{ marginLeft: 4, fontSize: 11 }}>已实现</Tag>
            )}
          </span>
        );
      },
    },
  ];

  const actionColumn = {
    title: "操作",
    key: "action",
    width: 160,
    render: (_: unknown, record: HoldingWithQuote) => (
      <Space>
        <Button type="link" size="small" onClick={() => handleShowDetail(record)}>
          明细
        </Button>
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
          {showCsvImportButton && (
            <Button
              icon={<UploadOutlined />}
              onClick={() => setHoldingCsvImportModalOpen(true)}
            >
              从CSV导入
            </Button>
          )}
          {showMoomooCsvImportButton && (
            <Button
              icon={<UploadOutlined />}
              onClick={() => setHoldingMoomooCsvImportModalOpen(true)}
            >
              从CSV导入
            </Button>
          )}
          {showFirstradeCsvImportButton && (
            <Button
              icon={<UploadOutlined />}
              onClick={() => setHoldingFirstradeCsvImportModalOpen(true)}
            >
              从CSV导入
            </Button>
          )}
          {showIbCsvImportButton && (
            <Button
              icon={<UploadOutlined />}
              onClick={() => setHoldingIbCsvImportModalOpen(true)}
            >
              从CSV导入
            </Button>
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

      {(() => {
        const PAGE_SIZE = 20;
        const clearedCount = allDisplayData.filter(
          (h) => isClearedPosition(h) &&
            (!filterAccountId || h.account_id === filterAccountId) &&
            (!filterMarket || h.market === filterMarket)
        ).length;
        return (
          <>
            <Table
              dataSource={displayData}
              columns={columns}
              rowKey="id"
              loading={holdingsLoading}
              pagination={displayData.length > PAGE_SIZE ? { pageSize: PAGE_SIZE } : false}
              scroll={{ x: showRealtime ? 1200 : undefined }}
            />

            {/* Cleared positions toggle button */}
            {clearedCount > 0 && (
              <div className="mt-2">
                <Button
                  size="small"
                  type={showCleared ? "primary" : "default"}
                  onClick={() => setShowCleared(!showCleared)}
                >
                  查看已清仓股票（{clearedCount}）
                </Button>
              </div>
            )}
          </>
        );
      })()}

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
            style={{ marginBottom: 12 }}
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
          <Row gutter={12}>
            <Col span={12}>
              <Form.Item
                name="symbol"
                label="股票代码"
                style={{ marginBottom: 12 }}
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
            </Col>
            <Col span={12}>
              <Form.Item
                name="name"
                label="股票名称"
                style={{ marginBottom: 12 }}
                rules={[{ required: true, message: "请输入股票名称" }]}
              >
                <Input
                  placeholder="如：苹果, 贵州茅台, 腾讯控股"
                  suffix={fetchingName ? <Spin size="small" /> : undefined}
                />
              </Form.Item>
            </Col>
          </Row>
          <Form.Item name="categoryId" label="投资类别" style={{ marginBottom: 12 }}>
            <Select placeholder="选择投资类别（可选）" allowClear>
              {categories.map((c) => (
                <Select.Option key={c.id} value={c.id}>
                  {c.icon} {c.name}
                </Select.Option>
              ))}
            </Select>
          </Form.Item>
          <Row gutter={12}>
            <Col span={12}>
              <Form.Item
                name="shares"
                label="持仓股数"
                style={{ marginBottom: 12 }}
                rules={[{ required: true, message: "请输入持仓股数" }]}
              >
                <InputNumber min={0} precision={0} style={{ width: "100%" }} placeholder="持有股数" />
              </Form.Item>
            </Col>
            <Col span={12}>
              <Form.Item
                name="avgCost"
                label="平均成本价"
                style={{ marginBottom: 12 }}
                rules={[{ required: true, message: "请输入平均成本价" }]}
              >
                <InputNumber precision={4} style={{ width: "100%" }} placeholder="买入均价" />
              </Form.Item>
            </Col>
          </Row>
          <Row gutter={12}>
            <Col span={12}>
              <Form.Item
                name="market"
                label="市场"
                style={{ marginBottom: 0 }}
                rules={[{ required: true, message: "请选择市场" }]}
              >
                <Select placeholder="选择市场">
                  <Select.Option value="US">🇺🇸 美股</Select.Option>
                  <Select.Option value="CN">🇨🇳 A股</Select.Option>
                  <Select.Option value="HK">🇭🇰 港股</Select.Option>
                </Select>
              </Form.Item>
            </Col>
            <Col span={12}>
              <Form.Item
                name="currency"
                label="币种"
                style={{ marginBottom: 0 }}
                rules={[{ required: true, message: "请选择币种" }]}
              >
                <Select placeholder="选择币种">
                  <Select.Option value="USD">USD 美元</Select.Option>
                  <Select.Option value="CNY">CNY 人民币</Select.Option>
                  <Select.Option value="HKD">HKD 港元</Select.Option>
                </Select>
              </Form.Item>
            </Col>
          </Row>
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

      {/* Transaction Detail Modal */}
      <Modal
        title={detailHolding ? `交易明细 — ${detailHolding.name} (${detailHolding.symbol})` : "交易明细"}
        open={detailModalOpen}
        onCancel={() => {
          setDetailModalOpen(false);
          setDetailHolding(null);
          setDetailTransactions([]);
        }}
        footer={null}
        width={detailHolding && isCashSymbol(detailHolding.symbol) ? 1000 : 900}
      >
        {detailHolding && isCashSymbol(detailHolding.symbol) ? (
          /* Cash flow history: shows all account transactions with cash impact */
          <Table
            dataSource={cashFlowRows}
            rowKey="id"
            loading={detailLoading}
            pagination={false}
            scroll={{ y: 400 }}
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
                render: (type: TransactionType, record: Transaction & { cashDelta: number }) => {
                  if (isCashSymbol(record.symbol)) {
                    return <Tag color="blue">存入</Tag>;
                  }
                  // In the cash flow view, colors reflect the cash impact:
                  // BUY → red (cash decreases), SELL → green (cash increases).
                  // This intentionally differs from the stock detail view where
                  // colors reflect the stock position change (BUY=green, SELL=red).
                  if (type === "BUY") return <Tag color="red">买入</Tag>;
                  if (type === "SELL") return <Tag color="green">卖出</Tag>;
                  if (type === "PAY") return <Tag color="orange">分红</Tag>;
                  return <Tag>{type}</Tag>;
                },
              },
              {
                title: "股票",
                key: "stock",
                render: (_: unknown, record: Transaction) => {
                  if (isCashSymbol(record.symbol)) {
                    return <span className="text-gray-500">—</span>;
                  }
                  return (
                    <Space>
                      <strong>{record.symbol}</strong>
                      <span className="text-gray-500 text-sm">{record.name}</span>
                    </Space>
                  );
                },
              },
              {
                title: "现金变动",
                key: "cashDelta",
                width: 140,
                align: "right" as const,
                render: (_: unknown, record: Transaction & { cashDelta: number }) => {
                  const delta = record.cashDelta;
                  const sym = currencySymbol[record.currency] ?? "";
                  const isPositive = delta >= 0;
                  return (
                    <span style={{ color: isPositive ? "#16a34a" : "#dc2626", fontWeight: 500 }}>
                      {isPositive ? "+" : ""}
                      {sym}{Math.abs(delta).toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
                    </span>
                  );
                },
              },
              {
                title: "余额",
                key: "runningBalance",
                width: 140,
                align: "right" as const,
                render: (_: unknown, record: Transaction & { runningBalance: number }) => {
                  const sym = currencySymbol[record.currency] ?? "";
                  return (
                    <span style={{ fontWeight: 500 }}>
                      {sym}{record.runningBalance.toLocaleString(undefined, { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
                    </span>
                  );
                },
              },
              {
                title: "备注",
                dataIndex: "notes",
                key: "notes",
                render: (v: string | null) => v || "—",
              },
            ]}
          />
        ) : (
          /* Regular stock holding: show standard transaction details */
          <Table
            dataSource={detailTransactions}
            rowKey="id"
            loading={detailLoading}
            pagination={false}
            scroll={{ y: 400 }}
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
                  <Tag color={type === "BUY" ? "green" : type === "OPEN" ? "blue" : type === "PAY" ? "orange" : "red"}>
                    {type === "BUY" ? "买入" : type === "OPEN" ? "建仓" : type === "PAY" ? "分红" : "卖出"}
                  </Tag>
                ),
              },
              {
                title: "股数",
                dataIndex: "shares",
                key: "shares",
                width: 100,
                render: (v: number) => v.toLocaleString(),
              },
              {
                title: "价格",
                dataIndex: "price",
                key: "price",
                width: 100,
                render: (v: number, record: Transaction) => `${currencySymbol[record.currency]}${v.toFixed(2)}`,
              },
              {
                title: "总金额",
                dataIndex: "total_amount",
                key: "total_amount",
                render: (v: number, record: Transaction) => `${currencySymbol[record.currency]}${v.toFixed(2)}`,
              },
              {
                title: "手续费",
                dataIndex: "commission",
                key: "commission",
                width: 100,
                render: (v: number, record: Transaction) => `${currencySymbol[record.currency]}${v.toFixed(2)}`,
              },
              {
                title: "备注",
                dataIndex: "notes",
                key: "notes",
                render: (v: string | null) => v || "—",
              },
            ]}
          />
        )}
      </Modal>

      {/* Import Holdings from CSV Modal (CN) */}
      {selectedAccount && (
        <ImportHoldingFromCsvModal
          open={holdingCsvImportModalOpen}
          account={selectedAccount}
          onClose={() => setHoldingCsvImportModalOpen(false)}
          onImported={() => {
            setHoldingCsvImportModalOpen(false);
            fetchHoldings();
          }}
        />
      )}

      {/* Import Holdings from IB CSV Modal (US / HK) */}
      {selectedAccount && (
        <ImportHoldingFromIbCsvModal
          open={holdingIbCsvImportModalOpen}
          account={selectedAccount}
          onClose={() => setHoldingIbCsvImportModalOpen(false)}
          onImported={() => {
            setHoldingIbCsvImportModalOpen(false);
            fetchHoldings();
          }}
        />
      )}

      {/* Import Holdings from Moomoo CSV Modal (US / HK) */}
      {selectedAccount && (
        <ImportHoldingFromMoomooCsvModal
          open={holdingMoomooCsvImportModalOpen}
          account={selectedAccount}
          onClose={() => setHoldingMoomooCsvImportModalOpen(false)}
          onImported={() => {
            setHoldingMoomooCsvImportModalOpen(false);
            fetchHoldings();
          }}
        />
      )}

      {/* Import Holdings from Firstrade CSV Modal (US) */}
      {selectedAccount && (
        <ImportHoldingFromFirstradeCsvModal
          open={holdingFirstradeCsvImportModalOpen}
          account={selectedAccount}
          onClose={() => setHoldingFirstradeCsvImportModalOpen(false)}
          onImported={() => {
            setHoldingFirstradeCsvImportModalOpen(false);
            fetchHoldings();
          }}
        />
      )}
    </div>
  );
}
