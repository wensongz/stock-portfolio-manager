import ReactECharts from "echarts-for-react";
import { Alert, Card, Empty, Space, Tag, Typography, theme } from "antd";
import type { StockReviewReport } from "../../types";
import { buildStockReviewCurveSeries, getStockActionTypeDisplay } from "./stockReviewViewModel";

const { Text } = Typography;

export default function PortfolioComparisonChart({
  report,
  onOpenCampaign,
}: {
  report: StockReviewReport;
  onOpenCampaign: (campaignId: string) => void;
}) {
  const { token } = theme.useToken();
  const enabled = {
    actual: report.data_quality.actual_result_availability.status !== "unavailable",
    shadow: report.data_quality.shadow_value_add_availability.status !== "unavailable",
    benchmark: report.summary.result_quality.benchmark_return != null,
  };
  const curveSeries = buildStockReviewCurveSeries(report.curves, enabled);
  const campaignByAction = new Map(
    report.campaigns.flatMap((campaign) =>
      campaign.action_ids.map((actionId) => [actionId, campaign.campaign_id] as const),
    ),
  );
  const actualByDate = new Map(
    report.curves.map((point) => [point.date, point.portfolio_return] as const),
  );
  const actionMarkers = report.actions.flatMap((action) => {
    const campaignId = campaignByAction.get(action.action_id);
    const date = action.traded_at.slice(0, 10);
    const value = actualByDate.get(date);
    if (!campaignId || value == null) return [];
    return [{
      name: `${action.symbol} ${getStockActionTypeDisplay(action.action_type)}`,
      value: [date, value],
      symbol: action.action_type === "open" || action.action_type === "add" ? "triangle" : "diamond",
      symbolRotate: action.action_type === "reduce" || action.action_type === "close" ? 180 : 0,
      symbolSize: 11,
      campaignId,
      actionId: action.action_id,
      itemStyle: { color: action.action_type === "open" || action.action_type === "add" ? token.colorSuccess : token.colorWarning },
    }];
  });
  const absentReasons = [
    !enabled.shadow ? report.data_quality.shadow_value_add_availability.note ?? "影子组合不可用" : null,
    !enabled.benchmark ? report.summary.result_quality.availability.note ?? "市场基准不可用" : null,
  ].filter((reason): reason is string => Boolean(reason));

  const option = {
    animation: false,
    tooltip: {
      trigger: "axis",
      formatter: (params: Array<{ seriesName: string; value: [string, number | null] }>) => {
        if (!params.length) return "";
        const date = Array.isArray(params[0].value) ? params[0].value[0] : "";
        const values = params
          .filter((param) => Array.isArray(param.value) && param.value[1] != null)
          .map((param) => `${param.seriesName}：<b>${Number(param.value[1]).toFixed(2)}</b>`);
        return `${date}<br/>${values.join("<br/>")}<br/><span>实际：${report.methodology.actual_return_method}；影子：${report.methodology.shadow_return_method}；基准：${report.methodology.benchmark_return_method}</span>`;
      },
    },
    legend: { bottom: 0 },
    grid: { left: 44, right: 24, top: 28, bottom: 54, containLabel: true },
    xAxis: { type: "time" },
    yAxis: { type: "value", scale: true, name: "期初 = 100" },
    dataZoom: [{ type: "inside" }, { type: "slider", bottom: 22, height: 18 }],
    series: [
      ...curveSeries.map((series, index) => ({
        name: series.name,
        type: "line",
        data: series.data,
        connectNulls: series.connectNulls,
        showSymbol: false,
        lineStyle: { width: index === 0 ? 2.5 : 1.75, type: index === 0 ? "solid" : "dashed" },
      })),
      ...(actionMarkers.length ? [{ name: "调仓动作", type: "scatter", data: actionMarkers, tooltip: { trigger: "item" } }] : []),
    ],
  };

  return (
    <Card title="实际组合 / 不调仓影子组合 / 市场基准" extra={<Space wrap><Tag>实际：{report.methodology.actual_return_method}</Tag><Tag>影子：{report.methodology.shadow_return_method}</Tag></Space>}>
      {absentReasons.length > 0 && <Alert type="warning" showIcon style={{ marginBottom: 12 }} message={absentReasons.join("；")} />}
      {curveSeries.length === 0 ? (
        <Empty description="所选区间没有可展示的组合曲线" />
      ) : (
        <ReactECharts
          option={option}
          style={{ height: 390, width: "100%" }}
          opts={{ renderer: "canvas" }}
          onEvents={{
            click: (params: { data?: { campaignId?: string } }) => {
              if (params.data?.campaignId) onOpenCampaign(params.data.campaignId);
            },
          }}
        />
      )}
      <Text type="secondary">曲线直接使用后端对齐的 100 基准序列；数据缺口保持断开。点击动作标记可打开对应 Campaign。</Text>
    </Card>
  );
}
