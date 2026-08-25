import { HistoryOutlined } from "@ant-design/icons";
import { Tabs, Typography } from "antd";
import OptionReviewTab from "./OptionReviewTab";
import StockReviewTab from "./StockReviewTab";

const { Title } = Typography;

export default function ReviewPage() {
  return (
    <div className="space-y-6">
      <Title level={2}><HistoryOutlined /> 操作复盘</Title>
      <Tabs
        defaultActiveKey="stock"
        items={[
          { key: "stock", label: "股票操作复盘", children: <StockReviewTab /> },
          { key: "options", label: "期权操作复盘", children: <OptionReviewTab /> },
        ]}
      />
    </div>
  );
}
