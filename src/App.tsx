import { useEffect } from "react";
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
  const quoteWarning = useQuoteStore((s) => s.quoteWarning);
  const setQuoteWarning = useQuoteStore((s) => s.setQuoteWarning);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    // Path 1 (fast): the background startup refresh emits `quote-warning`
    // carrying the warning text directly in the payload via peek (so
    // LAST_QUOTE_WARNING is NOT consumed and remains available below).
    listen<string>("quote-warning", (event) => {
      if (event.payload) setQuoteWarning(event.payload);
    }).then((fn) => {
      if (cancelled) fn();
      else unsubs.push(fn);
    });

    // Path 2 (reliable, page-agnostic): the background task emits
    // `quotes-refreshed` AFTER setting and peeking the warning. By the time
    // this event fires LAST_QUOTE_WARNING is guaranteed to hold any Xueqiu
    // error. Calling take_quote_warning here consumes it — this listener is
    // active on every page, unlike startAutoRefresh which only runs on the
    // Holdings page. This is the most reliable delivery path.
    listen<unknown>("quotes-refreshed", () => {
      invoke<string | null>("take_quote_warning")
        .then((w) => { if (w) setQuoteWarning(w); })
        .catch(() => {});
    }).then((fn) => {
      if (cancelled) fn();
      else unsubs.push(fn);
    });

    // Path 3 (safety net): poll take_quote_warning every 2 s for up to 30 s.
    // Catches warnings if both events above were missed due to listener
    // registration timing on slow machines.
    const MAX_POLLS = 15; // 15 × 2 s = 30 s
    let pollCount = 0;
    const pollId = setInterval(() => {
      if (cancelled || pollCount >= MAX_POLLS) {
        clearInterval(pollId);
        return;
      }
      pollCount++;
      invoke<string | null>("take_quote_warning")
        .then((w) => { if (w) setQuoteWarning(w); })
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
      {quoteWarning && (
        <div
          style={{
            position: "fixed",
            top: 24,
            right: 24,
            zIndex: 9999,
            maxWidth: 400,
            // 224 px = sidebar width (200 px) + border/padding; 48 px = left+right margin
            width: "calc(100vw - 224px - 48px)",
          }}
        >
          <Alert
            message="行情获取提示"
            description={quoteWarning}
            type="warning"
            closable
            showIcon
            onClose={() => setQuoteWarning(null)}
          />
        </div>
      )}
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
