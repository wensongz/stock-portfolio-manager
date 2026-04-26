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

    // Fast path: the background startup refresh emits `quote-warning` with
    // the warning text directly in the payload.  Set the store immediately
    // without consuming the Rust-side value so the polling fallback below
    // can still pick it up if this event arrived before the listener was ready.
    listen<string>("quote-warning", (event) => {
      if (event.payload) {
        useQuoteStore.getState().setQuoteWarning(event.payload);
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unsubs.push(fn);
    });

    // Reliable fallback: poll take_quote_warning every 2 s for up to 30 s.
    // This catches warnings even if the quote-warning event was emitted before
    // the listener above was fully registered (a common race on slow machines).
    // The Rust background task uses peek_quote_warning (not take) so the
    // warning stays available here until one of these polls consumes it.
    const MAX_POLLS = 15; // 15 × 2 s = 30 s
    let pollCount = 0;
    const pollId = setInterval(() => {
      if (cancelled || pollCount >= MAX_POLLS) {
        clearInterval(pollId);
        return;
      }
      pollCount++;
      invoke<string | null>("take_quote_warning")
        .then((w) => { if (w) useQuoteStore.getState().setQuoteWarning(w); })
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
