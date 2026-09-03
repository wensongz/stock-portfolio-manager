import ReactEChartsCore from "echarts-for-react/esm/core.js";
import type { EChartsReactProps } from "echarts-for-react";
import * as echarts from "echarts/core";
import { LabelLayout } from "echarts/features";
import { CanvasRenderer } from "echarts/renderers";

echarts.use([LabelLayout, CanvasRenderer]);

export { echarts };
export type EChartProps = Omit<EChartsReactProps, "echarts">;

export default function EChartCore(props: EChartProps) {
  return <ReactEChartsCore echarts={echarts} {...props} />;
}
