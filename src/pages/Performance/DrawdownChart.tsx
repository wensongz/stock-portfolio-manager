import ReactECharts from "echarts-for-react";
import { Typography, Descriptions } from "antd";
import type { DrawdownAnalysis } from "../../types";

const { Text } = Typography;

interface Props {
  drawdown: DrawdownAnalysis | null;
  height?: number;
}

export default function DrawdownChart({ drawdown, height = 280 }: Props) {
  if (!drawdown || drawdown.drawdown_series.length === 0) {
    return (
      <div className="flex items-center justify-center" style={{ height }}>
        <Text type="secondary">暂无数据</Text>
      </div>
    );
  }

  const dates = drawdown.drawdown_series.map((d) => d.date);
  const values = drawdown.drawdown_series.map((d) => parseFloat(d.drawdown.toFixed(2)));

  const option = {
    tooltip: {
      trigger: "axis",
      formatter: (params: { axisValue: string; value: number }[]) => {
        if (!params.length) return "";
        const p = params[0];
        return `${p.axisValue}<br/>回撤: <b>${p.value.toFixed(2)}%</b>`;
      },
    },
    grid: { left: "3%", right: "4%", bottom: "10%", containLabel: true },
    xAxis: {
      type: "category",
      data: dates,
      axisLabel: { rotate: 30, formatter: (v: string) => v.slice(5) },
    },
    yAxis: {
      type: "value",
      scale: true,
      max: 0,
      axisLabel: { formatter: (v: number) => `${v.toFixed(1)}%` },
    },
    dataZoom: [{ type: "inside" }],
    series: [
      {
        name: "回撤",
        type: "line",
        data: values,
        smooth: true,
        symbol: "none",
        lineStyle: { width: 1, color: "#cf1322" },
        areaStyle: { color: "#cf1322", opacity: 0.3 },
        markArea: {
          data: [
            [
              { xAxis: drawdown.peak_date },
              { xAxis: drawdown.trough_date },
            ],
          ],
          itemStyle: { color: "rgba(207,19,34,0.08)" },
        },
      },
    ],
  };

  return (
    <div>
      <Text strong>📉 回撤分析</Text>
      <ReactECharts
        option={option}
        style={{ height, width: "100%" }}
        opts={{ renderer: "canvas" }}
      />
      <Descriptions size="small" column={3} className="mt-2">
        <Descriptions.Item label="最大回撤">
          <Text type="danger">{drawdown.max_drawdown.toFixed(2)}%</Text>
        </Descriptions.Item>
        <Descriptions.Item label="峰值日期">{drawdown.peak_date}</Descriptions.Item>
        <Descriptions.Item label="谷值日期">{drawdown.trough_date}</Descriptions.Item>
        <Descriptions.Item label="回撤持续">
          {drawdown.drawdown_duration} 天
        </Descriptions.Item>
        <Descriptions.Item label="恢复日期">
          {drawdown.recovery_date ?? "未恢复"}
        </Descriptions.Item>
        <Descriptions.Item label="恢复持续">
          {drawdown.recovery_duration != null ? `${drawdown.recovery_duration} 天` : "-"}
        </Descriptions.Item>
      </Descriptions>
    </div>
  );
}
