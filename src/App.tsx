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
  // quoteWarning in the global store is the single source of truth for the
  // Xueqiu warning banner. All delivery paths write to it; the JSX below
  // reads from it. This avoids a split between local pendingWarning state
  // and the store copy that caused warnings set by fetchHoldingQuotes /
  // fetchQuotes to never reach the Alert.
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
    // `quotes-refreshed` AFTER peeking the warning. LAST_QUOTE_WARNING is
    // still set at this point; consume it here. This listener is active on
    // every page, unlike startAutoRefresh which only runs on the Holdings page.
    listen<unknown>("quotes-refreshed", () => {
      invoke<string | null>("take_quote_warning")
        .then((w) => { if (w) setQuoteWarning(w); })
        .catch(() => {});
    }).then((fn) => {
      if (cancelled) fn();
      else unsubs.push(fn);
    });

    // NOTE: A periodic polling fallback was deliberately removed here.
    // It raced with fetchHoldingQuotes's own take_quote_warning() call:
    // the poll could consume the warning first, then fetchHoldingQuotes
    // would receive null and write quoteWarning: null — clearing the alert.
    // PATH 1 (quote-warning event) and PATH 2 (quotes-refreshed) are
    // sufficient for startup; manual refreshes use their own direct call.

    return () => {
      cancelled = true;
      unsubs.forEach((fn) => fn());
    };
  }, [setQuoteWarning]);

  return (
    <>
      {/* Xueqiu warning banner — rendered in the React tree (not a portal) so
          it is guaranteed to display in Tauri's webview regardless of startup
          timing. Driven by quoteStore.quoteWarning, the single source of truth
          written by fetchHoldingQuotes, fetchQuotes, and the event/poll paths. */}
      {quoteWarning && (
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
            description={quoteWarning}
            showIcon
            closable
            onClose={() => setQuoteWarning(null)}
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
