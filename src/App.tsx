import { useEffect } from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { notification } from "antd";
import MainLayout from "./components/Layout/MainLayout";
import DashboardPage from "./pages/Dashboard";
import AccountsPage from "./pages/Accounts";
import HoldingsPage from "./pages/Holdings";
import TransactionsPage from "./pages/Transactions";
import CategoriesPage from "./pages/Categories";
import StatisticsPage from "./pages/Statistics";
import PerformancePage from "./pages/Performance";
import QuarterlyPage from "./pages/Quarterly";
import SnapshotDetail from "./pages/Quarterly/SnapshotDetail";
import QuarterComparisonPage from "./pages/Quarterly/QuarterComparison";
import TrendsPage from "./pages/Quarterly/TrendsPage";
import ImportPage from "./pages/Import";
import AlertsPage from "./pages/Alerts";
import ReviewPage from "./pages/Review";
import SettingsPage from "./pages/Settings";

const QUOTE_WARNING_NOTIFICATION_KEY = "quote-warning";

function showQuoteWarning(warning: string) {
  notification.warning({
    key: QUOTE_WARNING_NOTIFICATION_KEY,
    message: "行情获取提示",
    description: warning,
    duration: 0,
    placement: "topRight",
  });
}

function App() {
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let unlistenWarning: (() => void) | null = null;
    let cancelled = false;

    const pullQuoteWarning = async () => {
      try {
        const warning = await invoke<string | null>("take_quote_warning");
        if (warning) {
          showQuoteWarning(warning);
        }
      } catch {
        // ignore
      }
    };

    pullQuoteWarning();

    listen("quotes-refreshed", () => {
      pullQuoteWarning();
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlisten = fn;
      }
    });

    listen<string>("quote-warning", (event) => {
      const warning = event.payload;
      if (warning) {
        showQuoteWarning(warning);
        // Consume the warning so the fallback pullQuoteWarning() on
        // quotes-refreshed does not show a duplicate notification.
        invoke("take_quote_warning").catch(() => {});
      }
    }).then((fn) => {
      if (cancelled) {
        fn();
      } else {
        unlistenWarning = fn;
      }
    });

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
      if (unlistenWarning) unlistenWarning();
    };
  }, []);

  return (
    <MainLayout>
      <Routes>
        <Route path="/" element={<Navigate to="/dashboard" replace />} />
        <Route path="/dashboard" element={<DashboardPage />} />
        <Route path="/statistics" element={<StatisticsPage />} />
        <Route path="/performance" element={<PerformancePage />} />
        <Route path="/accounts" element={<AccountsPage />} />
        <Route path="/holdings" element={<HoldingsPage />} />
        <Route path="/transactions" element={<TransactionsPage />} />
        <Route path="/categories" element={<CategoriesPage />} />
        <Route path="/quarterly" element={<QuarterlyPage />} />
        <Route path="/quarterly/compare" element={<QuarterComparisonPage />} />
        <Route path="/quarterly/trends" element={<TrendsPage />} />
        <Route path="/quarterly/:snapshotId" element={<SnapshotDetail />} />
        <Route path="/import" element={<ImportPage />} />
        <Route path="/alerts" element={<AlertsPage />} />
        <Route path="/review" element={<ReviewPage />} />
        <Route path="/settings" element={<SettingsPage />} />
      </Routes>
    </MainLayout>
  );
}

export default App;
