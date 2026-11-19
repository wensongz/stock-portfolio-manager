// @ts-nocheck -- Node runs the actual TSX components through a Bun SSR probe.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

test("dashboard and currency-prefix cards put signs before currency in rendered amounts", () => {
  const probe = String.raw`
    globalThis.localStorage = { getItem: () => null, setItem: () => {} };
    const React = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const { default: SummaryCards } = await import("./src/pages/Dashboard/SummaryCards.tsx");
    const { default: StatCard } = await import("./src/components/charts/StatCard.tsx");
    const render = (component, props) => renderToStaticMarkup(React.createElement(component, props)).replace(/<[^>]*>/g, "");
    const summary = { base_currency: "USD", total_market_value: 1000, total_cost: 900, total_pnl: 1697148.22, total_pnl_percent: 2.38, daily_pnl: -740948.79 };
    process.stdout.write(JSON.stringify({
      dashboard: render(SummaryCards, { summary, loading: false, error: null }),
      reversed: render(SummaryCards, { summary: { ...summary, total_pnl: -1697148.22, daily_pnl: 740948.79 }, loading: false, error: null }),
      cards: ["USD", "¥", "HK$"].map(prefix => render(StatCard, { title: "余额", value: "-1234.50", prefix })),
      positive: render(StatCard, { title: "余额", value: "+1234.50", prefix: "$" }),
      ordinary: render(StatCard, { title: "比例", value: "-1.50", suffix: "%" }),
    }));
  `;
  const result = JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
  assert.match(result.dashboard, /总盈亏\+\$1,697,148\.22/);
  assert.match(result.dashboard, /今日盈亏-\$740,948\.79/);
  assert.match(result.reversed, /总盈亏-\$1,697,148\.22/);
  assert.match(result.reversed, /今日盈亏\+\$740,948\.79/);
  assert.deepEqual(result.cards, ["余额-$1,234.50", "余额-¥1,234.50", "余额-HK$1,234.50"]);
  assert.equal(result.positive, "余额+$1,234.50");
  assert.equal(result.ordinary, "比例-1.50%");
});

test("statistics profit and loss tooltips format amounts in the selected currency", () => {
  const probe = String.raw`
    import { mock } from "bun:test";
    globalThis.localStorage = { getItem: () => null, setItem: () => {} };
    const { createElement } = await import("react");
    const { renderToStaticMarkup } = await import("react-dom/server");
    const currencies = ["USD", "CNY", "HKD"];
    const overview = {
      total_market_value: 10000000, total_cost: 10000000, total_pnl: 0, total_pnl_percent: 0,
      market_distribution: [], category_distribution: [], account_distribution: [],
      stock_distribution: [], holdings: [],
      top_gainers: [{ symbol: "DDOG", name: "Datadog, Inc.", pnl: 721850.46, pnl_percent: 1, market_value: 1000000 }],
      top_losers: [{ symbol: "BABA", name: "Alibaba", pnl: -6098950, pnl_percent: -1, market_value: 1000000 }],
    };
    const store = await import("./src/stores/statisticsStore");
    store.useStatisticsStore.setState({
      overviewByCurrency: Object.fromEntries(currencies.map(currency => [currency, overview])),
    });
    const readState = store.useStatisticsStore.getState;
    mock.module("./src/stores/statisticsStore", () => ({
      ...store, useStatisticsStore: () => readState(),
    }));
    const tooltips = [];
    mock.module("./src/components/charts/EChart", () => ({
      default: ({ option }) => {
        const [name] = option.xAxis.data;
        const [{ value }] = option.series[0].data;
        tooltips.push(option.tooltip.formatter([{ name, value, dataIndex: 0 }]));
        return null;
      },
    }));
    const { default: OverviewTab } = await import("./src/pages/Statistics/OverviewTab.tsx");
    for (const baseCurrency of currencies) renderToStaticMarkup(createElement(OverviewTab, { baseCurrency }));
    process.stdout.write(JSON.stringify(tooltips));
  `;
  const result = JSON.parse(execFileSync("bun", ["--eval", probe], { cwd: projectRoot, encoding: "utf8" }));
  assert.deepEqual(result, [
    "DDOG<br/>Datadog, Inc.<br/>+$721,850.46", "BABA<br/>Alibaba<br/>-$6,098,950.00",
    "DDOG<br/>Datadog, Inc.<br/>+¥721,850.46", "BABA<br/>Alibaba<br/>-¥6,098,950.00",
    "DDOG<br/>Datadog, Inc.<br/>+HK$721,850.46", "BABA<br/>Alibaba<br/>-HK$6,098,950.00",
  ]);
});
