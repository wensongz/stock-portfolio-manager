import { useState, useEffect } from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Alert } from "antd";
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

  // pendingWarning drives the JSX-based Alert banner below.
  // Async callbacks (Tauri events, setInterval) set this via setPendingWarning.
  // Rendering the Alert in JSX is the most reliable approach in Tauri's webview
  // because it avoids antd notification portals, which can silently fail to
  // mount at startup before the portal root is ready.
  const [pendingWarning, setPendingWarning] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];
    const show = (text: string) => {
      setPendingWarning(text);
      setQuoteWarning(text);
    };

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
    // Catches warnings set by any fetch that ran before events could fire,
    // e.g. the Dashboard's own initial cache-miss fetch at startup.
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
  }, [setQuoteWarning]);

  const handleWarningClose = () => {
    setPendingWarning(null);
    setQuoteWarning(null);
  };

  return (
    <>
      {/* Xueqiu warning banner — rendered in the React tree (not a portal) so
          it is guaranteed to display in Tauri's webview regardless of startup
          timing or portal availability. */}
      {pendingWarning && (
        <div style={{
          position: "fixed",
          top: 16,
          right: 16,
          zIndex: 9999,
          maxWidth: 400,
          width: "calc(100vw - 32px)",
          boxShadow: "0 4px 12px rgba(0,0,0,0.15)",
          borderRadius: 8,
        }}>
          <Alert
            type="warning"
            message="行情获取提示"
            description={pendingWarning}
            showIcon
            closable
            onClose={handleWarningClose}
          />
        </div>
      )}
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
    </>
  );
}

export default App;
