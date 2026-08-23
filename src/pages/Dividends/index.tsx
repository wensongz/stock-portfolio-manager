import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Button,
  Card,
  Col,
  Empty,
  Pagination,
  Row,
  Select,
  Spin,
  Statistic,
  Table,
  Typography,
  message,
} from "antd";
import { invoke } from "@tauri-apps/api/core";
import { DownOutlined, GiftOutlined } from "@ant-design/icons";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import type { Currency, CurrencyDividend, DividendAnalysis, DividendEntry } from "../../types";

const { Title, Text } = Typography;

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

const originalCurrencySymbol: Record<string, string> = {
  CNY: "¥",
  HKD: "HK$",
  USD: "US$",
};

const CURRENCY_OPTIONS: Currency[] = ["CNY", "USD", "HKD"];
type YearFilter = number | "all";
type SummaryMode = "currency" | "account" | "market";

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
  const [year, setYear] = useState<YearFilter>(() => new Date().getFullYear());
  const [summaryMode, setSummaryMode] = useState<SummaryMode>("currency");

  // Years that actually have dividend records (from all fetched years). We
  // fetch the current year by default; a small "available years" list comes
  // from a lightweight query.
  const [availableYears, setAvailableYears] = useState<number[]>([]);

  const loadYears = useCallback(async () => {
    try {
      // Distinct years with PAY transactions, newest first.
      const years = await invoke<number[]>("get_dividend_years");
      setAvailableYears(years);
      if (year !== "all" && years.length > 0 && !years.includes(year)) {
        setYear(years[0]);
      }
    } catch (err) {
      message.error(`获取分红年份失败: ${err}`);
    }
  }, [year]);

  const loadAnalysis = useCallback(async (selectedYear: YearFilter) => {
    setLoading(true);
    try {
      const data = await invoke<DividendAnalysis>("get_dividend_analysis", {
        year: selectedYear === "all" ? null : selectedYear,
      });
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

  // Convert one actual transaction-currency total to the selected base currency.
  // When rates haven't loaded yet, fall back to the native amount rather than
  // crashing (convertWithCachedRates dereferences the rates object).
  const convertCurrencyTotal = useCallback(
    (group: CurrencyDividend): number =>
      rates ? convertWithCachedRates(group.total, group.currency, baseCurrency) : group.total,
    [convertWithCachedRates, baseCurrency, rates]
  );

  // Grand total across actual transaction currencies in the selected base currency.
  const grandTotalBase = useMemo(() => {
    if (!analysis) return 0;
    return analysis.currencies.reduce((sum, group) => sum + convertCurrencyTotal(group), 0);
  }, [analysis, convertCurrencyTotal]);

  const baseSymbol = currencySymbol[baseCurrency] ?? "$";
  const baseName = currencyNames[baseCurrency] ?? baseCurrency;
  const yearLabel = year === "all" ? "全部" : `${year}年`;
  const grandOriginalAmounts = useMemo(() => {
    const amounts = new Map<string, number>();
    for (const group of analysis?.currencies ?? []) {
      amounts.set(group.currency, group.total);
    }
    return fmtOriginalAmounts(amounts);
  }, [analysis]);
  const dimensionCards = useMemo(() => {
    if (!analysis) return [];

    if (summaryMode === "currency") {
      return analysis.currencies.map((group) => ({
        key: group.currency,
        title: `${currencyNames[group.currency] ?? group.currency}分红`,
        value: convertCurrencyTotal(group),
        prefix: baseSymbol,
        original: fmt(group.total, originalCurrencySymbol[group.currency] ?? group.currency),
      }));
    }

    const groups = new Map<
      string,
      { label: string; total: number; originalAmounts: Map<string, number> }
    >();
    for (const entry of analysis.entries ?? []) {
      const key = summaryMode === "account" ? entry.accountId : entry.market;
      const label =
        summaryMode === "account"
          ? entry.accountName
          : marketNames[entry.market] ?? entry.market;
      const group = groups.get(key) ?? { label, total: 0, originalAmounts: new Map() };
      group.total += rates
        ? convertWithCachedRates(entry.total, entry.currency, baseCurrency)
        : entry.total;
      group.originalAmounts.set(
        entry.currency,
        (group.originalAmounts.get(entry.currency) ?? 0) + entry.total
      );
      groups.set(key, group);
    }

    return Array.from(groups, ([key, group]) => ({
      key,
      title: `${group.label}分红`,
      value: group.total,
      prefix: baseSymbol,
      original: fmtOriginalAmounts(group.originalAmounts),
    })).sort((a, b) => {
      if (summaryMode === "market") {
        const order: Record<string, number> = { CN: 0, HK: 1, US: 2 };
        return (order[a.key] ?? 3) - (order[b.key] ?? 3);
      }
      return a.title.localeCompare(b.title, "zh-CN");
    });
  }, [
    analysis,
    summaryMode,
    rates,
    convertWithCachedRates,
    baseCurrency,
    baseSymbol,
    convertCurrencyTotal,
  ]);

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      <div className="flex justify-between items-center">
        <Title level={2} className="!mb-0">
          <GiftOutlined style={{ color: "#f5222d" }} /> 分红分析
        </Title>
        <div style={{ display: "flex", gap: 12 }}>
          <Select
            value={summaryMode}
            onChange={setSummaryMode}
            style={{ width: 160 }}
            options={[
              { value: "currency", label: "按分红货币" },
              { value: "account", label: "按证券账户" },
              { value: "market", label: "按市场" },
            ]}
          />
          <Select
            value={year}
            onChange={setYear}
            style={{ width: 130 }}
            options={[
              { value: "all", label: "全部" },
              ...availableYears.map((y) => ({ value: y, label: `${y}年` })),
            ]}
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
      ) : !analysis || analysis.currencies.length === 0 ? (
        <Empty description="该年份暂无分红记录" />
      ) : (
        <>
          {/* Annual summary cards */}
          <Row gutter={[16, 16]}>
            <Col xs={24} sm={12} lg={6}>
              <Card>
                <Statistic
                  title={`${yearLabel}分红总计 (${baseName})`}
                  value={grandTotalBase.toFixed(2)}
                  prefix={baseSymbol}
                />
                <Text type="secondary" style={{ display: "block", marginTop: 8 }}>
                  {grandOriginalAmounts}
                </Text>
              </Card>
            </Col>
            {dimensionCards.map((card) => (
              <Col xs={24} sm={12} lg={6} key={card.key}>
                <Card>
                  <Statistic
                    title={card.title}
                    value={card.value.toFixed(2)}
                    prefix={card.prefix}
                  />
                  {card.original && (
                    <Text type="secondary" style={{ display: "block", marginTop: 8 }}>
                      {card.original}
                    </Text>
                  )}
                </Card>
              </Col>
            ))}
          </Row>

          <DividendTables
            entries={analysis.entries ?? []}
            mode={summaryMode}
            baseCurrency={baseCurrency}
            convert={(amount, from) =>
              rates ? convertWithCachedRates(amount, from, baseCurrency) : amount
            }
          />

          {/* Grand total */}
          <Card>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
              <Text strong style={{ fontSize: 16 }}>
                各币种分红总计（{baseName}）
              </Text>
              <Text strong style={{ fontSize: 20 }}>
                {fmt(grandTotalBase, baseSymbol)}
              </Text>
            </div>
            <Text type="secondary" style={{ fontSize: 12 }}>
              按每笔分红的实际入账币种汇总后，以所选币种换算（汇率口径与统计分析一致）。
            </Text>
          </Card>
        </>
      )}
    </div>
  );
}

interface DetailItem {
  key: string;
  label: string;
  amount: number;
  currency: string;
}

function fmtOriginalAmounts(amounts: Map<string, number>): string {
  return Array.from(amounts)
    .filter(([, amount]) => amount !== 0)
    .sort(([a], [b]) => currencyOrder(a) - currencyOrder(b))
    .map(([currency, amount]) => fmt(amount, originalCurrencySymbol[currency] ?? currency))
    .join(" + ");
}

interface SummaryRow {
  key: string;
  symbol: string;
  name: string;
  details: DetailItem[];
  total: number;
  totalCurrency: string;
}

interface SummarySection {
  key: string;
  title: string;
  rows: SummaryRow[];
}

const marketNames: Record<string, string> = {
  CN: "A股",
  HK: "港股",
  US: "美股",
};

const currencyOrder = (currency: string) =>
  ({ CNY: 0, USD: 1, HKD: 2 })[currency] ?? 3;

interface DetailViewState {
  expanded: boolean;
  page: number;
}

const DETAIL_PAGE_SIZE = 10;

function DetailColumnCell({
  items,
  view,
  side,
  onToggle,
  onPageChange,
}: {
  items: DetailItem[];
  view: DetailViewState;
  side: "label" | "amount";
  onToggle: () => void;
  onPageChange: (page: number) => void;
}) {
  const visibleItems = view.expanded
    ? items.slice((view.page - 1) * DETAIL_PAGE_SIZE, view.page * DETAIL_PAGE_SIZE)
    : items.slice(0, 2);

  return (
    <div
      style={{
        minWidth: side === "label" ? 150 : 240,
        position: "relative",
        paddingBottom: items.length > 2 ? 30 : 0,
        textAlign: side === "label" ? "left" : "right",
      }}
    >
      <div style={{ lineHeight: 1.8 }}>
        {visibleItems.map((item) => (
          <div key={`${item.key}-${side}`}>
            {side === "label"
              ? item.label
              : fmt(item.amount, currencySymbol[item.currency] ?? "")}
          </div>
        ))}
      </div>
      {side === "amount" && items.length > 2 && (
        <div
          style={{
            position: "absolute",
            right: 0,
            bottom: 0,
            display: "flex",
            alignItems: "center",
            gap: 8,
          }}
        >
          {view.expanded && items.length > DETAIL_PAGE_SIZE && (
            <Pagination
              size="small"
              simple
              current={view.page}
              pageSize={DETAIL_PAGE_SIZE}
              total={items.length}
              showSizeChanger={false}
              onChange={onPageChange}
            />
          )}
          <Button
            type="link"
            size="small"
            onClick={onToggle}
            style={{ padding: 0, height: 24 }}
            icon={<DownOutlined rotate={view.expanded ? 180 : 0} />}
          >
            {view.expanded ? "收起" : "展开"}
          </Button>
        </div>
      )}
    </div>
  );
}

function buildSections(
  entries: DividendEntry[],
  mode: SummaryMode,
  baseCurrency: Currency,
  convert: (amount: number, from: Currency) => number
): SummarySection[] {
  const sectionMap = new Map<string, { title: string; entries: DividendEntry[] }>();

  for (const entry of entries) {
    const sectionKey =
      mode === "currency" ? entry.currency : mode === "account" ? entry.accountId : entry.market;
    const title =
      mode === "currency"
        ? `${currencyNames[entry.currency] ?? entry.currency}（${entry.currency}）`
        : mode === "account"
          ? entry.accountName
          : marketNames[entry.market] ?? entry.market;
    const section = sectionMap.get(sectionKey) ?? { title, entries: [] };
    section.entries.push(entry);
    sectionMap.set(sectionKey, section);
  }

  const sections = Array.from(sectionMap, ([key, section]) => {
    const companyMap = new Map<string, DividendEntry[]>();
    for (const entry of section.entries) {
      const companyEntries = companyMap.get(entry.symbol) ?? [];
      companyEntries.push(entry);
      companyMap.set(entry.symbol, companyEntries);
    }

    const rows = Array.from(companyMap, ([symbol, companyEntries]) => {
      const first = companyEntries[0];
      const detailMap = new Map<string, DetailItem>();

      for (const entry of companyEntries) {
        const detailKey =
          mode === "currency"
            ? entry.accountId
            : `${entry.month}-${entry.currency}`;
        const detailLabel =
          mode === "currency"
            ? entry.accountName
            : `${entry.month.slice(0, 4)}-${entry.month.slice(4, 6)}`;
        const existing = detailMap.get(detailKey);
        if (existing) {
          existing.amount += entry.total;
        } else {
          detailMap.set(detailKey, {
            key: detailKey,
            label: detailLabel,
            amount: entry.total,
            currency: entry.currency,
          });
        }
      }

      const details = Array.from(detailMap.values()).sort((a, b) =>
        mode === "currency"
          ? a.label.localeCompare(b.label, "zh-CN")
          : b.label.localeCompare(a.label) ||
            a.currency.localeCompare(b.currency) ||
            b.amount - a.amount
      );
      const total = companyEntries.reduce(
        (sum, entry) =>
          sum +
          (mode === "currency"
            ? entry.total
            : convert(entry.total, entry.currency as Currency)),
        0
      );

      return {
        key: `${key}-${symbol}`,
        symbol,
        name: first.name,
        details,
        total,
        totalCurrency: mode === "currency" ? first.currency : baseCurrency,
      };
    }).sort((a, b) => b.total - a.total);

    return { key, title: section.title, rows };
  });

  return sections.sort((a, b) => {
    if (mode === "currency") return currencyOrder(a.key) - currencyOrder(b.key);
    if (mode === "market") {
      const order: Record<string, number> = { CN: 0, HK: 1, US: 2 };
      return (order[a.key] ?? 3) - (order[b.key] ?? 3);
    }
    return a.title.localeCompare(b.title, "zh-CN");
  });
}

function DividendTables({
  entries,
  mode,
  baseCurrency,
  convert,
}: {
  entries: DividendEntry[];
  mode: SummaryMode;
  baseCurrency: Currency;
  convert: (amount: number, from: Currency) => number;
}) {
  const [detailViews, setDetailViews] = useState<Record<string, DetailViewState>>({});
  const sections = useMemo(
    () => buildSections(entries, mode, baseCurrency, convert),
    [entries, mode, baseCurrency, convert]
  );

  useEffect(() => {
    setDetailViews({});
  }, [entries, mode]);

  const getDetailView = (rowKey: string): DetailViewState =>
    detailViews[rowKey] ?? { expanded: false, page: 1 };

  const toggleDetails = (rowKey: string) => {
    setDetailViews((current) => {
      const previous = current[rowKey] ?? { expanded: false, page: 1 };
      return {
        ...current,
        [rowKey]: { expanded: !previous.expanded, page: 1 },
      };
    });
  };

  const changeDetailPage = (rowKey: string, page: number) => {
    setDetailViews((current) => ({
      ...current,
      [rowKey]: { expanded: true, page },
    }));
  };

  const detailGroupTitle = mode === "currency" ? "账户" : "年月";

  const columns = [
    {
      title: "公司",
      dataIndex: "symbol",
      key: "symbol",
      width: 260,
      render: (_: unknown, row: SummaryRow) => (
        <span>
          <strong>{row.name}</strong>
          <span className="text-sm" style={{ color: "var(--color-text-secondary)" }}>
            {" "}
            {row.symbol}
          </span>
        </span>
      ),
    },
    {
      title: "明细",
      key: "details",
      children: [
        {
          title: detailGroupTitle,
          dataIndex: "details",
          key: "detailLabel",
          align: "left" as const,
          width: 220,
          onCell: () => ({ style: { borderInlineEnd: "none" } }),
          render: (details: DetailItem[], row: SummaryRow) => (
            <DetailColumnCell
              items={details}
              view={getDetailView(row.key)}
              side="label"
              onToggle={() => toggleDetails(row.key)}
              onPageChange={(page) => changeDetailPage(row.key, page)}
            />
          ),
        },
        {
          title: "金额",
          dataIndex: "details",
          key: "detailAmount",
          align: "right" as const,
          width: 300,
          render: (details: DetailItem[], row: SummaryRow) => (
            <DetailColumnCell
              items={details}
              view={getDetailView(row.key)}
              side="amount"
              onToggle={() => toggleDetails(row.key)}
              onPageChange={(page) => changeDetailPage(row.key, page)}
            />
          ),
        },
      ],
    },
    {
      title: "小计",
      dataIndex: "total",
      key: "total",
      align: "right" as const,
      width: 210,
      render: (total: number, row: SummaryRow) =>
        fmt(total, currencySymbol[row.totalCurrency] ?? ""),
    },
  ];

  return (
    <>
      {sections.map((section) => (
        <Card key={section.key} title={section.title}>
          <Table
            columns={columns}
            dataSource={section.rows}
            rowKey="key"
            size="small"
            pagination={false}
            scroll={{ x: 990 }}
            bordered
          />
        </Card>
      ))}
    </>
  );
}
