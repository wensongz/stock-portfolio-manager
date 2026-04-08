import ReactECharts from "echarts-for-react";
import { Typography } from "antd";
import type { HoldingPerformance } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";

const { Text } = Typography;

interface Props {
  gainers: HoldingPerformance[];
  losers: HoldingPerformance[];
  height?: number;
}

export default function RankingChart({ gainers, losers, height = 340 }: Props) {
  const { profitColor, lossColor } = usePnlColor();
  // ECharts horizontal bar charts render categories bottom-to-top,
  // so reverse the arrays to show highest return rate at the top.
  const topGainers = gainers.slice(0, 10).reverse();
  const topLosers = losers.slice(0, 10).reverse();

  const gainNames = topGainers.map((h) => `${h.symbol} ${h.name}`);
  const lossNames = topLosers.map((h) => `${h.symbol} ${h.name}`);

  const option = {
    tooltip: {
      trigger: "axis",
      axisPointer: { type: "shadow" },
      formatter: (params: { seriesName: string; name: string; value: number }[]) => {
        const p = params[0];
        return `${p.name}<br/>${p.seriesName}: <b>${p.value >= 0 ? "+" : ""}${p.value.toFixed(2)}%</b>`;
      },
    },
    legend: { bottom: 0 },
    grid: [
      { left: "4%", right: "52%", top: "10%", bottom: "12%", containLabel: true },
      { left: "52%", right: "4%", top: "10%", bottom: "12%", containLabel: true },
    ],
    xAxis: [
      { gridIndex: 0, type: "value", axisLabel: { formatter: (v: number) => `${v}%` } },
      { gridIndex: 1, type: "value", axisLabel: { formatter: (v: number) => `${v}%` } },
    ],
    yAxis: [
      { gridIndex: 0, type: "category", data: gainNames, axisLabel: { fontSize: 11 } },
      { gridIndex: 1, type: "category", data: lossNames, axisLabel: { fontSize: 11 } },
    ],
    series: [
      {
        name: "收益率",
        type: "bar",
        xAxisIndex: 0,
        yAxisIndex: 0,
        data: topGainers.map((h) => ({
          value: parseFloat(h.return_rate.toFixed(2)),
          itemStyle: { color: profitColor },
        })),
        label: { show: true, position: "right", formatter: (p: { value: number }) => `${p.value >= 0 ? "+" : ""}${p.value}%` },
      },
      {
        name: "收益率 (亏损)",
        type: "bar",
        xAxisIndex: 1,
        yAxisIndex: 1,
        data: topLosers.map((h) => ({
          value: parseFloat(h.return_rate.toFixed(2)),
          itemStyle: { color: lossColor },
        })),
        label: { show: true, position: "left", formatter: (p: { value: number }) => `${p.value.toFixed(1)}%` },
      },
    ],
  };

  if (gainers.length === 0 && losers.length === 0) {
    return (
      <div className="flex items-center justify-center" style={{ height }}>
        <Text type="secondary">暂无个股表现数据</Text>
      </div>
    );
  }

  return (
    <div>
      <Text strong>🏆 个股表现排行</Text>
      <div style={{ display: "flex", gap: 4, marginBottom: 4 }}>
        <Text style={{ flex: 1, textAlign: "center", fontSize: 12, color: profitColor }}>
          最佳表现
        </Text>
        <Text style={{ flex: 1, textAlign: "center", fontSize: 12, color: lossColor }}>
          最差表现
        </Text>
      </div>
      <ReactECharts
        option={option}
        style={{ height, width: "100%" }}
        opts={{ renderer: "canvas" }}
      />
    </div>
  );
}
