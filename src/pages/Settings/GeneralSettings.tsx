import { Card, Form, Select, Typography, message } from "antd";
import { useQuoteStore } from "../../stores/quoteStore";

const { Paragraph } = Typography;

const INTERVAL_OPTIONS = [
  { value: 60_000, label: "1 分钟" },
  { value: 2 * 60_000, label: "2 分钟" },
  { value: 5 * 60_000, label: "5 分钟（默认）" },
  { value: 10 * 60_000, label: "10 分钟" },
  { value: 15 * 60_000, label: "15 分钟" },
  { value: 30 * 60_000, label: "30 分钟" },
];

export default function GeneralSettings() {
  const { refreshIntervalMs, setRefreshInterval } = useQuoteStore();

  const handleChange = (value: number) => {
    setRefreshInterval(value);
    message.success("刷新频率已更新");
  };

  return (
    <div className="space-y-6">
      <Card title="行情刷新设置">
        <Form layout="vertical" style={{ maxWidth: 400 }}>
          <Form.Item label="自动刷新频率">
            <Select
              value={refreshIntervalMs}
              onChange={handleChange}
              options={INTERVAL_OPTIONS}
            />
          </Form.Item>
        </Form>
        <Paragraph type="secondary">
          设置持仓行情的自动刷新间隔时间，应用到所有行情数据的自动刷新。修改后将立即生效。
        </Paragraph>
      </Card>
    </div>
  );
}
