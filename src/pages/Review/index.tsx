import { HistoryOutlined } from "@ant-design/icons";
import { Tabs, Typography } from "antd";
import { useState } from "react";
import OptionReviewTab from "./OptionReviewTab";
import StockReviewTab from "./StockReviewTab";
import { isReviewTab, loadReviewTab, saveReviewTab } from "./reviewTabPreference";

const { Title } = Typography;

export default function ReviewPage() {
  const [activeTab, setActiveTab] = useState(() => loadReviewTab(localStorage));

  return (
    <div className="space-y-6">
      <Title level={2}><HistoryOutlined /> 操作复盘</Title>
      <Tabs
        activeKey={activeTab}
        onChange={(tab) => {
          if (!isReviewTab(tab)) return;
          saveReviewTab(localStorage, tab);
          setActiveTab(tab);
        }}
        items={[
          { key: "stock", label: "股票操作复盘", children: <StockReviewTab /> },
          { key: "options", label: "期权操作复盘", children: <OptionReviewTab /> },
        ]}
      />
    </div>
  );
}
