import { useState, useEffect } from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { App as AntdApp } from "antd";
import { useQuoteStore } from "./stores/quoteStore";
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

function App() {
  // quoteWarning lives in the global quoteStore so that any fetch path
  // (Dashboard, Holdings, Statistics, manual refresh…) can update it.
  const setQuoteWarning = useQuoteStore((s) => s.setQuoteWarning);

  // Use antd's imperative notification API (available because this component
  // is rendered inside <AntdApp> in main.tsx).
  const { notification } = AntdApp.useApp();

  // pendingWarning is set by async callbacks (Tauri events, setInterval).
  // Calling notification.warning() directly from those async contexts can
  // silently fail in React 18 because the call happens outside the React
  // commit cycle. By routing through useState + useEffect we guarantee the
  // notification is triggered from inside the React lifecycle.
  const [pendingWarning, setPendingWarning] = useState<string | null>(null);

  useEffect(() => {
    if (!pendingWarning) return;
    setQuoteWarning(pendingWarning); // keep store in sync
    notification.warning({
      key: "quote-warning",
      message: "行情获取提示",
      description: pendingWarning,
      duration: 0,
      onClose: () => {
        setPendingWarning(null);
        setQuoteWarning(null);
      },
    });
  }, [pendingWarning, notification, setQuoteWarning]);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];
    // Async callbacks only set state; the effect above handles display.
    const show = (text: string) => setPendingWarning(text);

    // Path 1 (fast): the background startup refresh emits `quote-warning`
    // carrying the warning text directly in the payload via peek (so
    // LAST_QUOTE_WARNING is NOT consumed and remains available below).
    listen<string>("quote-warning", (event) => {
      if (event.payload) show(event.payload);
    }).then((fn) => {
      if (cancelled) fn();
      else unsubs.push(fn);
    });

    // Path 2 (reliable, page-agnostic): the background task emits
    // `quotes-refreshed` AFTER peeking the warning. LAST_QUOTE_WARNING is
    // still set at this point; consume it here. This listener is active on
    // every page, unlike startAutoRefresh which only runs on the Holdings page.
    listen<unknown>("quotes-refreshed", () => {
      invoke<string | null>("take_quote_warning")
        .then((w) => { if (w) show(w); })
        .catch(() => {});
    }).then((fn) => {
      if (cancelled) fn();
      else unsubs.push(fn);
    });

    // Path 3 (safety net): poll take_quote_warning every 2 s for up to 10 s.
    // Catches warnings if both events above were missed (e.g. listener
    // registration timing on slow machines, or the warning was set by the
    // Dashboard's own initial cache-miss fetch before any event fired).
    const MAX_POLLS = 5; // 5 × 2 s = 10 s
    let pollCount = 0;
    const pollId = setInterval(() => {
      if (cancelled || pollCount >= MAX_POLLS) {
        clearInterval(pollId);
        return;
      }
      pollCount++;
      invoke<string | null>("take_quote_warning")
        .then((w) => { if (w) show(w); })
        .catch(() => {});
    }, 2000);

    return () => {
      cancelled = true;
      clearInterval(pollId);
      unsubs.forEach((fn) => fn());
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
