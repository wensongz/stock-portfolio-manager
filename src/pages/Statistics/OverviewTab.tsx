import { useMemo, useState, useCallback } from "react";
import { Alert, Row, Col, Card, Spin, Empty, Table, Tag, Typography, Button } from "antd";
import type { ColumnsType } from "antd/es/table";
import PieChart from "../../components/charts/PieChart";
import BarChart from "../../components/charts/BarChart";
import StatCard from "../../components/charts/StatCard";
import type { Currency } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";
import { useTablePageSize } from "../../hooks/tablePageSize";
import { statisticsViewKey, useStatisticsStore } from "../../stores/statisticsStore";
import AccountStockTransactionsModal from "./AccountStockTransactionsModal";

const { Text } = Typography;

const currencySymbol: Record<string, string> = {
  USD: "$",
  CNY: "¥",
  HKD: "HK$",
};

interface AggregatedStock {
  symbol: string;
  name: string;
  market: string;
  category_name: string;
  category_color: string;
  shares: number;
  avg_cost: number;
  current_price: number;
  currency: string;
  market_value: number;
  market_value_usd: number;
  pnl: number;
  pnl_percent: number | null;
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

interface Props {
  baseCurrency: Currency;
}

export default function OverviewTab({ baseCurrency }: Props) {
  const { pageSize, onShowSizeChange } = useTablePageSize();
  const { pnlColor } = usePnlColor();
  const currency = currencySymbol[baseCurrency] ?? "$";
  const { overviewByCurrency, loadingByView, errorByView } =
    useStatisticsStore();
  const overview = overviewByCurrency[baseCurrency] ?? null;
  const viewKey = statisticsViewKey({ kind: "overview", baseCurrency });
  const loading = loadingByView[viewKey] ?? false;
  const error = errorByView[viewKey] ?? null;

  // Aggregate holdings by symbol across all accounts/markets, matching the
  // MarketTab table structure, and keep the per-account breakdown for each
  // symbol so the row can be expanded into an account-level sub-table.
  const aggregatedStocks = useMemo((): AggregatedStock[] => {
    const map = new Map<string, {
      symbol: string;
      name: string;
      market: string;
      category_name: string;
      category_color: string;
      shares: number;
      cost_value: number;
      market_value: number;
      market_value_usd: number;
      pnl: number;
      current_price: number;
      currency: string;
      byAccount: Map<string, {
        account_name: string;
        shares: number;
        cost_value: number;
        market_value: number;
        market_value_usd: number;
        pnl: number;
        currency: string;
      }>;
    }>();
    for (const holding of overview?.holdings ?? []) {
      if (holding.symbol.startsWith("$CASH-")) continue;
      // Skip cleared positions (shares == 0): they have no market value and
      // belong only in the holdings page's "已清仓股票" view, not in the
      // per-stock detail table (consistent with market statistics, which
      // filter WHERE h.shares > 0).
      if (holding.shares <= 0) continue;
      const key = holding.symbol;
      const existing = map.get(key);
      const mvNative = holding.market_value;
      const mvUsd = holding.market_value_usd;
      const costNative = holding.cost_value;
      if (existing) {
        existing.shares += holding.shares;
        existing.cost_value += costNative;
        existing.market_value += mvNative;
        existing.market_value_usd += mvUsd;
        existing.pnl += holding.pnl;
        existing.current_price = holding.current_price;
        // Accumulate the per-account entry.
        const acct = existing.byAccount.get(holding.account_id);
        if (acct) {
          acct.shares += holding.shares;
          acct.cost_value += costNative;
          acct.market_value += mvNative;
          acct.market_value_usd += mvUsd;
          acct.pnl += holding.pnl;
        } else {
          existing.byAccount.set(holding.account_id, {
            account_name: holding.account_name,
            shares: holding.shares,
            cost_value: costNative,
            market_value: mvNative,
            market_value_usd: mvUsd,
            pnl: holding.pnl,
            currency: holding.currency,
          });
        }
      } else {
        const byAccount = new Map<string, {
          account_name: string;
          shares: number;
          cost_value: number;
          market_value: number;
          market_value_usd: number;
          pnl: number;
          currency: string;
        }>();
        byAccount.set(holding.account_id, {
          account_name: holding.account_name,
          shares: holding.shares,
          cost_value: costNative,
          market_value: mvNative,
          market_value_usd: mvUsd,
          pnl: holding.pnl,
          currency: holding.currency,
        });
        map.set(key, {
          symbol: holding.symbol,
          name: holding.name,
          market: holding.market,
          category_name: holding.category_name,
          category_color: holding.category_color,
          shares: holding.shares,
          cost_value: costNative,
          market_value: mvNative,
          market_value_usd: mvUsd,
          pnl: holding.pnl,
          current_price: holding.current_price,
          currency: holding.currency,
          byAccount,
        });
      }
    }

    // Total market value (USD) across all aggregated stocks, used
    // for the per-account position percentage in the sub-table.
    const totalUsd = Array.from(map.values()).reduce(
      (sum, value) => sum + value.market_value_usd,
      0,
    );

    return Array.from(map.values())
      .map((v) => {
        // Build per-account rows, sorted by market value descending.
        const accountRows: AccountHoldingRow[] = Array.from(v.byAccount.entries())
          .map(([accountId, a]) => {
            const acctPnlPercent =
              a.cost_value > 0 ? (a.pnl / a.cost_value) * 100 : null;
            return {
              key: accountId,
              accountName: a.account_name,
              account_id: accountId,
              symbol: v.symbol,
              shares: a.shares,
              avg_cost: a.shares > 0 ? a.cost_value / a.shares : 0,
              market_value: a.market_value,
              market_value_usd: a.market_value_usd,
              position_pct: totalUsd > 0 ? (a.market_value_usd / totalUsd) * 100 : 0,
              pnl: a.pnl,
              pnl_percent: acctPnlPercent,
              currency: a.currency,
            };
          })
          .sort((a, b) => b.market_value_usd - a.market_value_usd);

        return {
          symbol: v.symbol,
          name: v.name,
          market: v.market,
          category_name: v.category_name,
          category_color: v.category_color,
          shares: v.shares,
          avg_cost: v.shares > 0 ? v.cost_value / v.shares : 0,
          current_price: v.current_price,
          currency: v.currency,
          market_value: v.market_value,
          market_value_usd: v.market_value_usd,
          pnl: v.pnl,
          pnl_percent: v.cost_value > 0 ? (v.pnl / v.cost_value) * 100 : null,
          accountRows,
        };
      })
      .sort((a, b) => b.market_value_usd - a.market_value_usd);
  }, [overview]);

  // Columns matching the MarketTab table.
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
        price.toLocaleString("en-US", { minimumFractionDigits: 3, maximumFractionDigits: 3 }),
      align: "right" as const,
      width: 90,
    },
    {
      title: "现价",
      dataIndex: "current_price",
      key: "current_price",
      sorter: (a, b) => a.current_price - b.current_price,
      render: (price: number, record: AggregatedStock) => {
        const sym = currencySymbol[record.currency] ?? "";
        return `${sym}${price.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
      },
      align: "right" as const,
      width: 90,
    },
    {
      title: "市值",
      dataIndex: "market_value",
      key: "market_value",
      sorter: (a, b) => a.market_value_usd - b.market_value_usd,
      defaultSortOrder: "descend" as const,
      render: (value: number, record: AggregatedStock) => {
        const sym = currencySymbol[record.currency] ?? "";
        return `${sym}${value.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
      },
      align: "right" as const,
      width: 140,
    },
    {
      title: "仓位%",
      key: "position_pct",
      sorter: (a, b) => a.market_value_usd - b.market_value_usd,
      render: (_: unknown, record: AggregatedStock) => {
        const total = aggregatedStocks.reduce((s, r) => s + r.market_value_usd, 0);
        const pct = total > 0 ? (record.market_value_usd / total) * 100 : 0;
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
      render: (pnl: number, record: AggregatedStock) => {
        const sym = currencySymbol[record.currency] ?? "";
        const sign = pnl >= 0 ? "+" : "-";
        return (
          <span style={{ color: pnlColor(pnl) }}>
            {sign}{sym}{Math.abs(pnl).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
          </span>
        );
      },
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
  ], [aggregatedStocks, pnlColor]);

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
        render: (value: number, record: AccountHoldingRow) => {
          const sym = currencySymbol[record.currency] ?? "";
          return `${sym}${value.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
        },
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
        render: (pnl: number, record: AccountHoldingRow) => {
          const sym = currencySymbol[record.currency] ?? "";
          const sign = pnl >= 0 ? "+" : "-";
          return (
            <span style={{ color: pnlColor(pnl) }}>
              {sign}{sym}{Math.abs(pnl).toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}
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

  if (loading && !overview) {
    return (
      <div className="flex justify-center py-16">
        <Spin size="large" />
      </div>
    );
  }
  if (!overview) {
    return error ? (
      <Alert title={error} type="error" showIcon />
    ) : (
      <Empty description="暂无数据" />
    );
  }

  const totalPnlPos = overview.total_pnl >= 0;

  const gainersData = overview.top_gainers.map((g) => ({
    name: g.symbol,
    value: parseFloat(g.pnl.toFixed(2)),
  }));
  const losersData = overview.top_losers.map((g) => ({
    name: g.symbol,
    value: parseFloat(g.pnl.toFixed(2)),
  }));

  return (
    <div>
      {error && <Alert title={error} type="error" showIcon className="mb-4" />}

      {/* Summary stats */}
      <Row gutter={[16, 16]} className="mb-4">
        <Col xs={24} sm={8}>
          <StatCard
            title={`总市值 (${baseCurrency})`}
            value={overview.total_market_value.toFixed(2)}
            prefix={currency}
          />
        </Col>
        <Col xs={24} sm={8}>
          <StatCard
            title={`总成本 (${baseCurrency})`}
            value={overview.total_cost.toFixed(2)}
            prefix={currency}
          />
        </Col>
        <Col xs={24} sm={8}>
          <StatCard
            title={`总盈亏 (${baseCurrency})`}
            value={overview.total_pnl.toFixed(2)}
            valueStyle={{ color: pnlColor(overview.total_pnl) }}
            suffix={`(${totalPnlPos ? "+" : ""}${overview.total_pnl_percent.toFixed(2)}%)`}
          />
        </Col>
      </Row>

      {/* Distribution charts */}
      <Row gutter={[16, 16]}>
        <Col xs={24} md={8}>
          <Card title="市场分布">
            <PieChart data={overview.market_distribution} height={260} currencyCode={baseCurrency} />
          </Card>
        </Col>
        <Col xs={24} md={8}>
          <Card title="类别分布">
            <PieChart data={overview.category_distribution} height={260} currencyCode={baseCurrency} />
          </Card>
        </Col>
        <Col xs={24} md={8}>
          <Card title="账户分布">
            <PieChart data={overview.account_distribution} height={260} currencyCode={baseCurrency} />
          </Card>
        </Col>
      </Row>

      {/* Stock distribution chart */}
      {overview.stock_distribution.length > 0 && (
        <Row gutter={[16, 16]} className="mt-4">
          <Col xs={24}>
            <Card title="个股分布">
              <PieChart data={overview.stock_distribution} height={360} currencyCode={baseCurrency} />
            </Card>
          </Col>
        </Row>
      )}

      {/* PnL charts */}
      {(gainersData.length > 0 || losersData.length > 0) && (
        <Row gutter={[16, 16]} className="mt-4">
          <Col xs={24} md={12}>
            <Card title="盈利 Top 5">
              <BarChart data={gainersData} colorByValue height={220} />
            </Card>
          </Col>
          <Col xs={24} md={12}>
            <Card title="亏损 Top 5">
              <BarChart data={losersData} colorByValue height={220} />
            </Card>
          </Col>
        </Row>
      )}

      {/* 个股明细 */}
      {aggregatedStocks.length > 0 && (
        <Row gutter={[16, 16]} className="mt-4">
          <Col xs={24}>
            <Card title="持仓明细">
              <Table
                columns={stockColumns}
                dataSource={aggregatedStocks}
                rowKey="symbol"
                size="small"
                className="account-detail-table"
                scroll={{ x: 1200 }}
                pagination={{ pageSize, showSizeChanger: true, onShowSizeChange }}
                expandable={{
                  expandedRowRender: (record: AggregatedStock) => (
                    // ml-8 indentation like the quarterly sub-table. The
                    // account-detail-table class (on the parent) lets CSS
                    // strip the expanded cell's padding, and account-sub-table
                    // forces square corners on this table.
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
          </Col>
        </Row>
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
