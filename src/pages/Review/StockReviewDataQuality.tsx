import { Alert, Collapse, Descriptions, List, Space, Tag, Typography } from "antd";
import type { StockReviewReport } from "../../types";
import { getStockReviewStatusDisplay, sortStockReviewIssues } from "./stockReviewViewModel";

const { Text } = Typography;

function coverage(value: number | null) {
  return value == null ? "—" : `${(value * 100).toFixed(1)}%`;
}

export default function StockReviewDataQuality({ report }: { report: StockReviewReport }) {
  const { data_quality: quality, methodology } = report;
  const issues = sortStockReviewIssues(quality.issues);
  const pending = report.actions.filter((action) => action.status === "pending").length;
  const status = getStockReviewStatusDisplay(quality.availability.status);
  const summary = `行情覆盖 ${coverage(quality.market_data_coverage)}，汇率覆盖 ${coverage(quality.exchange_rate_coverage)}，共分析 ${report.actions.length} 项操作，其中 ${pending} 项仍在观察中。`;

  return (
    <Alert
      type={issues.some((issue) => issue.severity === "error") ? "error" : issues.length ? "warning" : "success"}
      showIcon
      message={<Space wrap><Text>{summary}</Text><Tag color={status.color}>{status.label}</Tag></Space>}
      description={
        <Collapse
          ghost
          size="small"
          items={[{
            key: "quality-detail",
            label: issues.length ? `查看 ${issues.length} 项数据限制与计算口径` : "查看计算口径",
            children: (
              <Space orientation="vertical" size="middle" style={{ width: "100%" }}>
                {issues.length > 0 && (
                  <List
                    size="small"
                    dataSource={issues}
                    renderItem={(issue) => (
                      <List.Item>
                        <Space wrap align="start">
                          <Tag color={issue.severity === "error" ? "red" : issue.severity === "warning" ? "gold" : "blue"}>
                            {issue.severity === "error" ? "阻断" : issue.severity === "warning" ? "警告" : "信息"}
                          </Tag>
                          <Text>{issue.message}</Text>
                          {issue.affected_symbol && <Tag>{issue.affected_symbol}</Tag>}
                          {issue.affected_date && <Text type="secondary">{issue.affected_date}</Text>}
                          <Text type="secondary">影响：{issue.code}</Text>
                        </Space>
                      </List.Item>
                    )}
                  />
                )}
                <Descriptions size="small" column={{ xs: 1, sm: 2, lg: 3 }}>
                  <Descriptions.Item label="实际收益口径">{methodology.actual_return_method}</Descriptions.Item>
                  <Descriptions.Item label="影子收益口径">{methodology.shadow_return_method}</Descriptions.Item>
                  <Descriptions.Item label="基准口径">{methodology.benchmark_return_method}</Descriptions.Item>
                  <Descriptions.Item label="基准">{methodology.benchmark_symbol ?? "期初固定权重自动混合"}</Descriptions.Item>
                  <Descriptions.Item label="行情覆盖">{coverage(methodology.market_data_coverage.coverage_ratio)}</Descriptions.Item>
                  <Descriptions.Item label="行情提供方">由后端缓存与行情服务确定（当前契约未单列）</Descriptions.Item>
                  <Descriptions.Item label="汇率覆盖">{coverage(methodology.exchange_rate_coverage.coverage_ratio)}</Descriptions.Item>
                  <Descriptions.Item label="算法版本">{methodology.algorithm_version}</Descriptions.Item>
                  <Descriptions.Item label="区间回撤">{quality.interval_drawdown_only ? "仅按所选区间计算" : "包含完整可用区间"}</Descriptions.Item>
                </Descriptions>
              </Space>
            ),
          }]}
        />
      }
    />
  );
}
