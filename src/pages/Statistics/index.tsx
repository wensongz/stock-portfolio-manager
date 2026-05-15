import { useState, useEffect, useMemo } from "react";
import { Typography, Tabs, Button, Select } from "antd";
import { ReloadOutlined } from "@ant-design/icons";
import { useStatisticsStore } from "../../stores/dashboardStore";
import { useAccountStore } from "../../stores/accountStore";
import { useCategoryStore } from "../../stores/categoryStore";
import { useHoldingStore } from "../../stores/holdingStore";
import { useQuoteStore } from "../../stores/quoteStore";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import type { Currency, Market } from "../../types";
import OverviewTab from "./OverviewTab";
import MarketTab from "./MarketTab";
import AccountTab from "./AccountTab";
import CategoryTab from "./CategoryTab";

const { Title, Text } = Typography;

const MARKET_STORAGE_KEY = "statistics_selected_market";
const VALID_MARKETS = ["US", "CN", "HK"];

function loadSelectedMarket(): string | null {
  const stored = localStorage.getItem(MARKET_STORAGE_KEY);
  return stored && VALID_MARKETS.includes(stored) ? stored : null;
}

export default function StatisticsPage() {
  const [activeTab, setActiveTab] = useState("overview");
  const [selectedAccountId, setSelectedAccountId] = useState("");
  const [selectedCategoryId, setSelectedCategoryId] = useState("");
  const [refreshing, setRefreshing] = useState(false);

  const { baseCurrency, setBaseCurrency } = useExchangeRateStore();

  const {
    overview, loadingOverview,
    fetchOverview, fetchMarketStats, fetchAccountStats, fetchCategoryStats,
  } = useStatisticsStore();
  const { accounts, fetchAccounts } = useAccountStore();
  const { categories, fetchCategories } = useCategoryStore();
  const { holdings, fetchHoldings } = useHoldingStore();
  const { fetchHoldingQuotes } = useQuoteStore();

  // Derive available markets from holdings
  const availableMarkets = useMemo(() => {
    const marketSet = new Set(holdings.map((h) => h.market));
    return VALID_MARKETS.filter((m) => marketSet.has(m as Market));
  }, [holdings]);

  // Determine the initial selected market: prefer saved value, then first market with holdings
  const [selectedMarket, setSelectedMarket] = useState<string>(() => {
    return loadSelectedMarket() ?? "CN";
  });

  // Once holdings are loaded, if no saved preference exists, pick the first available market
  useEffect(() => {
    if (availableMarkets.length > 0 && !loadSelectedMarket()) {
      setSelectedMarket(availableMarkets[0]);
    }
  }, [availableMarkets]);

  const handleMarketChange = (market: string) => {
    localStorage.setItem(MARKET_STORAGE_KEY, market);
    setSelectedMarket(market);
  };

  const handleCurrencyChange = (currency: Currency) => {
    setBaseCurrency(currency);
    fetchOverview(currency);
    if (selectedCategoryId) fetchCategoryStats(selectedCategoryId, currency);
  };

  useEffect(() => {
    fetchAccounts();
    fetchCategories();
    fetchHoldings();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []);

  useEffect(() => {
    fetchOverview(baseCurrency);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [baseCurrency]);

  // Preselect first account and category
  useEffect(() => {
    if (accounts.length > 0 && !selectedAccountId) {
      setSelectedAccountId(accounts[0].id);
    }
  }, [accounts, selectedAccountId]);

  useEffect(() => {
    if (categories.length > 0 && !selectedCategoryId) {
      setSelectedCategoryId(categories[0].id);
    }
  }, [categories, selectedCategoryId]);

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      // When the "By Account" tab is active with a selected account, only
      // force-refresh the quotes for that account's holdings.
      if (activeTab === "account" && selectedAccountId) {
        const seen = new Set<string>();
        const symbols: [string, string][] = [];
        for (const h of holdings) {
          if (h.account_id === selectedAccountId && !seen.has(h.symbol)) {
            seen.add(h.symbol);
            symbols.push([h.symbol, h.market]);
          }
        }
        await fetchHoldingQuotes(symbols);
      } else {
        // Force-refresh all quotes from the API
        await fetchHoldingQuotes();
      }
      // Re-fetch all statistics data using the now-fresh cache.
      // Since the backend reads from cache only, these are fast.
      const promises: Promise<void>[] = [fetchOverview(baseCurrency)];
      promises.push(fetchMarketStats(selectedMarket));
      if (selectedAccountId) promises.push(fetchAccountStats(selectedAccountId));
      if (selectedCategoryId) promises.push(fetchCategoryStats(selectedCategoryId, baseCurrency));
      await Promise.all(promises);
    } finally {
      setRefreshing(false);
    }
  };

  const tabs = [
    {
      key: "overview",
      label: "整体统计",
      children: <OverviewTab overview={overview} loading={loadingOverview} baseCurrency={baseCurrency} />,
    },
    {
      key: "market",
      label: "按市场",
      children: (
        <MarketTab
          selectedMarket={selectedMarket}
          onMarketChange={handleMarketChange}
        />
      ),
    },
    {
      key: "account",
      label: "按账户",
      children: (
        <AccountTab
          selectedAccountId={selectedAccountId}
          onAccountChange={setSelectedAccountId}
        />
      ),
    },
    {
      key: "category",
      label: "按类别",
      children: (
        <CategoryTab
          selectedCategoryId={selectedCategoryId}
          onCategoryChange={setSelectedCategoryId}
          baseCurrency={baseCurrency}
        />
      ),
    },
  ];

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <Title level={2} className="!mb-0">
          📈 统计分析
        </Title>
        <div className="flex items-center gap-2">
          <Button
            icon={<ReloadOutlined />}
            onClick={handleRefresh}
            loading={refreshing || loadingOverview}
            size="small"
          >
            刷新
          </Button>
          <Text type="secondary">基准货币:</Text>
          <Select
            value={baseCurrency}
            onChange={handleCurrencyChange}
            size="small"
            style={{ width: 120 }}
          >
            <Select.Option value="USD">USD 美元</Select.Option>
            <Select.Option value="CNY">CNY 人民币</Select.Option>
            <Select.Option value="HKD">HKD 港元</Select.Option>
          </Select>
        </div>
      </div>

      <Tabs
        activeKey={activeTab}
        onChange={setActiveTab}
        items={tabs}
        destroyInactiveTabPane={false}
      />
    </div>
  );
}
