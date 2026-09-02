import { useCallback, useEffect, useMemo, useState } from "react";
import { Typography, Tabs, Button, Select } from "antd";
import { ReloadOutlined, BarChartOutlined } from "@ant-design/icons";
import dayjs from "dayjs";
import {
  statisticsViewKey,
  useStatisticsStore,
} from "../../stores/statisticsStore";
import { useAccountStore } from "../../stores/accountStore";
import { useCategoryStore } from "../../stores/categoryStore";
import { useQuoteStore } from "../../stores/quoteStore";
import { useExchangeRateStore } from "../../stores/exchangeRateStore";
import type { Currency, Market } from "../../types";
import OverviewTab from "./OverviewTab";
import MarketTab from "./MarketTab";
import AccountTab from "./AccountTab";
import CategoryTab from "./CategoryTab";
import {
  resolveStatisticsView,
  type StatisticsSelection,
} from "./statisticsView";

const { Title, Text } = Typography;

const MARKET_STORAGE_KEY = "statistics_selected_market";
const VALID_MARKETS = ["US", "CN", "HK"];

function loadSelectedMarket(): string | null {
  const stored = localStorage.getItem(MARKET_STORAGE_KEY);
  return stored && VALID_MARKETS.includes(stored) ? stored : null;
}

export default function StatisticsPage() {
  const [activeTab, setActiveTab] =
    useState<StatisticsSelection["activeTab"]>("overview");
  const [selectedAccountId, setSelectedAccountId] = useState("");
  const [selectedCategoryId, setSelectedCategoryId] = useState("");
  const [selectedMarket, setSelectedMarket] = useState(
    () => loadSelectedMarket() ?? "CN",
  );
  const [refreshing, setRefreshing] = useState(false);

  const { baseCurrency, setBaseCurrency } = useExchangeRateStore();
  const {
    overview,
    accountStats,
    loadingByView,
    fetchView,
  } = useStatisticsStore();
  const { accounts, fetchAccounts } = useAccountStore();
  const { categories, fetchCategories } = useCategoryStore();
  const { fetchHoldingQuotes, lastUpdatedAt } = useQuoteStore();

  const availableMarkets = useMemo(() => {
    const markets = new Set(
      (overview?.holdings ?? []).map((holding) => holding.market),
    );
    return VALID_MARKETS.filter((market) => markets.has(market as Market));
  }, [overview]);

  const loadCurrentView = useCallback(
    (overrides: Partial<StatisticsSelection> = {}) => {
      const view = resolveStatisticsView({
        activeTab,
        baseCurrency,
        selectedMarket,
        selectedAccountId,
        selectedCategoryId,
        ...overrides,
      });
      return view ? fetchView(view) : Promise.resolve();
    }, [
      activeTab,
      baseCurrency,
      selectedMarket,
      selectedAccountId,
      selectedCategoryId,
      fetchView,
    ],
  );

  useEffect(() => {
    void Promise.all([
      fetchAccounts(),
      fetchCategories(),
      fetchView({
        kind: "overview",
        baseCurrency: useExchangeRateStore.getState().baseCurrency,
      }),
    ]);
  }, [fetchAccounts, fetchCategories, fetchView]);

  useEffect(() => {
    if (
      availableMarkets.length > 0 &&
      !loadSelectedMarket() &&
      selectedMarket !== availableMarkets[0]
    ) {
      const market = availableMarkets[0];
      setSelectedMarket(market);
      if (activeTab === "market") {
        void loadCurrentView({ selectedMarket: market });
      }
    }
  }, [availableMarkets, selectedMarket, activeTab, loadCurrentView]);

  useEffect(() => {
    if (accounts.length > 0 && !selectedAccountId) {
      const accountId = accounts[0].id;
      setSelectedAccountId(accountId);
      if (activeTab === "account") {
        void loadCurrentView({ selectedAccountId: accountId });
      }
    }
  }, [accounts, selectedAccountId, activeTab, loadCurrentView]);

  useEffect(() => {
    if (categories.length > 0 && !selectedCategoryId) {
      const categoryId = categories[0].id;
      setSelectedCategoryId(categoryId);
      if (activeTab === "category") {
        void loadCurrentView({ selectedCategoryId: categoryId });
      }
    }
  }, [categories, selectedCategoryId, activeTab, loadCurrentView]);

  const handleTabChange = (tab: string) => {
    const nextTab = tab as StatisticsSelection["activeTab"];
    setActiveTab(nextTab);
    void loadCurrentView({ activeTab: nextTab });
  };

  const handleMarketChange = (market: string) => {
    localStorage.setItem(MARKET_STORAGE_KEY, market);
    setSelectedMarket(market);
    if (activeTab === "market") {
      void loadCurrentView({ selectedMarket: market });
    }
  };

  const handleAccountChange = (accountId: string) => {
    setSelectedAccountId(accountId);
    if (activeTab === "account") {
      void loadCurrentView({ selectedAccountId: accountId });
    }
  };

  const handleCategoryChange = (categoryId: string) => {
    setSelectedCategoryId(categoryId);
    if (activeTab === "category") {
      void loadCurrentView({ selectedCategoryId: categoryId });
    }
  };

  const handleCurrencyChange = (currency: Currency) => {
    setBaseCurrency(currency);
    if (activeTab === "overview" || activeTab === "category") {
      void loadCurrentView({ baseCurrency: currency });
    }
  };

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      if (activeTab === "account" && selectedAccountId) {
        const accountHoldings =
          accountStats[selectedAccountId]?.holdings ?? overview?.holdings ?? [];
        const seen = new Set<string>();
        const symbols: [string, string][] = [];
        for (const holding of accountHoldings) {
          if (
            holding.account_id === selectedAccountId &&
            !seen.has(holding.symbol)
          ) {
            seen.add(holding.symbol);
            symbols.push([holding.symbol, holding.market]);
          }
        }
        await fetchHoldingQuotes(symbols);
      } else {
        await fetchHoldingQuotes();
      }
      await loadCurrentView();
    } finally {
      setRefreshing(false);
    }
  };

  const currentView = resolveStatisticsView({
    activeTab,
    baseCurrency,
    selectedMarket,
    selectedAccountId,
    selectedCategoryId,
  });
  const currentLoading = currentView
    ? (loadingByView[statisticsViewKey(currentView)] ?? false)
    : false;

  const tabs = [
    {
      key: "overview",
      label: "整体统计",
      children: <OverviewTab baseCurrency={baseCurrency} />,
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
          onAccountChange={handleAccountChange}
        />
      ),
    },
    {
      key: "category",
      label: "按类别",
      children: (
        <CategoryTab
          selectedCategoryId={selectedCategoryId}
          onCategoryChange={handleCategoryChange}
          baseCurrency={baseCurrency}
        />
      ),
    },
  ];

  return (
    <div>
      <div className="flex justify-between items-center mb-4">
        <Title level={2} className="!mb-0">
          <BarChartOutlined style={{ color: "#1677ff" }} /> 统计分析
        </Title>
        <div className="flex items-center gap-2">
          <Button
            icon={<ReloadOutlined />}
            onClick={handleRefresh}
            loading={refreshing || currentLoading}
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
        onChange={handleTabChange}
        items={tabs}
        destroyOnHidden={false}
        tabBarExtraContent={
          lastUpdatedAt ? (
            <Text type="secondary" style={{ fontSize: 12 }}>
              行情数据更新于 {dayjs(lastUpdatedAt).format("YYYY-MM-DD HH:mm:ss")}
            </Text>
          ) : null
        }
      />
    </div>
  );
}
