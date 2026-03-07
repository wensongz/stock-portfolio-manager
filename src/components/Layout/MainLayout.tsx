import { useState } from "react";
import { Layout, Menu } from "antd";
import { useNavigate, useLocation } from "react-router-dom";
import {
  DashboardOutlined,
  BankOutlined,
  StockOutlined,
  SwapOutlined,
  TagsOutlined,
} from "@ant-design/icons";

const { Sider, Content } = Layout;

const menuItems = [
  { key: "/dashboard", icon: <DashboardOutlined />, label: "仪表盘" },
  { key: "/accounts", icon: <BankOutlined />, label: "证券账户" },
  { key: "/holdings", icon: <StockOutlined />, label: "持仓管理" },
  { key: "/transactions", icon: <SwapOutlined />, label: "交易记录" },
  { key: "/categories", icon: <TagsOutlined />, label: "投资类别" },
];

interface Props {
  children: React.ReactNode;
}

export default function MainLayout({ children }: Props) {
  const [collapsed, setCollapsed] = useState(false);
  const navigate = useNavigate();
  const location = useLocation();

  return (
    <Layout style={{ minHeight: "100vh" }}>
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
          selectedKeys={[location.pathname]}
          items={menuItems}
          onClick={({ key }) => navigate(key)}
        />
      </Sider>
      <Layout>
        <Content className="p-6 bg-gray-50" style={{ minHeight: "100vh" }}>
          {children}
        </Content>
      </Layout>
    </Layout>
  );
}
