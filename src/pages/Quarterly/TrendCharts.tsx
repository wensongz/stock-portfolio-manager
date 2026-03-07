import ReactECharts from "echarts-for-react";
import { Card, Col, Row } from "antd";
import type { QuarterlyTrends } from "../../types";

interface Props {
  trends: QuarterlyTrends;
  height?: number;
}

export default function TrendCharts({ trends, height = 300 }: Props) {
  const { quarters, total_values, total_costs, total_pnls, market_values, category_values, holding_counts } =
    trends;

  // A) Total value + cost trend
  const valueOption = {
    title: { text: "A) 总市值趋势", left: "center", textStyle: { fontSize: 13 } },
    tooltip: { trigger: "axis" },
    legend: { bottom: 0, data: ["总市值", "总成本"] },
    grid: { left: "3%", right: "4%", bottom: "15%", containLabel: true },
    xAxis: { type: "category", data: quarters },
    yAxis: { type: "value", scale: true },
    series: [
      {
        name: "总市值",
        type: "bar",
        data: total_values.map((v) => parseFloat(v.toFixed(2))),
        itemStyle: { color: "#5470c6" },
      },
      {
        name: "总成本",
        type: "line",
        data: total_costs.map((v) => parseFloat(v.toFixed(2))),
        smooth: true,
        symbol: "circle",
        lineStyle: { width: 2, type: "dashed" },
        itemStyle: { color: "#fc8452" },
      },
    ],
  };

  // B) Market value stacked area
  const marketOption = {
    title: { text: "B) 各市场市值趋势", left: "center", textStyle: { fontSize: 13 } },
    tooltip: { trigger: "axis" },
    legend: { bottom: 0, data: ["🇺🇸 美股", "🇨🇳 A股", "🇭🇰 港股"] },
    grid: { left: "3%", right: "4%", bottom: "15%", containLabel: true },
    xAxis: { type: "category", data: quarters },
    yAxis: { type: "value" },
    series: [
      {
        name: "🇺🇸 美股",
        type: "line",
        stack: "market",
        areaStyle: { opacity: 0.5 },
        data: (market_values["US"] ?? []).map((v) => parseFloat(v.toFixed(2))),
        smooth: true,
        symbol: "none",
        itemStyle: { color: "#5470c6" },
      },
      {
        name: "🇨🇳 A股",
        type: "line",
        stack: "market",
        areaStyle: { opacity: 0.5 },
        data: (market_values["CN"] ?? []).map((v) => parseFloat(v.toFixed(2))),
        smooth: true,
        symbol: "none",
        itemStyle: { color: "#ee6666" },
      },
      {
        name: "🇭🇰 港股",
        type: "line",
        stack: "market",
        areaStyle: { opacity: 0.5 },
        data: (market_values["HK"] ?? []).map((v) => parseFloat(v.toFixed(2))),
        smooth: true,
        symbol: "none",
        itemStyle: { color: "#91cc75" },
      },
    ],
  };

  // C) Category values stacked bar
  const categoryNames = Object.keys(category_values);
  const categoryOption = {
    title: { text: "C) 各类别占比趋势", left: "center", textStyle: { fontSize: 13 } },
    tooltip: { trigger: "axis", axisPointer: { type: "shadow" } },
    legend: { bottom: 0, data: categoryNames },
    grid: { left: "3%", right: "4%", bottom: "15%", containLabel: true },
    xAxis: { type: "category", data: quarters },
    yAxis: { type: "value" },
    series: categoryNames.map((cat) => ({
      name: cat,
      type: "bar",
      stack: "category",
      data: (category_values[cat] ?? []).map((v) => parseFloat(v.toFixed(2))),
    })),
  };

  // D) PnL trend
  const pnlOption = {
    title: { text: "D) 盈亏趋势", left: "center", textStyle: { fontSize: 13 } },
    tooltip: { trigger: "axis" },
    legend: { bottom: 0, data: ["季度盈亏"] },
    grid: { left: "3%", right: "4%", bottom: "15%", containLabel: true },
    xAxis: { type: "category", data: quarters },
    yAxis: { type: "value" },
    series: [
      {
        name: "季度盈亏",
        type: "bar",
        data: total_pnls.map((v) => ({
          value: parseFloat(v.toFixed(2)),
          itemStyle: { color: v >= 0 ? "#91cc75" : "#ee6666" },
        })),
      },
    ],
  };

  // E) Holding count trend
  const countOption = {
    title: { text: "E) 持仓数量趋势", left: "center", textStyle: { fontSize: 13 } },
    tooltip: { trigger: "axis" },
    grid: { left: "3%", right: "4%", bottom: "15%", containLabel: true },
    xAxis: { type: "category", data: quarters },
    yAxis: { type: "value", minInterval: 1 },
    series: [
      {
        name: "持仓数",
        type: "line",
        data: holding_counts,
        smooth: true,
        symbol: "circle",
        lineStyle: { width: 2 },
        itemStyle: { color: "#fac858" },
        label: { show: true, position: "top" },
      },
    ],
  };

  return (
    <Row gutter={[16, 16]}>
      <Col xs={24} lg={12}>
        <Card size="small">
          <ReactECharts option={valueOption} style={{ height, width: "100%" }} />
        </Card>
      </Col>
      <Col xs={24} lg={12}>
        <Card size="small">
          <ReactECharts option={marketOption} style={{ height, width: "100%" }} />
        </Card>
      </Col>
      <Col xs={24} lg={12}>
        <Card size="small">
          <ReactECharts option={categoryOption} style={{ height, width: "100%" }} />
        </Card>
      </Col>
      <Col xs={24} lg={12}>
        <Card size="small">
          <ReactECharts option={pnlOption} style={{ height, width: "100%" }} />
        </Card>
      </Col>
      <Col xs={24} lg={12}>
        <Card size="small">
          <ReactECharts option={countOption} style={{ height, width: "100%" }} />
        </Card>
      </Col>
    </Row>
  );
}
