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
