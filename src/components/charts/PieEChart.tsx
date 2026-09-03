import { PieChart } from "echarts/charts";
import {
  LegendComponent,
  TitleComponent,
  TooltipComponent,
} from "echarts/components";
import EChartCore, { echarts, type EChartProps } from "./EChartCore";

echarts.use([PieChart, LegendComponent, TitleComponent, TooltipComponent]);

export default function PieEChart(props: EChartProps) {
  return <EChartCore {...props} />;
}
