import { useState } from "react";
import { Button, Tabs, Typography } from "antd";
import { BellOutlined, HistoryOutlined } from "@ant-design/icons";
import { ALERTS_MENU_LABEL } from "./alertsCopy";
import { buildInvestmentAlertsTabs } from "./alertsTabs";
import AlertMessageCenter from "./AlertMessageCenter";
import PortfolioAlertsTab from "./PortfolioAlertsTab";
import PriceAlertsTab from "./PriceAlertsTab";

const { Title } = Typography;

export default function AlertsPage() {
  const [messageCenterOpen, setMessageCenterOpen] = useState(false);
  const { defaultActiveKey, items } = buildInvestmentAlertsTabs({
    portfolioTab: {
      label: "组合提醒",
      children: <PortfolioAlertsTab />,
    },
    priceTab: {
      label: "价格提醒",
      children: <PriceAlertsTab />,
    },
  });

  return (
    <div className="space-y-6">
      <Title level={2}>
        <BellOutlined style={{ color: "#fa8c16" }} /> {ALERTS_MENU_LABEL}
      </Title>
      <Tabs
        defaultActiveKey={defaultActiveKey}
        items={items}
        tabBarExtraContent={(
          <Button
            icon={<HistoryOutlined />}
            onClick={() => setMessageCenterOpen(true)}
          >
            消息中心
          </Button>
        )}
      />
      <AlertMessageCenter
        open={messageCenterOpen}
        onClose={() => setMessageCenterOpen(false)}
      />
    </div>
  );
}
