import ReactECharts from "echarts-for-react";
import type { QuarterComparison } from "../../types";

interface Props {
  comparison: QuarterComparison;
  height?: number;
}

export default function ComparisonCharts({ comparison, height = 320 }: Props) {
  const { quarter1, quarter2, by_market } = comparison;

  // Bar chart: market value comparison
  const markets = by_market.map((m) => {
    const labels: Record<string, string> = { US: "🇺🇸 美股", CN: "🇨🇳 A股", HK: "🇭🇰 港股" };
    return labels[m.market] ?? m.market;
  });

  const barOption = {
    title: { text: "市场市值对比", left: "center", textStyle: { fontSize: 14 } },
    tooltip: { trigger: "axis", axisPointer: { type: "shadow" } },
    legend: { bottom: 0, data: [quarter1, quarter2] },
    grid: { left: "3%", right: "4%", bottom: "15%", containLabel: true },
    xAxis: { type: "category", data: markets },
    yAxis: { type: "value" },
    series: [
      {
        name: quarter1,
        type: "bar",
        data: by_market.map((m) => parseFloat(m.q1_value.toFixed(2))),
        itemStyle: { color: "#5470c6" },
        label: { show: false },
      },
      {
        name: quarter2,
        type: "bar",
        data: by_market.map((m) => parseFloat(m.q2_value.toFixed(2))),
        itemStyle: { color: "#91cc75" },
        label: { show: false },
      },
    ],
  };

  // Bar chart: PnL comparison
  const pnlOption = {
    title: { text: "市场盈亏对比", left: "center", textStyle: { fontSize: 14 } },
    tooltip: { trigger: "axis", axisPointer: { type: "shadow" } },
    legend: { bottom: 0, data: [quarter1, quarter2] },
    grid: { left: "3%", right: "4%", bottom: "15%", containLabel: true },
    xAxis: { type: "category", data: markets },
    yAxis: { type: "value" },
    series: [
      {
        name: quarter1,
        type: "bar",
        data: by_market.map((m) => parseFloat(m.q1_pnl.toFixed(2))),
        itemStyle: { color: "#5470c6" },
      },
      {
        name: quarter2,
        type: "bar",
        data: by_market.map((m) => parseFloat(m.q2_pnl.toFixed(2))),
        itemStyle: { color: "#91cc75" },
      },
    ],
  };

  return (
    <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-4">
      <div className="bg-white rounded p-2 border border-gray-100">
        <ReactECharts option={barOption} style={{ height, width: "100%" }} />
      </div>
      <div className="bg-white rounded p-2 border border-gray-100">
        <ReactECharts option={pnlOption} style={{ height, width: "100%" }} />
      </div>
    </div>
  );
}
