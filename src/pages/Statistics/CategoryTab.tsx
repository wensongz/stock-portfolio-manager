import { useCallback, useEffect, useMemo, useState } from "react";
import { Row, Col, Card, Statistic, Spin, Empty, Select, Tag, Table, Typography, Button } from "antd";
import type { ColumnsType } from "antd/es/table";
import PieChart from "../../components/charts/PieChart";
import { useStatisticsStore } from "../../stores/dashboardStore";
import { useCategoryStore } from "../../stores/categoryStore";
import type { CategoryStatistics, HoldingDetail } from "../../types";
import type { Currency } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";
import AccountStockTransactionsModal from "./AccountStockTransactionsModal";
import { filterActiveStockHoldings } from "./categoryHoldings";

const { Text } = Typography;

const currencySymbol: Record<string, string> = {
  USD: "$",
  CNY: "¥",
  HKD: "HK$",
};

interface Props {
  selectedCategoryId: string;
  onCategoryChange: (id: string) => void;
  baseCurrency: Currency;
}

interface AccountHoldingRow extends HoldingDetail {
  key: string;
  position_pct: number;
}

interface AggregatedStock extends HoldingDetail {
  accountRows: AccountHoldingRow[];
}

export default function CategoryTab({ selectedCategoryId, onCategoryChange, baseCurrency }: Props) {
  const { pnlColor } = usePnlColor();
  const { categoryStats, fetchCategoryStats } = useStatisticsStore();
  const { categories, fetchCategories } = useCategoryStore();
  const symbol = currencySymbol[baseCurrency] ?? "$";
  const [txnModal, setTxnModal] = useState<{
    accountId: string;
    accountName: string;
    symbol: string;
    stockName: string;
  } | null>(null);

  useEffect(() => {
    fetchCategories();
  }, [fetchCategories]);

  useEffect(() => {
    if (selectedCategoryId) {
      fetchCategoryStats(selectedCategoryId, baseCurrency);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [selectedCategoryId, baseCurrency]);

  const stats: CategoryStatistics | undefined = categoryStats[selectedCategoryId];

  const aggregatedStocks = useMemo((): AggregatedStock[] => {
    if (!stats) return [];
    const activeHoldings = filterActiveStockHoldings(stats.holdings);
    const totalValueUsd = activeHoldings.reduce((sum, h) => sum + h.market_value_usd, 0);
    const groups = new Map<string, HoldingDetail[]>();
    for (const holding of activeHoldings) {
      const rows = groups.get(holding.symbol) ?? [];
      rows.push(holding);
      groups.set(holding.symbol, rows);
    }

    return Array.from(groups.values())
      .map((rows) => {
        const first = rows[0];
        const shares = rows.reduce((sum, h) => sum + h.shares, 0);
        const costValue = rows.reduce((sum, h) => sum + h.cost_value, 0);
        const marketValue = rows.reduce((sum, h) => sum + h.market_value, 0);
        const marketValueUsd = rows.reduce((sum, h) => sum + h.market_value_usd, 0);
        const pnl = rows.reduce((sum, h) => sum + h.pnl, 0);
        return {
          ...first,
          id: first.symbol,
          shares,
          avg_cost: shares > 0 ? costValue / shares : 0,
          market_value: marketValue,
          market_value_usd: marketValueUsd,
          cost_value: costValue,
          pnl,
          pnl_percent: costValue > 0 ? (pnl / costValue) * 100 : null,
          accountRows: rows.map((row) => ({
            ...row,
            key: row.id,
            position_pct: totalValueUsd > 0 ? (row.market_value_usd / totalValueUsd) * 100 : 0,
          })),
        };
      })
      .sort((a, b) => b.market_value_usd - a.market_value_usd);
  }, [stats]);

  const formatMoney = useCallback((value: number, currency: string) => {
    const prefix = currencySymbol[currency] ?? "";
    return `${prefix}${value.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 })}`;
  }, []);

  const handleShowTransactions = useCallback((record: AccountHoldingRow) => {
    setTxnModal({
      accountId: record.account_id,
      accountName: record.account_name,
      symbol: record.symbol,
      stockName: record.name,
    });
  }, []);

  const accountDetailColumns: ColumnsType<AccountHoldingRow> = useMemo(() => [
    { title: "账户", dataIndex: "account_name", key: "account_name", width: 160 },
    { title: "持仓数量", dataIndex: "shares", key: "shares", align: "right", width: 90, render: (v: number) => v.toLocaleString() },
    { title: "均价", dataIndex: "avg_cost", key: "avg_cost", align: "right", width: 90, render: (v: number) => v.toLocaleString("en-US", { minimumFractionDigits: 3, maximumFractionDigits: 3 }) },
    { title: "市值", dataIndex: "market_value", key: "market_value", align: "right", width: 140, render: (v: number, r) => formatMoney(v, r.currency) },
    { title: "仓位", dataIndex: "position_pct", key: "position_pct", align: "right", width: 70, render: (v: number) => `${v.toFixed(2)}%` },
    { title: "盈亏金额", dataIndex: "pnl", key: "pnl", align: "right", width: 140, render: (v: number, r) => <span style={{ color: pnlColor(v) }}>{v >= 0 ? "+" : "-"}{formatMoney(Math.abs(v), r.currency)}</span> },
    { title: "盈亏比例", dataIndex: "pnl_percent", key: "pnl_percent", align: "right", width: 80, render: (v: number | null) => v == null ? "-" : <span style={{ color: pnlColor(v) }}>{v >= 0 ? "+" : ""}{v.toFixed(2)}%</span> },
    { title: "交易", key: "transactions", align: "center", width: 80, render: (_, record) => <Button type="link" size="small" onClick={() => handleShowTransactions(record)}>明细</Button> },
  ], [formatMoney, handleShowTransactions, pnlColor]);

  const stockColumns: ColumnsType<AggregatedStock> = useMemo(() => [
    { title: "代码", dataIndex: "symbol", key: "symbol", fixed: "left", width: 100, sorter: (a, b) => a.symbol.localeCompare(b.symbol), render: (v: string) => <Text strong>{v}</Text> },
    { title: "名称", dataIndex: "name", key: "name", ellipsis: true, width: 140 },
    { title: "类别", dataIndex: "category_name", key: "category_name", width: 60, render: (v: string, r) => <Tag color={r.category_color}>{v}</Tag> },
    { title: "持仓数量", dataIndex: "shares", key: "shares", align: "right", width: 90, sorter: (a, b) => a.shares - b.shares, render: (v: number) => v.toLocaleString() },
    { title: "均价", dataIndex: "avg_cost", key: "avg_cost", align: "right", width: 90, sorter: (a, b) => a.avg_cost - b.avg_cost, render: (v: number) => v.toLocaleString("en-US", { minimumFractionDigits: 3, maximumFractionDigits: 3 }) },
    { title: "现价", dataIndex: "current_price", key: "current_price", align: "right", width: 90, sorter: (a, b) => a.current_price - b.current_price, render: (v: number, r) => formatMoney(v, r.currency) },
    { title: "市值", dataIndex: "market_value", key: "market_value", align: "right", width: 140, defaultSortOrder: "descend", sorter: (a, b) => a.market_value_usd - b.market_value_usd, render: (v: number, r) => formatMoney(v, r.currency) },
    { title: "仓位%", key: "position_pct", align: "right", width: 70, sorter: (a, b) => a.market_value_usd - b.market_value_usd, render: (_, r) => { const total = aggregatedStocks.reduce((sum, row) => sum + row.market_value_usd, 0); return `${(total > 0 ? r.market_value_usd / total * 100 : 0).toFixed(2)}%`; } },
    { title: "盈亏金额", dataIndex: "pnl", key: "pnl", align: "right", width: 140, sorter: (a, b) => a.pnl - b.pnl, render: (v: number, r) => <span style={{ color: pnlColor(v) }}>{v >= 0 ? "+" : "-"}{formatMoney(Math.abs(v), r.currency)}</span> },
    { title: "盈亏比例", dataIndex: "pnl_percent", key: "pnl_percent", align: "right", width: 80, render: (v: number | null) => v == null ? "-" : <span style={{ color: pnlColor(v) }}>{v >= 0 ? "+" : ""}{v.toFixed(2)}%</span> },
  ], [aggregatedStocks, formatMoney, pnlColor]);

  return (
    <div>
      <div className="mb-4">
        <Select
          value={selectedCategoryId || undefined}
          onChange={onCategoryChange}
          placeholder="选择投资类别"
          style={{ width: 220 }}
        >
          {categories.map((c) => (
            <Select.Option key={c.id} value={c.id}>
              <Tag color={c.color} style={{ marginRight: 4 }}>
                {c.icon}
              </Tag>
              {c.name}
            </Select.Option>
          ))}
        </Select>
      </div>

      {!selectedCategoryId ? (
        <Empty description="请选择投资类别" />
      ) : !stats ? (
        <div className="flex justify-center py-16">
          <Spin size="large" />
        </div>
      ) : stats.holdings.length === 0 ? (
        <Empty description="该类别暂无持仓" />
      ) : (
        <>
          <Row gutter={[16, 16]} className="mb-4">
            <Col xs={24} sm={8}>
              <Card>
                <Statistic title={`类别总市值 (${baseCurrency})`} value={stats.total_market_value.toFixed(2)} prefix={symbol} />
              </Card>
            </Col>
            <Col xs={24} sm={8}>
              <Card>
                <Statistic title={`类别总成本 (${baseCurrency})`} value={stats.total_cost.toFixed(2)} prefix={symbol} />
              </Card>
            </Col>
            <Col xs={24} sm={8}>
              <Card>
                <Statistic
                  title={`类别总盈亏 (${baseCurrency})`}
                  value={stats.total_pnl.toFixed(2)}
                  styles={{ content: {  color: pnlColor(stats.total_pnl)  } }}
                  suffix={`(${stats.total_pnl >= 0 ? "+" : ""}${stats.total_pnl_percent.toFixed(2)}%)`}
                />
              </Card>
            </Col>
          </Row>

          {stats.market_distribution.length > 0 && (
            <Row gutter={[16, 16]} className="mb-4">
              <Col xs={24} md={12}>
                <Card title="市场分布">
                  <PieChart data={stats.market_distribution} height={260} currencyCode={baseCurrency} />
                </Card>
              </Col>
            </Row>
          )}

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
                    pagination={{ pageSize: 20, showSizeChanger: true }}
                    expandable={{
                      expandedRowRender: (record) => (
                        <Table
                          columns={accountDetailColumns}
                          dataSource={record.accountRows}
                          rowKey="key"
                          size="small"
                          pagination={false}
                          className="ml-8 account-sub-table"
                        />
                      ),
                      rowExpandable: (record) => record.accountRows.length > 0,
                    }}
                  />
                </Card>
              </Col>
            </Row>
          )}
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
