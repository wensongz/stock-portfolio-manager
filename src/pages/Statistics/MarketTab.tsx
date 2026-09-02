import { useMemo, useState, useCallback } from "react";
import { Alert, Row, Col, Card, Spin, Empty, Select, Table, Tag, Typography, Button } from "antd";
import type { ColumnsType } from "antd/es/table";
import PieChart from "../../components/charts/PieChart";
import StatCard from "../../components/charts/StatCard";
import { statisticsViewKey, useStatisticsStore } from "../../stores/statisticsStore";
import type { MarketStatistics } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";
import { useTablePageSize } from "../../hooks/tablePageSize";
import AccountStockTransactionsModal from "./AccountStockTransactionsModal";

const { Text } = Typography;

interface Props {
  selectedMarket: string;
  onMarketChange: (m: string) => void;
}

const markets = [
  { value: "US", label: "🇺🇸 美股" },
  { value: "CN", label: "🇨🇳 A股" },
  { value: "HK", label: "🇭🇰 港股" },
];

const marketCurrency: Record<string, { code: string; symbol: string }> = {
  US: { code: "USD", symbol: "$" },
  CN: { code: "CNY", symbol: "¥" },
  HK: { code: "HKD", symbol: "HK$" },
};

interface AggregatedStock {
  symbol: string;
  name: string;
  category_name: string;
  category_color: string;
  shares: number;
  avg_cost: number;
  current_price: number;
  market_value: number;
  market_value_usd: number;
  pnl: number;
  pnl_percent: number | null;
  _totalMv: number;
  /** Per-account breakdown for the expandable sub-table. Not named
   *  "children" because antd Table treats that field as nested row data. */
  accountRows?: AccountHoldingRow[];
}

interface AccountHoldingRow {
  key: string;
  accountName: string;
  account_id: string;
  symbol: string;
  shares: number;
  avg_cost: number;
  market_value: number;
  market_value_usd: number;
  position_pct: number;
  pnl: number;
  pnl_percent: number | null;
  currency: string;
}

interface StockAccumulator {
  shares: number;
  cost_value: number;
  market_value: number;
  market_value_usd: number;
  pnl: number;
  current_price: number;
  name: string;
  category_name: string;
  category_color: string;
  byAccount: Map<string, {
    shares: number;
    cost_value: number;
    market_value: number;
    market_value_usd: number;
    pnl: number;
    currency: string;
  }>;
  accountNames: Map<string, string>;
}

export default function MarketTab({ selectedMarket, onMarketChange }: Props) {
  const { pageSize, onShowSizeChange } = useTablePageSize();
  const { pnlColor } = usePnlColor();
  const { marketStats, loadingByView, errorByView } = useStatisticsStore();
  const viewKey = statisticsViewKey({ kind: "market", market: selectedMarket });
  const loading = loadingByView[viewKey] ?? false;
  const error = errorByView[viewKey] ?? null;

  const stats: MarketStatistics | undefined = marketStats[selectedMarket];
  const currencySymbol = marketCurrency[selectedMarket]?.symbol ?? "$";
  const currencyCode = marketCurrency[selectedMarket]?.code ?? "USD";

  const aggregatedStocks = useMemo((): AggregatedStock[] => {
    if (!stats) return [];
    const map = new Map<string, StockAccumulator>();
    for (const h of stats.holdings) {
      const existing = map.get(h.symbol);
      if (existing) {
        existing.shares += h.shares;
        existing.cost_value += h.cost_value;
        existing.market_value += h.market_value;
        existing.market_value_usd += h.market_value_usd;
        existing.pnl += h.pnl;
        // All rows for the same symbol share the same live quote; take the last seen.
        existing.current_price = h.current_price;
        // Accumulate per-account entry.
        const acct = existing.byAccount.get(h.account_id);
        if (acct) {
          acct.shares += h.shares;
          acct.cost_value += h.cost_value;
          acct.market_value += h.market_value;
          acct.market_value_usd += h.market_value_usd;
          acct.pnl += h.pnl;
        } else {
          existing.byAccount.set(h.account_id, {
            shares: h.shares,
            cost_value: h.cost_value,
            market_value: h.market_value,
            market_value_usd: h.market_value_usd,
            pnl: h.pnl,
            currency: h.currency,
          });
          existing.accountNames.set(h.account_id, h.account_name);
        }
      } else {
        const byAccount = new Map<string, {
          shares: number;
          cost_value: number;
          market_value: number;
          market_value_usd: number;
          pnl: number;
          currency: string;
        }>();
        byAccount.set(h.account_id, {
          shares: h.shares,
          cost_value: h.cost_value,
          market_value: h.market_value,
          market_value_usd: h.market_value_usd,
          pnl: h.pnl,
          currency: h.currency,
        });
        const accountNames = new Map<string, string>([[h.account_id, h.account_name]]);
        map.set(h.symbol, {
          shares: h.shares,
          cost_value: h.cost_value,
          market_value: h.market_value,
          market_value_usd: h.market_value_usd,
          pnl: h.pnl,
          current_price: h.current_price,
          name: h.name,
          category_name: h.category_name,
          category_color: h.category_color,
          byAccount,
          accountNames,
        });
      }
    }
    const totalMv = Array.from(map.values()).reduce((s, v) => s + v.market_value_usd, 0);
    return Array.from(map.entries()).map(([symbol, v]) => {
      // Build per-account rows, sorted by USD market value descending.
      const accountRows: AccountHoldingRow[] = Array.from(v.byAccount.entries())
        .map(([accountId, a]) => ({
          key: accountId,
          accountName: v.accountNames.get(accountId) ?? accountId,
          account_id: accountId,
          symbol,
          shares: a.shares,
          avg_cost: a.shares > 0 ? a.cost_value / a.shares : 0,
          market_value: a.market_value,
          market_value_usd: a.market_value_usd,
          position_pct: totalMv > 0 ? (a.market_value_usd / totalMv) * 100 : 0,
          pnl: a.pnl,
          pnl_percent: a.cost_value > 0 ? (a.pnl / a.cost_value) * 100 : null,
          currency: a.currency,
        }))
        .sort((a, b) => b.market_value_usd - a.market_value_usd);

      return {
        symbol,
        name: v.name,
        category_name: v.category_name,
        category_color: v.category_color,
        shares: v.shares,
        avg_cost: v.shares > 0 ? v.cost_value / v.shares : 0,
        current_price: v.current_price,
        market_value: v.market_value,
        market_value_usd: v.market_value_usd,
        pnl: v.pnl,
        pnl_percent: v.cost_value > 0 ? (v.pnl / v.cost_value) * 100 : null,
        _totalMv: totalMv,
        accountRows,
      };
    });
  }, [stats]);

  const stockColumns: ColumnsType<AggregatedStock> = useMemo(() => [
    {
      title: "代码",
      dataIndex: "symbol",
      key: "symbol",
      sorter: (a, b) => a.symbol.localeCompare(b.symbol),
      render: (symbol: string) => <Text strong>{symbol}</Text>,
      fixed: "left" as const,
      width: 100,
    },
    {
      title: "名称",
      dataIndex: "name",
      key: "name",
      ellipsis: true,
      width: 140,
    },
    {
      title: "类别",
      dataIndex: "category_name",
      key: "category_name",
      sorter: (a, b) => a.category_name.localeCompare(b.category_name),
      render: (name: string, record: AggregatedStock) => (
        <Tag color={record.category_color}>{name}</Tag>
      ),
      width: 60,
    },
    {
      title: "持仓数量",
      dataIndex: "shares",
      key: "shares",
      sorter: (a, b) => a.shares - b.shares,
      render: (shares: number) => shares.toLocaleString(),
      align: "right" as const,
      width: 90,
    },
    {
      title: "均价",
      dataIndex: "avg_cost",
      key: "avg_cost",
      sorter: (a, b) => a.avg_cost - b.avg_cost,
      render: (price: number) =>
        `${price.toLocaleString("en-US", { minimumFractionDigits: 3, maximumFractionDigits: 3 })}`,
      align: "right" as const,
      width: 90,
    },
    {
      title: "现价",
      dataIndex: "current_price",
      key: "current_price",
      sorter: (a, b) => a.current_price - b.current_price,
      render: (price: number) =>
        `${currencySymbol}${price.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`,
      align: "right" as const,
      width: 90,
    },
    {
      title: "市值",
      dataIndex: "market_value",
      key: "market_value",
      sorter: (a, b) => a.market_value_usd - b.market_value_usd,
      defaultSortOrder: "descend" as const,
      render: (value: number) =>
        `${currencySymbol}${value.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`,
      align: "right" as const,
      width: 140,
    },
    {
      title: "仓位%",
      key: "position_pct",
      sorter: (a, b) => a.market_value_usd - b.market_value_usd,
      render: (_: unknown, record: AggregatedStock) => {
        const pct = record._totalMv > 0 ? (record.market_value_usd / record._totalMv) * 100 : 0;
        return `${pct.toFixed(2)}%`;
      },
      align: "right" as const,
      width: 70,
    },
    {
      title: "盈亏金额",
      dataIndex: "pnl",
      key: "pnl",
      sorter: (a, b) => a.pnl - b.pnl,
      render: (pnl: number) => (
        <span style={{ color: pnlColor(pnl) }}>
          {pnl >= 0 ? "+" : "-"}
          {currencySymbol}{Math.abs(pnl).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
        </span>
      ),
      align: "right" as const,
      width: 140,
    },
    {
      title: "盈亏比例",
      dataIndex: "pnl_percent",
      key: "pnl_percent",
      render: (pnl: number | null) =>
        pnl != null ? (
          <span style={{ color: pnlColor(pnl) }}>
            {pnl >= 0 ? "+" : ""}
            {pnl.toFixed(2)}%
          </span>
        ) : (
          <span>-</span>
        ),
      align: "right" as const,
      width: 80,
    },
  ], [currencySymbol, pnlColor]);

  // State for the per-account transaction detail modal.
  const [txnModal, setTxnModal] = useState<{
    accountId: string;
    accountName: string;
    symbol: string;
    stockName: string;
  } | null>(null);

  const handleShowTransactions = useCallback((record: AccountHoldingRow) => {
    setTxnModal({
      accountId: record.account_id,
      accountName: record.accountName,
      symbol: record.symbol,
      stockName: record.symbol,
    });
  }, []);

  // Sub-table columns: per-account breakdown of one stock.
  const accountDetailColumns: ColumnsType<AccountHoldingRow> = useMemo(
    () => [
      {
        title: "账户",
        dataIndex: "accountName",
        key: "accountName",
        width: 160,
      },
      {
        title: "持仓数量",
        dataIndex: "shares",
        key: "shares",
        align: "right" as const,
        width: 90,
        render: (shares: number) => shares.toLocaleString(),
      },
      {
        title: "均价",
        dataIndex: "avg_cost",
        key: "avg_cost",
        align: "right" as const,
        width: 90,
        render: (price: number) =>
          price.toLocaleString("en-US", { minimumFractionDigits: 3, maximumFractionDigits: 3 }),
      },
      {
        title: "市值",
        dataIndex: "market_value",
        key: "market_value",
        align: "right" as const,
        width: 140,
        render: (value: number) =>
          `${currencySymbol}${value.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`,
      },
      {
        title: "仓位",
        dataIndex: "position_pct",
        key: "position_pct",
        align: "right" as const,
        width: 70,
        render: (pct: number) => `${pct.toFixed(2)}%`,
      },
      {
        title: "盈亏金额",
        dataIndex: "pnl",
        key: "pnl",
        align: "right" as const,
        width: 140,
        render: (pnl: number) => {
          const sign = pnl >= 0 ? "+" : "-";
          return (
            <span style={{ color: pnlColor(pnl) }}>
              {sign}{currencySymbol}{Math.abs(pnl).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
            </span>
          );
        },
      },
      {
        title: "盈亏比例",
        dataIndex: "pnl_percent",
        key: "pnl_percent",
        align: "right" as const,
        width: 80,
        render: (pnl: number | null) =>
          pnl != null ? (
            <span style={{ color: pnlColor(pnl) }}>
              {pnl >= 0 ? "+" : ""}
              {pnl.toFixed(2)}%
            </span>
          ) : (
            <span>-</span>
          ),
      },
      {
        title: "交易",
        key: "transactions",
        width: 80,
        align: "center" as const,
        render: (_: unknown, record: AccountHoldingRow) => (
          <Button
            type="link"
            size="small"
            onClick={() => handleShowTransactions(record)}
          >
            明细
          </Button>
        ),
      },
    ],
    [handleShowTransactions, pnlColor]
  );

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      <div>
        <Select
          value={selectedMarket}
          onChange={onMarketChange}
          style={{ width: 150 }}
        >
          {markets.map((m) => (
            <Select.Option key={m.value} value={m.value}>
              {m.label}
            </Select.Option>
          ))}
        </Select>
      </div>

      {error && <Alert title={error} type="error" showIcon />}

      {loading && !stats ? (
        <div className="flex justify-center py-16">
          <Spin size="large" />
        </div>
      ) : !stats ? (
        <Empty description="暂无市场统计数据" />
      ) : stats.holdings.length === 0 ? (
        <Empty description="该市场暂无持仓" />
      ) : (
        <>
          <Row gutter={[16, 16]}>
            <Col xs={24} sm={8}>
              <StatCard title={`市场总市值 (${currencyCode})`} value={stats.total_market_value.toFixed(2)} prefix={currencySymbol} />
            </Col>
            <Col xs={24} sm={8}>
              <StatCard title={`市场总成本 (${currencyCode})`} value={stats.total_cost.toFixed(2)} prefix={currencySymbol} />
            </Col>
            <Col xs={24} sm={8}>
              <StatCard
                title={`市场总盈亏 (${currencyCode})`}
                value={stats.total_pnl.toFixed(2)}
                valueStyle={{ color: pnlColor(stats.total_pnl) }}
                suffix={`(${stats.total_pnl >= 0 ? "+" : ""}${stats.total_pnl_percent.toFixed(2)}%)`}
              />
            </Col>
          </Row>

          <Row gutter={[16, 16]}>
            {stats.account_distribution.length > 0 && (
              <Col xs={24} md={8}>
                <Card title="账户分布">
                  <PieChart data={stats.account_distribution} height={260} currencyCode={currencyCode} />
                </Card>
              </Col>
            )}
            {stats.category_distribution.length > 0 && (
              <Col xs={24} md={8}>
                <Card title="类别分布">
                  <PieChart data={stats.category_distribution} height={260} currencyCode={currencyCode} />
                </Card>
              </Col>
            )}
            {stats.stock_distribution.length > 0 && (
              <Col xs={24} md={8}>
                <Card title="个股分布">
                  <PieChart data={stats.stock_distribution} height={260} currencyCode={currencyCode} />
                </Card>
              </Col>
            )}
          </Row>

          <Card title="持仓明细" className="mt-4">
            <Table
              columns={stockColumns}
              dataSource={aggregatedStocks}
              rowKey="symbol"
              loading={loading}
              className="account-detail-table"
              scroll={{ x: 1200 }}
              size="small"
              pagination={{ pageSize, showSizeChanger: true, onShowSizeChange }}
              expandable={{
                expandedRowRender: (record: AggregatedStock) => (
                  // Same styling as the overview tab sub-table: ml-8 indent,
                  // account-sub-table class strips expanded-cell padding and
                  // forces square corners.
                  <Table
                    columns={accountDetailColumns}
                    dataSource={record.accountRows ?? []}
                    rowKey="key"
                    size="small"
                    pagination={false}
                    className="ml-8 account-sub-table"
                  />
                ),
                rowExpandable: (record: AggregatedStock) => (record.accountRows?.length ?? 0) > 0,
              }}
            />
          </Card>
        </>
      )}

      {txnModal && (
        <AccountStockTransactionsModal
          open
          accountId={txnModal.accountId}
          accountName={txnModal.accountName}
          symbol={txnModal.symbol}
          stockName={txnModal.stockName}
          onClose={() => setTxnModal(null)}
        />
      )}
    </div>
  );
}
