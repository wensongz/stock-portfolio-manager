import EChart from "./EChart";
import type { DailyPortfolioValue } from "../../types";

interface LineChartProps {
  data: DailyPortfolioValue[];
  title?: string;
  height?: number;
  showMarkets?: boolean;
}

export default function LineChart({ data, title, height = 350, showMarkets = false }: LineChartProps) {
  const dates = data.map((d) => d.date);
  const totalValues = data.map((d) => parseFloat(d.total_value.toFixed(2)));
  const totalCosts = data.map((d) => parseFloat(d.total_cost.toFixed(2)));

  const series: object[] = [
    {
      name: "总市值",
      type: "line",
      data: totalValues,
      smooth: true,
      symbol: "none",
      lineStyle: { width: 2 },
      areaStyle: { opacity: 0.08 },
    },
    {
      name: "总成本",
      type: "line",
      data: totalCosts,
      smooth: true,
      symbol: "none",
      lineStyle: { width: 1.5, type: "dashed" },
    },
  ];

  if (showMarkets) {
    series.push(
      {
        name: "🇺🇸 美股",
        type: "line",
        data: data.map((d) => parseFloat(d.us_value.toFixed(2))),
        smooth: true,
        symbol: "none",
        lineStyle: { width: 1.5 },
      },
      {
        name: "🇨🇳 A股",
        type: "line",
        data: data.map((d) => parseFloat(d.cn_value.toFixed(2))),
        smooth: true,
        symbol: "none",
        lineStyle: { width: 1.5 },
      },
      {
        name: "🇭🇰 港股",
        type: "line",
        data: data.map((d) => parseFloat(d.hk_value.toFixed(2))),
        smooth: true,
        symbol: "none",
        lineStyle: { width: 1.5 },
      }
    );
  }

  const option = {
    title: title
      ? { text: title, left: "center", textStyle: { fontSize: 14, fontWeight: "bold" } }
      : undefined,
    tooltip: {
      trigger: "axis",
      axisPointer: { type: "cross" },
    },
    legend: { bottom: 0, left: "center" },
    grid: { left: "3%", right: "4%", bottom: "12%", containLabel: true },
    xAxis: {
      type: "category",
      data: dates,
      axisLabel: {
        rotate: 30,
        formatter: (v: string) => v.slice(5), // show MM-DD
      },
    },
    yAxis: { type: "value", scale: true },
    dataZoom: [
      { type: "inside", start: 0, end: 100 },
      { type: "slider", bottom: 30, start: 0, end: 100 },
    ],
    series,
  };

  return (
    <EChart
      option={option}
      style={{ height, width: "100%" }}
      opts={{ renderer: "canvas" }}
    />
  );
}
