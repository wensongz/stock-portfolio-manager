import { Tabs, Typography } from "antd";
import { BellOutlined } from "@ant-design/icons";
import { ALERTS_MENU_LABEL } from "./alertsCopy";
import { buildInvestmentAlertsTabs } from "./alertsTabs";
import PortfolioAlertsTab from "./PortfolioAlertsTab";
import PriceAlertsTab from "./PriceAlertsTab";

const { Title } = Typography;

export default function AlertsPage() {
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
      <Tabs defaultActiveKey={defaultActiveKey} items={items} />
    </div>
  );
}
