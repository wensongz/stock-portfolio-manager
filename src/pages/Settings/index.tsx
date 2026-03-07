import { Tabs, Typography, Space, Tag } from "antd";
import { SettingOutlined } from "@ant-design/icons";
import AIPage from "../AI";

const { Title } = Typography;

export default function SettingsPage() {
  const items = [
    {
      key: "ai",
      label: (
        <Space>
          🤖 AI 配置
          <Tag color="orange" style={{ fontSize: 10 }}>实验性</Tag>
        </Space>
      ),
      children: <AIPage />,
    },
  ];

  return (
    <div className="space-y-6">
      <Title level={2}>
        <SettingOutlined /> 设置
      </Title>
      <Tabs items={items} />
    </div>
  );
}
