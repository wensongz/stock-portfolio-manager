// @ts-nocheck -- Node runs the existing TSX components in Bun with chart props captured.
import test from "node:test";
import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const projectRoot = fileURLToPath(new URL("../../../", import.meta.url));

test("performance charts preserve valuation dates, benchmark gaps and period labels", () => {
  const output = execFileSync("bun", ["-e", String.raw`
    import { mock } from "bun:test";
    import { Window } from "happy-dom";
    import React from "react";
    import { renderToStaticMarkup } from "react-dom/server";
    const dom = new Window();
    Object.defineProperty(globalThis, "localStorage", { value: dom.localStorage });
    let option;
    mock.module("./src/components/charts/EChart", () => ({ default: props => { option = props.option; return null; } }));
    const { default: ReturnChart } = await import("./src/pages/Performance/ReturnChart.tsx");
    const { default: DrawdownChart } = await import("./src/pages/Performance/DrawdownChart.tsx");
    const { default: MonthlyReturnsTable } = await import("./src/pages/Performance/MonthlyReturnsTable.tsx");
    const h = React.createElement;
    const portfolio = [
      { date: "2026-11-13", cumulative_return: 1 },
      { date: "2026-11-16", cumulative_return: 2 },
      { date: "2026-11-17", cumulative_return: 3 },
    ];
    const benchmark = [
      { date: "2026-11-12", cumulative_return: 0 },
      { date: "2026-11-13", cumulative_return: 0.5 },
      { date: "2026-11-17", cumulative_return: 3 },
    ];
    renderToStaticMarkup(h(ReturnChart, { startDate: "2026-11-12", returnSeries: portfolio,
      benchmarkSeries: { "^GSPC": benchmark }, benchmarkErrors: {}, selectedBenchmarks: ["^GSPC"], onBenchmarkChange() {} }));
    const returns = { dates: option.xAxis.data, portfolio: option.series[0].data, benchmark: option.series[1].data };
    const analysis = { peak_date: "2026-11-12", trough_date: "2026-11-17", max_drawdown: -2,
      drawdown_duration: 5, recovery_date: null, recovery_duration: null,
      drawdown_series: [{ date: "2026-11-13", drawdown: -1 }, { date: "2026-11-17", drawdown: -2 }] };
    renderToStaticMarkup(h(DrawdownChart, { drawdown: analysis }));
    const drawdown = { dates: option.xAxis.data, values: option.series[0].data, markup: option.series[0].markArea.data };
    const flat = renderToStaticMarkup(h(DrawdownChart, { drawdown: { ...analysis, max_drawdown: 0 } }));
    const month = renderToStaticMarkup(h(MonthlyReturnsTable, { data: [{ year: 2026, month: 11, return_rate: 2 }] }));
    console.log(JSON.stringify({ returns, drawdown, flat, month }));
  `], { cwd: projectRoot, encoding: "utf8" });
  const { returns, drawdown, flat, month } = JSON.parse(output);
  assert.deepEqual(returns, {
    dates: ["2026-11-12", "2026-11-13", "2026-11-16", "2026-11-17"],
    portfolio: [0, 1, 2, 3],
    benchmark: [0, 0.5, null, 3],
  });
  assert.deepEqual(drawdown, {
    dates: ["2026-11-12", "2026-11-13", "2026-11-17"],
    values: [0, -1, -2],
    markup: [[{ xAxis: "2026-11-12" }, { xAxis: "2026-11-17" }]],
  });
  assert.doesNotMatch(flat, /未恢复/);
  assert.match(month, /区间合计/);
  assert.doesNotMatch(month, /全年/);
});
