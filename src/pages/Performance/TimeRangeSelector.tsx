import { Button, DatePicker, Space } from "antd";
import type { TimeRange } from "../../stores/performanceStore";
import dayjs from "dayjs";

const { RangePicker } = DatePicker;

interface Props {
  timeRange: TimeRange;
  customStart: string | null;
  customEnd: string | null;
  onChange: (range: TimeRange, start?: string, end?: string) => void;
}

const PRESETS: { label: string; value: TimeRange }[] = [
  { label: "1周", value: "1W" },
  { label: "1月", value: "1M" },
  { label: "3月", value: "3M" },
  { label: "6月", value: "6M" },
  { label: "今年", value: "YTD" },
  { label: "1年", value: "1Y" },
  { label: "3年", value: "3Y" },
  { label: "5年", value: "5Y" },
  { label: "全部", value: "ALL" },
];

export default function TimeRangeSelector({ timeRange, customStart, customEnd, onChange }: Props) {
  return (
    <Space wrap>
      {PRESETS.map((p) => (
        <Button
          key={p.value}
          type={timeRange === p.value ? "primary" : "default"}
          size="small"
          onClick={() => onChange(p.value)}
        >
          {p.label}
        </Button>
      ))}
      <RangePicker
        size="small"
        value={
          timeRange === "CUSTOM" && customStart && customEnd
            ? [dayjs(customStart), dayjs(customEnd)]
            : null
        }
        onChange={(dates) => {
          if (dates && dates[0] && dates[1]) {
            onChange("CUSTOM", dates[0].format("YYYY-MM-DD"), dates[1].format("YYYY-MM-DD"));
          }
        }}
        allowClear={false}
      />
    </Space>
  );
}
