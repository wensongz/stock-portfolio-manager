import ReactEChartsCore from "echarts-for-react/esm/core.js";
import type { EChartsReactProps } from "echarts-for-react";
import * as echarts from "echarts/core";
import {
  BarChart as EChartsBarChart,
  LineChart as EChartsLineChart,
  PieChart as EChartsPieChart,
} from "echarts/charts";
import {
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  MarkAreaComponent,
  TitleComponent,
  TooltipComponent,
} from "echarts/components";
import { LabelLayout } from "echarts/features";
import { CanvasRenderer } from "echarts/renderers";

echarts.use([
  EChartsBarChart,
  EChartsLineChart,
  EChartsPieChart,
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  MarkAreaComponent,
  LabelLayout,
  TitleComponent,
  TooltipComponent,
  CanvasRenderer,
]);

type Props = Omit<EChartsReactProps, "echarts">;

export default function EChart(props: Props) {
  return <ReactEChartsCore echarts={echarts} {...props} />;
}
