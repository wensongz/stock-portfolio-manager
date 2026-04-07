import ReactECharts from "echarts-for-react";
import { Tabs, Typography } from "antd";
import type { ReturnAttribution, AttributionItem } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";

const { Text } = Typography;

interface Props {
  attribution: ReturnAttribution | null;
  height?: number;
  currency?: string;
}

function makeWaterfallOption(items: AttributionItem[], title: string, colorFn: (v: number) => string, currency: string) {
  const sorted = [...items].sort((a, b) => b.pnl - a.pnl);
  const names = sorted.map((i) => i.name);
  const values = sorted.map((i) => parseFloat(i.pnl.toFixed(2)));
  const colors = values.map((v) => colorFn(v));

  return {
    title: { text: title, textStyle: { fontSize: 13 } },
    tooltip: {
      trigger: "axis",
      formatter: (params: { name: string; value: number; dataIndex: number }[]) => {
        const p = params[0];
        const item = sorted[p.dataIndex];
        return (
          `${p.name}<br/>` +
          `盈亏: <b>${p.value >= 0 ? "+" : ""}${p.value.toFixed(2)} ${currency}</b><br/>` +
          `贡献: ${item.contribution_percent.toFixed(1)}%`
        );
      },
    },
    grid: { left: "3%", right: "4%", bottom: "3%", containLabel: true },
    xAxis: { type: "category", data: names, axisLabel: { interval: 0, rotate: 30 } },
    yAxis: { type: "value", axisLabel: { formatter: (v: number) => `${v >= 0 ? "+" : ""}${v.toFixed(0)}` } },
    series: [
      {
        type: "bar",
        data: values.map((v, i) => ({ value: v, itemStyle: { color: colors[i] } })),
      },
    ],
  };
}

export default function AttributionChart({ attribution, height = 300, currency = "CNY" }: Props) {
  const { pnlColorDark } = usePnlColor();
  if (!attribution) {
    return (
      <div className="flex items-center justify-center" style={{ height }}>
        <Text type="secondary">暂无数据</Text>
      </div>
    );
  }

  const tabs = [
    {
      key: "market",
      label: "按市场",
      children: (
        <ReactECharts
          option={makeWaterfallOption(attribution.by_market, "市场收益贡献", pnlColorDark, currency)}
          style={{ height, width: "100%" }}
          opts={{ renderer: "canvas" }}
        />
      ),
    },
    {
      key: "category",
      label: "按类别",
      children: (
        <ReactECharts
          option={makeWaterfallOption(attribution.by_category, "类别收益贡献", pnlColorDark, currency)}
          style={{ height, width: "100%" }}
          opts={{ renderer: "canvas" }}
        />
      ),
    },
    {
      key: "holding",
      label: "按个股",
      children: (
        <ReactECharts
          option={makeWaterfallOption(attribution.by_holding.slice(0, 20), "个股收益贡献 (Top 20)", pnlColorDark, currency)}
          style={{ height, width: "100%" }}
          opts={{ renderer: "canvas" }}
        />
      ),
    },
  ];

  return (
    <div>
      <Text strong>🧩 收益贡献分解</Text>
      <Tabs items={tabs} size="small" className="mt-1" />
    </div>
  );
}
