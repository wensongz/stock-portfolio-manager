import ReactECharts from "echarts-for-react";
import { Select, Space, Typography } from "antd";
import type { ReturnDataPoint } from "../../types";
import { BENCHMARK_SYMBOLS } from "../../stores/performanceStore";

const { Text } = Typography;

interface Props {
  returnSeries: ReturnDataPoint[];
  benchmarkSeries: Record<string, ReturnDataPoint[]>;
  selectedBenchmarks: string[];
  onBenchmarkChange: (symbols: string[]) => void;
  height?: number;
}

export default function ReturnChart({
  returnSeries,
  benchmarkSeries,
  selectedBenchmarks,
  onBenchmarkChange,
  height = 380,
}: Props) {
  const dates = returnSeries.map((d) => d.date);

  const series: object[] = [
    {
      name: "组合收益率",
      type: "line",
      data: returnSeries.map((d) => parseFloat(d.cumulative_return.toFixed(2))),
      smooth: true,
      symbol: "none",
      lineStyle: { width: 2 },
      areaStyle: { opacity: 0.06 },
    },
  ];

  for (const sym of selectedBenchmarks) {
    const bData = benchmarkSeries[sym];
    if (!bData || bData.length === 0) continue;
    const label = BENCHMARK_SYMBOLS.find((b) => b.value === sym)?.label ?? sym;
    series.push({
      name: label,
      type: "line",
      data: bData.map((d) => parseFloat(d.cumulative_return.toFixed(2))),
      smooth: true,
      symbol: "none",
      lineStyle: { width: 1.5, type: "dashed" },
    });
  }

  const option = {
    tooltip: {
      trigger: "axis",
      axisPointer: { type: "cross" },
      formatter: (params: { seriesName: string; value: number; axisValue: string }[]) => {
        if (!params.length) return "";
        const date = params[0].axisValue;
        const lines = params.map(
          (p) => `${p.seriesName}: <b>${p.value >= 0 ? "+" : ""}${p.value.toFixed(2)}%</b>`
        );
        return `${date}<br/>${lines.join("<br/>")}`;
      },
    },
    legend: { bottom: 0, left: "center" },
    grid: { left: "3%", right: "4%", bottom: "14%", containLabel: true },
    xAxis: {
      type: "category",
      data: dates,
      axisLabel: { rotate: 30, formatter: (v: string) => v.slice(5) },
    },
    yAxis: {
      type: "value",
      scale: true,
      axisLabel: { formatter: (v: number) => `${v.toFixed(1)}%` },
    },
    dataZoom: [
      { type: "inside", start: 0, end: 100 },
      { type: "slider", bottom: 30, start: 0, end: 100 },
    ],
    series,
  };

  return (
    <div>
      <div className="flex items-center justify-between mb-2">
        <Text strong>📈 累计收益率曲线</Text>
        <Space>
          <Text type="secondary" style={{ fontSize: 12 }}>
            叠加基准：
          </Text>
          <Select
            mode="multiple"
            style={{ minWidth: 200 }}
            size="small"
            placeholder="选择基准指数"
            value={selectedBenchmarks}
            onChange={onBenchmarkChange}
            options={BENCHMARK_SYMBOLS.map((b) => ({ label: b.label, value: b.value }))}
          />
        </Space>
      </div>
      {returnSeries.length === 0 ? (
        <div className="flex items-center justify-center" style={{ height }}>
          <Text type="secondary">暂无数据，正在自动计算历史持仓市值…</Text>
        </div>
      ) : (
        <ReactECharts
          option={option}
          style={{ height, width: "100%" }}
          opts={{ renderer: "canvas" }}
        />
      )}
    </div>
  );
}
