import { Tabs, Typography } from "antd";
import { SettingOutlined } from "@ant-design/icons";
import AIPage from "../AI";
import GeneralSettings from "./GeneralSettings";
import StockSplitSettings from "./StockSplitSettings";
import BackupSettings from "./BackupSettings";
import CategoriesPage from "../Categories";

const { Title } = Typography;

export default function SettingsPage() {
  const items = [
    {
      key: "general",
      label: "⚙️ 通用设置",
      children: <GeneralSettings />,
    },
    {
      key: "categories",
      label: "🏷️ 投资类别",
      children: <CategoriesPage />,
    },
    {
      key: "stockSplits",
      label: "📊 期权管理",
      children: <StockSplitSettings />,
    },
    {
      key: "backup",
      label: "💾 SQLite 备份",
      children: <BackupSettings />,
    },
    {
      key: "ai",
      label: "🤖 AI 配置",
      children: <AIPage />,
    },
  ];

  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 16 }}>
      <Title level={2}>
        <SettingOutlined /> 设置
      </Title>
      <Tabs items={items} />
    </div>
  );
}
