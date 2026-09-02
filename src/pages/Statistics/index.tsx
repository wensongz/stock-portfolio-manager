import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
import { createStatisticsDispatcher } from "./statisticsDispatcher";
import { resolveAccountHoldingsCoverage } from "./statisticsAccountHoldings";

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
    overviewByCurrency,
    loadingByView,
    fetchView,
  } = useStatisticsStore();
  const { accounts, fetchAccounts } = useAccountStore();
  const { categories, fetchCategories } = useCategoryStore();
  const { fetchHoldingQuotes, lastUpdatedAt } = useQuoteStore();
  const overview = overviewByCurrency[baseCurrency] ?? null;

  const selectionRef = useRef<StatisticsSelection>({
    activeTab,
    baseCurrency,
    selectedMarket,
    selectedAccountId,
    selectedCategoryId,
  });
  selectionRef.current = {
    activeTab,
    baseCurrency,
    selectedMarket,
    selectedAccountId,
    selectedCategoryId,
  };

  const updateSelection = useCallback(
    (selection: StatisticsSelection) => {
      selectionRef.current = selection;
      setActiveTab(selection.activeTab);
      setSelectedMarket(selection.selectedMarket);
      setSelectedAccountId(selection.selectedAccountId);
      setSelectedCategoryId(selection.selectedCategoryId);
      setBaseCurrency(selection.baseCurrency);
    },
    [setBaseCurrency],
  );

  const getAccountHoldings = useCallback(
    (accountId: string, currency: Currency) => {
      return resolveAccountHoldingsCoverage(
        useStatisticsStore.getState(),
        accountId,
        currency,
      );
    },
    [],
  );

  const dispatcher = useMemo(
    () =>
      createStatisticsDispatcher({
        getSelection: () => selectionRef.current,
        updateSelection,
        fetchView,
        fetchHoldingQuotes,
        getAccountHoldings,
      }),
    [fetchView, fetchHoldingQuotes, getAccountHoldings, updateSelection],
  );

  const availableMarkets = useMemo(() => {
    const markets = new Set(
      (overview?.holdings ?? []).map((holding) => holding.market),
    );
    return VALID_MARKETS.filter((market) => markets.has(market as Market));
  }, [overview]);

  useEffect(() => {
    void Promise.all([
      fetchAccounts(),
      fetchCategories(),
      dispatcher.initialize(),
    ]);
  }, [dispatcher, fetchAccounts, fetchCategories]);

  useEffect(() => {
    if (
      availableMarkets.length > 0 &&
      !loadSelectedMarket() &&
      selectedMarket !== availableMarkets[0]
    ) {
      const market = availableMarkets[0];
      void dispatcher.changeMarket(market);
    }
  }, [availableMarkets, selectedMarket, dispatcher]);

  useEffect(() => {
    if (accounts.length > 0 && !selectedAccountId) {
      const accountId = accounts[0].id;
      void dispatcher.changeAccount(accountId);
    }
  }, [accounts, selectedAccountId, dispatcher]);

  useEffect(() => {
    if (categories.length > 0 && !selectedCategoryId) {
      const categoryId = categories[0].id;
      void dispatcher.changeCategory(categoryId);
    }
  }, [categories, selectedCategoryId, dispatcher]);

  const handleTabChange = (tab: string) => {
    const nextTab = tab as StatisticsSelection["activeTab"];
    void dispatcher.changeTab(nextTab);
  };

  const handleMarketChange = (market: string) => {
    localStorage.setItem(MARKET_STORAGE_KEY, market);
    void dispatcher.changeMarket(market);
  };

  const handleAccountChange = (accountId: string) => {
    void dispatcher.changeAccount(accountId);
  };

  const handleCategoryChange = (categoryId: string) => {
    void dispatcher.changeCategory(categoryId);
  };

  const handleCurrencyChange = (currency: Currency) => {
    void dispatcher.changeCurrency(currency);
  };

  const handleRefresh = async () => {
    setRefreshing(true);
    try {
      await dispatcher.refresh();
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
