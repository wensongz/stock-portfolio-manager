import { Alert, Card, Typography } from "antd";

const { Text } = Typography;

export interface PortfolioAlertsTabProps {
  readonly title?: string;
}

export default function PortfolioAlertsTab({
  title = "组合提醒",
}: PortfolioAlertsTabProps) {
  return (
    <Card>
      <Alert
        type="info"
        showIcon
        title={title}
        description={
          <Text type="secondary">
            组合提醒功能将在 Task 8 中补齐，这里先保留页签位置。
          </Text>
        }
      />
    </Card>
  );
}
