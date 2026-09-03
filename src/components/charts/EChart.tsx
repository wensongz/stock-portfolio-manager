import {
  BarChart,
  LineChart,
} from "echarts/charts";
import {
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  MarkAreaComponent,
  TitleComponent,
  TooltipComponent,
} from "echarts/components";
import EChartCore, { echarts, type EChartProps } from "./EChartCore";

echarts.use([
  BarChart,
  LineChart,
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  MarkAreaComponent,
  TitleComponent,
  TooltipComponent,
]);

export default function EChart(props: EChartProps) {
  return <EChartCore {...props} />;
}
