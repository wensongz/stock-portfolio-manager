import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Card,
  Col,
  Empty,
  Row,
  Select,
  Spin,
  Statistic,
  Table,
  Typography,
  message,
} from "antd";
import { invoke } from "@tauri-apps/api/core";
import { GiftOutlined } from "@ant-design/icons";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import type { Currency, DividendAnalysis, MarketDividend } from "../../types";

const { Title, Text } = Typography;

const marketCurrency: Record<string, { code: Currency; symbol: string; label: string }> = {
  CN: { code: "CNY", symbol: "¥", label: "🇨🇳 A股" },
  US: { code: "USD", symbol: "$", label: "🇺🇸 美股" },
  HK: { code: "HKD", symbol: "HK$", label: "🇭🇰 港股" },
};

const currencyNames: Record<string, string> = {
  USD: "美元",
  CNY: "人民币",
  HKD: "港元",
};

/** Currency code → symbol (for the selected summary currency). */
const currencySymbol: Record<string, string> = {
  USD: "$",
  CNY: "¥",
  HKD: "HK$",
};

const CURRENCY_OPTIONS: Currency[] = ["CNY", "USD", "HKD"];

function fmt(amount: number, symbol: string): string {
  return `${symbol}${amount.toLocaleString("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 2,
  })}`;
}

export default function DividendsPage() {
  const { baseCurrency, setBaseCurrency, convertWithCachedRates, rates, fetchRates } =
    useExchangeRateStore();
  const [analysis, setAnalysis] = useState<DividendAnalysis | null>(null);
  const [loading, setLoading] = useState(false);
  const [year, setYear] = useState<number>(() => new Date().getFullYear());

  // Years that actually have dividend records (from all fetched years). We
  // fetch the current year by default; a small "available years" list comes
  // from a lightweight query.
  const [availableYears, setAvailableYears] = useState<number[]>([]);

  const loadYears = useCallback(async () => {
    try {
      // Distinct years with PAY transactions, newest first.
      const years = await invoke<number[]>("get_dividend_years");
      setAvailableYears(years);
      if (years.length > 0 && !years.includes(year)) {
        setYear(years[0]);
      }
    } catch (err) {
      message.error(`获取分红年份失败: ${err}`);
    }
  }, [year]);

  const loadAnalysis = useCallback(async (y: number) => {
    setLoading(true);
    try {
      const data = await invoke<DividendAnalysis>("get_dividend_analysis", { year: y });
      setAnalysis(data);
    } catch (err) {
      message.error(`获取分红分析失败: ${err}`);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    // Ensure exchange rates are loaded before any currency conversion, or
    // convertWithCachedRates would dereference a null rates object and crash.
    if (!rates) fetchRates();
  }, [rates, fetchRates]);

  useEffect(() => {
    loadYears();
  }, [loadYears]);

  useEffect(() => {
    loadAnalysis(year);
  }, [year, loadAnalysis]);

  // Convert a market's native-currency total to the selected base currency.
  // When rates haven't loaded yet, fall back to the native amount rather than
  // crashing (convertWithCachedRates dereferences the rates object).
  const convertMarketTotal = useCallback(
    (m: MarketDividend): number =>
      rates ? convertWithCachedRates(m.total, m.currency, baseCurrency) : m.total,
    [convertWithCachedRates, baseCurrency, rates]
  );

  // Grand total across markets in the selected base currency.
  const grandTotalBase = useMemo(() => {
    if (!analysis) return 0;
    return analysis.markets.reduce((s, m) => s + convertMarketTotal(m), 0);
  }, [analysis, convertMarketTotal]);

  const baseSymbol = currencySymbol[baseCurrency] ?? "$";
  const baseName = currencyNames[baseCurrency] ?? baseCurrency;

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      <div className="flex justify-between items-center">
        <Title level={2} className="!mb-0">
          <GiftOutlined style={{ color: "#f5222d" }} /> 分红分析
        </Title>
        <div style={{ display: "flex", gap: 12 }}>
          <Select
            value={year}
            onChange={setYear}
            style={{ width: 130 }}
            options={availableYears.map((y) => ({ value: y, label: `${y}年` }))}
            placeholder="选择年份"
          />
          <Select
            value={baseCurrency}
            onChange={setBaseCurrency}
            style={{ width: 140 }}
            options={CURRENCY_OPTIONS.map((c) => ({
              value: c,
              label: `${c} ${currencyNames[c]}`,
            }))}
            placeholder="总计币种"
          />
        </div>
      </div>

      {loading && !analysis ? (
        <div className="flex justify-center py-16">
          <Spin size="large" />
        </div>
      ) : !analysis || analysis.markets.length === 0 ? (
        <Empty description="该年份暂无分红记录" />
      ) : (
        <>
          {/* Annual summary cards */}
          <Row gutter={[16, 16]}>
            <Col xs={24} sm={12} lg={6}>
              <Card>
                <Statistic
                  title={`${year}年分红总计 (${baseName})`}
                  value={grandTotalBase.toFixed(2)}
                  prefix={baseSymbol}
                />
              </Card>
            </Col>
            {analysis.markets.map((m) => (
              <Col xs={24} sm={12} lg={6} key={m.market}>
                <Card>
                  <Statistic
                    title={`${marketCurrency[m.market]?.label ?? m.market} 分红`}
                    value={m.total.toFixed(2)}
                    prefix={marketCurrency[m.market]?.symbol ?? ""}
                    suffix={`≈ ${fmt(convertMarketTotal(m), baseSymbol)}`}
                  />
                </Card>
              </Col>
            ))}
          </Row>

          {/* Per-market tables */}
          {analysis.markets.map((m) => (
            <MarketTable key={m.market} market={m} />
          ))}

          {/* Grand total */}
          <Card>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <Text strong style={{ fontSize: 16 }}>
                三个市场分红总计（{baseName}）
              </Text>
              <Text strong style={{ fontSize: 20 }}>
                {fmt(grandTotalBase, baseSymbol)}
              </Text>
            </div>
            <Text type="secondary" style={{ fontSize: 12 }}>
              按各市场本位币汇总后，以所选币种换算（汇率口径与统计分析一致）。
            </Text>
          </Card>
        </>
      )}
    </div>
  );
}

/** One market's dividend table: row = company, column = account + 合计. */
export function MarketTable({ market }: { market: MarketDividend }) {
  const currencyCode = marketCurrency[market.market]?.code ?? "USD";
  const symbol = marketCurrency[market.market]?.symbol ?? "$";
  const label = marketCurrency[market.market]?.label ?? market.market;

  const columns = useMemo(() => {
    const cols: any[] = [
      {
        title: "公司",
        dataIndex: "symbol",
        key: "symbol",
        fixed: "left" as const,
        ellipsis: true,
        width: 175,
        render: (_: unknown, row: { symbol: string; name: string }) => (
          <span>
            <strong>{row.symbol}</strong>
            <span className="text-sm" style={{ color: "var(--color-text-secondary)" }}>
              {" "}
              {row.name}
            </span>
          </span>
        ),
      },
      ...market.accounts.map((a) => ({
        title: a.accountName,
        dataIndex: "perAccount",
        key: a.accountId,
        align: "right" as const,
        width: 130,
        render: (_: unknown, row: { perAccount: [string, number][] }) => {
          const entry = (Array.isArray(row.perAccount) ? row.perAccount : []).find(
            (e) => Array.isArray(e) && e[0] === a.accountId
          );
          const amount = typeof entry?.[1] === "number" ? entry[1] : 0;
          return amount !== 0 ? fmt(amount, symbol) : "0.00";
        },
      })),
      {
        title: "小计",
        dataIndex: "total",
        key: "total",
        align: "right" as const,
        width: 140,
        render: (total: number) => fmt(total, symbol),
      },
    ];
    return cols;
  }, [market.accounts, symbol]);

  const dataSource = useMemo(
    () =>
      market.rows.map((r) => ({
        ...r,
        key: r.symbol,
      })),
    [market.rows]
  );

  // Summary row (via Table.summary so it aligns with the columns): label +
  // per-account totals + the market total under the 小计 column.
  const summary = () => {
    const acctTotals = market.accounts.map((a) => {
      let sum = 0;
      for (const r of market.rows) {
        const found = (Array.isArray(r.perAccount) ? r.perAccount : []).find(
          (entry) => Array.isArray(entry) && entry[0] === a.accountId
        );
        sum += typeof found?.[1] === "number" ? found[1] : 0;
      }
      return sum;
    });
    return (
      <Table.Summary.Row>
        <Table.Summary.Cell index={0} colSpan={1}>
          <Text strong>合计</Text>
        </Table.Summary.Cell>
        {acctTotals.map((t, i) => (
          <Table.Summary.Cell key={i} index={i + 1} align="right">
            <Text strong>{fmt(t, symbol)}</Text>
          </Table.Summary.Cell>
        ))}
        <Table.Summary.Cell index={acctTotals.length + 1} align="right">
          <Text strong>{fmt(market.total, symbol)}</Text>
        </Table.Summary.Cell>
      </Table.Summary.Row>
    );
  };

  return (
    <Card title={`${label} 分红明细（${currencyCode}）`}>
      <Table
        columns={columns}
        dataSource={dataSource}
        rowKey="symbol"
        size="small"
        pagination={false}
        scroll={{ x: "max-content" }}
        summary={summary}
        bordered
      />
    </Card>
  );
}
