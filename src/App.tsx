import { useEffect, useRef } from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { App as AntdApp } from "antd";
import type { NotificationInstance } from "antd/es/notification/interface";
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

const QUOTE_WARNING_KEY = "quote-warning";

// Polls `take_quote_warning` once and shows a notification if a warning is set.
// Uses the hook-based notification API passed in via ref to ensure it runs
// within React's component context (reliable in Tauri's WebKit webview).
async function pollWarning(notifRef: React.RefObject<NotificationInstance | null>) {
  try {
    const warning = await invoke<string | null>("take_quote_warning");
    if (warning && notifRef.current) {
      notifRef.current.warning({
        key: QUOTE_WARNING_KEY,
        message: "行情获取提示",
        description: warning,
        duration: 0,
        placement: "topRight",
      });
    }
  } catch {
    // ignore
  }
}

function App() {
  // Use the hook-based notification API from antd's App context.
  // This is more reliable than the static notification import in Tauri's WebKit
  // because it is backed by a React portal already mounted in the component tree.
  const { notification } = AntdApp.useApp();
  const notifRef = useRef<NotificationInstance | null>(null);
  notifRef.current = notification;

  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | null = null;
    let unlistenWarning: (() => void) | null = null;

    // Poll once immediately (catches any warning set before the event listeners
    // are registered, e.g. a very fast backend startup).
    pollWarning(notifRef);

    // Poll every 2 s for the first 30 s after startup as a guaranteed fallback.
    // The background refresh task runs ~2 s after launch, so this catches it
    // even if the Tauri event delivery is delayed or missed.
    let pollCount = 0;
    const MAX_POLLS = 15; // 15 × 2 s = 30 s
    const pollInterval = setInterval(() => {
      if (cancelled || pollCount >= MAX_POLLS) {
        clearInterval(pollInterval);
        return;
      }
      pollCount++;
      pollWarning(notifRef);
    }, 2000);

    // Primary path: Tauri emits `quote-warning` directly when the background
    // refresh encounters a Xueqiu error.
    listen<string>("quote-warning", (event) => {
      if (event.payload && notifRef.current) {
        notifRef.current.warning({
          key: QUOTE_WARNING_KEY,
          message: "行情获取提示",
          description: event.payload,
          duration: 0,
          placement: "topRight",
        });
        // Consume the stored warning so the polling fallback doesn't duplicate it.
        invoke("take_quote_warning").catch(() => {});
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unlistenWarning = fn;
    });

    // Secondary path: after the backend signals it has finished refreshing,
    // pull any remaining (un-consumed) warning.
    listen("quotes-refreshed", () => {
      pollWarning(notifRef);
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });

    return () => {
      cancelled = true;
      clearInterval(pollInterval);
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
