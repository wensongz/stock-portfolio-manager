import ReactECharts from "echarts-for-react";
import { Typography } from "antd";
import type { HoldingPerformance } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";

const { Text } = Typography;

interface Props {
  gainers: HoldingPerformance[];
  losers: HoldingPerformance[];
  height?: number;
  currency?: string;
}

const CURRENCY_SYMBOL: Record<string, string> = { USD: "$", CNY: "¥", HKD: "HK$" };

export default function RankingChart({ gainers, losers, height = 340, currency = "USD" }: Props) {
  const { profitColor, lossColor } = usePnlColor();
  const cSym = CURRENCY_SYMBOL[currency] ?? "";
  // ECharts horizontal bar charts render categories bottom-to-top,
  // so reverse the arrays to show highest pnl at the top.
  const topGainers = gainers.slice(0, 10).reverse();
  const topLosers = losers.slice(0, 10).reverse();

  const gainNames = topGainers.map((h) => `${h.symbol} ${h.name}`);
  const lossNames = topLosers.map((h) => `${h.symbol} ${h.name}`);

  const fmtPnl = (v: number) => {
    const abs = Math.abs(v);
    if (currency === "CNY") {
      if (abs >= 1e8) return `${(v / 1e8).toFixed(2)}亿`;
      if (abs >= 1e4) return `${(v / 1e4).toFixed(2)}万`;
    } else {
      if (abs >= 1e6) return `${(v / 1e6).toFixed(2)}M`;
      if (abs >= 1e3) return `${(v / 1e3).toFixed(2)}K`;
    }
    return v.toFixed(2);
  };

  // Format PnL with sign and currency symbol, e.g. "+¥1.25万" or "-$300.00"
  const fmtSignedPnl = (v: number) => {
    const sign = v >= 0 ? "+" : "-";
    const abs = Math.abs(v);
    let formatted: string;
    if (currency === "CNY") {
      if (abs >= 1e8) formatted = `${(abs / 1e8).toFixed(2)}亿`;
      else if (abs >= 1e4) formatted = `${(abs / 1e4).toFixed(2)}万`;
      else formatted = abs.toFixed(2);
    } else {
      if (abs >= 1e6) formatted = `${(abs / 1e6).toFixed(2)}M`;
      else if (abs >= 1e3) formatted = `${(abs / 1e3).toFixed(2)}K`;
      else formatted = abs.toFixed(2);
    }
    return `${sign}${cSym}${formatted}`;
  };

  const option = {
    tooltip: {
      trigger: "axis",
      axisPointer: { type: "shadow" },
      formatter: (params: { seriesName: string; name: string; value: number; dataIndex: number; seriesIndex: number }[]) => {
        const p = params[0];
        const list = p.seriesIndex === 0 ? topGainers : topLosers;
        const h = list[p.dataIndex];
        const rr = h?.return_rate ?? 0;
        return `${p.name}<br/>盈亏: <b>${fmtSignedPnl(p.value)}</b><br/>收益率: <b>${rr >= 0 ? "+" : ""}${rr.toFixed(2)}%</b>`;
      },
    },
    legend: { bottom: 0 },
    grid: [
      { left: "4%", right: "52%", top: "10%", bottom: "12%", containLabel: true },
      { left: "52%", right: "4%", top: "10%", bottom: "12%", containLabel: true },
    ],
    xAxis: [
      { gridIndex: 0, type: "value", axisLabel: { formatter: (v: number) => fmtPnl(v) } },
      { gridIndex: 1, type: "value", axisLabel: { formatter: (v: number) => fmtPnl(v) } },
    ],
    yAxis: [
      { gridIndex: 0, type: "category", data: gainNames, axisLabel: { fontSize: 11 } },
      { gridIndex: 1, type: "category", data: lossNames, axisLabel: { fontSize: 11 } },
    ],
    series: [
      {
        name: "盈亏",
        type: "bar",
        xAxisIndex: 0,
        yAxisIndex: 0,
        data: topGainers.map((h) => ({
          value: parseFloat(h.pnl.toFixed(2)),
          itemStyle: { color: profitColor },
        })),
        label: { show: true, position: "right", formatter: (p: { value: number }) => `${p.value >= 0 ? "+" : ""}${fmtPnl(p.value)}` },
      },
      {
        name: "盈亏 (亏损)",
        type: "bar",
        xAxisIndex: 1,
        yAxisIndex: 1,
        data: topLosers.map((h) => ({
          value: parseFloat(h.pnl.toFixed(2)),
          itemStyle: { color: lossColor },
        })),
        label: { show: true, position: "left", formatter: (p: { value: number }) => fmtPnl(p.value) },
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
