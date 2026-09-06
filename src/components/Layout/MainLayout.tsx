import { useState } from "react";
import { Layout, Menu } from "antd";
import { useNavigate, useLocation } from "react-router-dom";
import {
  DashboardOutlined,
  BankOutlined,
  StockOutlined,
  SwapOutlined,
  BarChartOutlined,
  LineChartOutlined,
  CalendarOutlined,
  ImportOutlined,
  BellOutlined,
  HistoryOutlined,
  SettingOutlined,
  FundOutlined,
  RobotOutlined,
  GiftOutlined,
} from "@ant-design/icons";
import { ALERTS_MENU_LABEL } from "../../pages/Alerts/alertsCopy";

const { Sider, Content } = Layout;

const menuItems = [
  { key: "/dashboard", icon: <DashboardOutlined />, label: "仪表盘" },
  { key: "/statistics", icon: <BarChartOutlined />, label: "统计分析" },
  { key: "/performance", icon: <LineChartOutlined />, label: "绩效分析" },
  { key: "/quarterly", icon: <CalendarOutlined />, label: "季度分析" },
  { key: "/accounts", icon: <BankOutlined />, label: "证券账户" },
  { key: "/holdings", icon: <StockOutlined />, label: "持仓管理" },
  { key: "/transactions", icon: <SwapOutlined />, label: "交易记录" },
  { key: "/options", icon: <FundOutlined />, label: "期权管理" },
  { key: "/dividends", icon: <GiftOutlined />, label: "分红分析" },
  { key: "/review", icon: <HistoryOutlined />, label: "操作复盘" },
  { key: "/ai-assistant", icon: <RobotOutlined />, label: "AI 助手" },
  { key: "/alerts", icon: <BellOutlined />, label: ALERTS_MENU_LABEL },
  { key: "/import", icon: <ImportOutlined />, label: "导入导出" },
  { key: "/settings", icon: <SettingOutlined />, label: "设置" },
];

interface Props {
  children: React.ReactNode;
}

export default function MainLayout({ children }: Props) {
  const [collapsed, setCollapsed] = useState(false);
  const navigate = useNavigate();
  const location = useLocation();

  return (
    <Layout style={{ height: "100vh", overflow: "hidden" }}>
      <Sider
        collapsible
        collapsed={collapsed}
        onCollapse={setCollapsed}
        theme="dark"
        width={200}
      >
        <div
          className="flex items-center justify-center py-4 px-2"
          style={{ color: "white", fontSize: collapsed ? 12 : 16, fontWeight: "bold" }}
        >
          {collapsed ? "SPM" : "📈 Portfolio"}
        </div>
        <Menu
          theme="dark"
          mode="inline"
          selectedKeys={[
            menuItems.find((item) => location.pathname.startsWith(item.key))?.key ??
              location.pathname,
          ]}
          items={menuItems}
          onClick={({ key }) => {
            // antd Menu in inline mode occasionally swallows clicks when the
            // target item is already in `selectedKeys` (which can happen with
            // our `startsWith` matching). Log + a defensive try/catch helps
            // diagnose, and falling back to `window.location` guarantees the
            // navigation actually happens even if the router hook misbehaves.
            try {
              navigate(key);
            } catch (err) {
              console.error("[MainLayout] navigate failed, falling back to window.location", err);
              window.location.assign(key);
            }
          }}
        />
      </Sider>
      <Layout style={{ height: "100vh", overflow: "hidden" }}>
        <Content
          className="p-6"
          style={{ height: "100%", overflowY: "auto", backgroundColor: "var(--color-bg-layout)" }}
        >
          {children}
        </Content>
      </Layout>
    </Layout>
  );
}
