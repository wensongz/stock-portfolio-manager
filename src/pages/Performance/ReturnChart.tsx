import EChart from "../../components/charts/EChart";
import { Select, Space, Tooltip, Typography } from "antd";
import type { ReturnDataPoint } from "../../types";
import { BENCHMARK_SYMBOLS } from "../../stores/performanceStore";

const { Text } = Typography;

interface Props {
  startDate?: string;
  returnSeries: ReturnDataPoint[];
  benchmarkSeries: Record<string, ReturnDataPoint[]>;
  benchmarkErrors?: Record<string, string>;
  selectedBenchmarks: string[];
  onBenchmarkChange: (symbols: string[]) => void;
  height?: number;
}

export default function ReturnChart({
  startDate,
  returnSeries,
  benchmarkSeries,
  benchmarkErrors = {},
  selectedBenchmarks,
  onBenchmarkChange,
  height = 380,
}: Props) {
  const portfolioByDate = new Map(returnSeries.map((point) => [point.date, point.cumulative_return]));
  if (startDate && returnSeries.length && startDate < returnSeries[0].date) {
    portfolioByDate.set(startDate, 0);
  }
  const benchmarks = BENCHMARK_SYMBOLS.filter(({ value }) => selectedBenchmarks.includes(value));
  const dates = [...new Set([
    ...portfolioByDate.keys(),
    ...benchmarks.flatMap(({ value }) => benchmarkSeries[value]?.map((point) => point.date) ?? []),
  ])].sort();

  const series: object[] = [
    {
      name: "组合收益率",
      type: "line",
      data: dates.map((date) => portfolioByDate.get(date) ?? null),
      connectNulls: true,
      symbol: "none",
      lineStyle: { width: 2 },
      areaStyle: { opacity: 0.06 },
    },
  ];

  for (const { label, value } of benchmarks) {
    const bData = benchmarkSeries[value];
    if (!bData || bData.length === 0) continue;
    const returnsByDate = new Map(bData.map((point) => [point.date, point.cumulative_return]));
    series.push({
      name: label,
      type: "line",
      data: dates.map((date) => returnsByDate.get(date) ?? null),
      connectNulls: true,
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
        const lines = params.filter((p) => Number.isFinite(p.value)).map(
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
          <Tooltip title="基准采用当地市场的指数收盘价收益。组合收益包含股息，并按组合的绩效计价币种计算。">
            <Text type="secondary" style={{ fontSize: 12 }}>叠加基准</Text>
          </Tooltip>
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
      {benchmarks.map(({ label, value }) => benchmarkErrors[value] ? (
        <div key={value}>
          <Tooltip title={benchmarkErrors[value]}>
            <Text type="warning">{label} 行情加载失败</Text>
          </Tooltip>
        </div>
      ) : benchmarkSeries[value]?.length === 0 ? (
        <div key={value}><Text type="secondary">{label} 暂无可比行情</Text></div>
      ) : null)}
      {returnSeries.length === 0 ? (
        <div className="flex items-center justify-center" style={{ height }}>
          <Text type="secondary">暂无数据，正在自动计算历史持仓市值…</Text>
        </div>
      ) : (
        <EChart
          option={option}
          style={{ height, width: "100%" }}
          opts={{ renderer: "canvas" }}
        />
      )}
    </div>
  );
}
