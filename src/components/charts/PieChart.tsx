import ReactECharts from "echarts-for-react";
import type { PieSlice } from "../../types";

interface PieChartProps {
  data: PieSlice[];
  title?: string;
  centerText?: string;
  height?: number;
  currencyCode?: string;
}

const DEFAULT_COLORS = [
  "#5470c6", "#91cc75", "#fac858", "#ee6666", "#73c0de",
  "#3ba272", "#fc8452", "#9a60b4", "#ea7ccc", "#48b8d0",
];

export default function PieChart({ data, title, centerText, height = 300, currencyCode = "USD" }: PieChartProps) {
  const seriesData = data.map((item, i) => ({
    name: item.name,
    value: item.value,
    itemStyle: item.color
      ? { color: item.color }
      : { color: DEFAULT_COLORS[i % DEFAULT_COLORS.length] },
  }));

  const option = {
    title: title
      ? { text: title, left: "center", textStyle: { fontSize: 14, fontWeight: "bold" } }
      : undefined,
    tooltip: {
      trigger: "item",
      formatter: (params: { name: string; value: number; percent: number }) =>
        `${params.name}<br/>${currencyCode} ${params.value.toFixed(2)} (${params.percent}%)`,
    },
    legend: {
      type: "scroll",
      orient: "horizontal",
      bottom: 0,
      left: "center",
    },
    series: [
      {
        type: "pie",
        radius: ["45%", "70%"],
        center: ["50%", "45%"],
        avoidLabelOverlap: true,
        label: centerText
          ? {
              show: true,
              position: "center",
              formatter: () => centerText,
              fontSize: 12,
              color: "#666",
            }
          : { show: false },
        emphasis: {
          label: {
            show: true,
            fontSize: 13,
            fontWeight: "bold",
          },
          scale: true,
        },
        data: seriesData,
      },
    ],
  };

  return (
    <ReactECharts
      option={option}
      style={{ height, width: "100%" }}
      opts={{ renderer: "canvas" }}
    />
  );
}
