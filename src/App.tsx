import { useEffect, useState } from "react";
import { Routes, Route, Navigate } from "react-router-dom";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { Alert } from "antd";
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
  // quoteWarning drives a React-rendered Alert banner.
  // Using React state (instead of calling notification APIs from async callbacks)
  // guarantees the warning is visible in Tauri's WebKit webview: the Alert is
  // part of the normal React component tree and always renders when state is set.
  const [quoteWarning, setQuoteWarning] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const unsubs: Array<() => void> = [];

    // Pull any pending warning from the Rust backend and set React state.
    const checkWarning = async () => {
      try {
        const warning = await invoke<string | null>("take_quote_warning");
        if (warning) setQuoteWarning(warning);
      } catch {
        // ignore
      }
    };

    // Immediate check (catches warnings set before listeners are registered).
    checkWarning();

    // Polling fallback: check every 2 s for the first 30 s after startup.
    // The background refresh task runs ~2 s after launch, so this catches it
    // even if the Tauri event is delivered before the listener is registered.
    let pollCount = 0;
    const MAX_POLLS = 15; // 15 × 2 s = 30 s
    const pollInterval = setInterval(() => {
      if (cancelled || pollCount >= MAX_POLLS) {
        clearInterval(pollInterval);
        return;
      }
      pollCount++;
      checkWarning();
    }, 2000);

    // Primary path: Tauri emits `quote-warning` when the background refresh
    // encounters a Xueqiu error. Consume the stored warning so the polling
    // fallback does not duplicate it.
    listen<string>("quote-warning", (event) => {
      if (event.payload) {
        setQuoteWarning(event.payload);
        invoke("take_quote_warning").catch(() => {});
      }
    }).then((fn) => {
      if (cancelled) fn();
      else unsubs.push(fn);
    });

    // Secondary path: after the backend signals quotes are refreshed, poll
    // for any remaining (un-consumed) warning.
    listen("quotes-refreshed", () => checkWarning()).then((fn) => {
      if (cancelled) fn();
      else unsubs.push(fn);
    });

    return () => {
      cancelled = true;
      clearInterval(pollInterval);
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
