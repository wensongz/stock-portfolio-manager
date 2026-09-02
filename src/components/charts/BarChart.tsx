import EChart from "./EChart";
import { usePnlColor } from "../../hooks/usePnlColor";

interface BarItem {
  name: string;
  value: number;
}

interface BarChartProps {
  data: BarItem[];
  title?: string;
  height?: number;
  horizontal?: boolean;
  colorByValue?: boolean;
}

export default function BarChart({
  data,
  title,
  height = 300,
  horizontal = false,
  colorByValue = false,
}: BarChartProps) {
  const { pnlColor } = usePnlColor();
  const names = data.map((d) => d.name);
  const values = data.map((d) => ({
    value: d.value,
    itemStyle: colorByValue
      ? { color: pnlColor(d.value) }
      : {},
  }));

  const option = {
    title: title
      ? { text: title, left: "center", textStyle: { fontSize: 14, fontWeight: "bold" } }
      : undefined,
    tooltip: {
      trigger: "axis",
      formatter: (params: { name: string; value: number }[]) => {
        const p = params[0];
        return `${p.name}<br/>${p.value >= 0 ? "+" : ""}${p.value.toFixed(2)}`;
      },
    },
    grid: { left: "3%", right: "4%", bottom: "10%", containLabel: true },
    ...(horizontal
      ? {
          xAxis: { type: "value" },
          yAxis: { type: "category", data: names },
        }
      : {
          xAxis: {
            type: "category",
            data: names,
            axisLabel: { rotate: names.length > 5 ? 30 : 0, interval: 0 },
          },
          yAxis: { type: "value" },
        }),
    series: [
      {
        type: "bar",
        data: values,
        barMaxWidth: 60,
        label: {
          show: true,
          position: horizontal ? "right" : "top",
          formatter: (params: { value: number }) =>
            `${params.value >= 0 ? "+" : ""}${params.value.toFixed(2)}`,
          fontSize: 11,
        },
      },
    ],
  };

  return (
    <EChart
      option={option}
      style={{ height, width: "100%" }}
      opts={{ renderer: "canvas" }}
    />
  );
}
