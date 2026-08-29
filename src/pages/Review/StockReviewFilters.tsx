import { Button, DatePicker, Flex, Input, Select, Space, Typography } from "antd";
import { ReloadOutlined, RobotOutlined } from "@ant-design/icons";
import dayjs from "dayjs";
import { useEffect, useState } from "react";
import type { Account, Currency, Market, StockReviewFilters as Filters, StockReviewPeriodPreset } from "../../types";
import { getStockReviewDateRange } from "./stockReviewViewModel";

const { Text } = Typography;
const { RangePicker } = DatePicker;

interface Props {
  filters: Filters;
  accounts: Account[];
  loading: boolean;
  canAskAi: boolean;
  onChange: (filters: Filters) => void;
  onRefresh: () => void;
  onAskAi: () => void;
}

export default function StockReviewFilters({
  filters,
  accounts,
  loading,
  canAskAi,
  onChange,
  onRefresh,
  onAskAi,
}: Props) {
  const [specifiedBenchmark, setSpecifiedBenchmark] = useState(filters.benchmarkSymbol != null);
  useEffect(() => setSpecifiedBenchmark(filters.benchmarkSymbol != null), [filters.benchmarkSymbol]);

  const update = (patch: Partial<Filters>) => onChange({ ...filters, ...patch });
  const changePreset = (periodPreset: StockReviewPeriodPreset) => {
    const range = periodPreset === "CUSTOM"
      ? { startDate: filters.startDate, endDate: filters.endDate }
      : getStockReviewDateRange(periodPreset);
    update({ periodPreset, ...range });
  };

  return (
    <Flex wrap gap={12} align="end" justify="space-between">
      <Flex wrap gap={12} align="end" style={{ flex: 1 }}>
        <Space orientation="vertical" size={2}>
          <Text type="secondary">账户</Text>
          <Select
            aria-label="股票复盘账户"
            value={filters.accountId ?? "all"}
            style={{ minWidth: 150 }}
            onChange={(value) => update({ accountId: value === "all" ? null : value })}
            options={[{ value: "all", label: "全部账户" }, ...accounts.map((account) => ({ value: account.id, label: account.name }))]}
          />
        </Space>
        <Space orientation="vertical" size={2}>
          <Text type="secondary">周期</Text>
          <Select
            aria-label="股票复盘周期"
            value={filters.periodPreset}
            style={{ minWidth: 130 }}
            onChange={changePreset}
            options={[
              { value: "QTD", label: "本季度" },
              { value: "PREV_QUARTER", label: "上季度" },
              { value: "YTD", label: "今年以来" },
              { value: "1Y", label: "近一年" },
              { value: "CUSTOM", label: "自定义" },
            ]}
          />
        </Space>
        {filters.periodPreset === "CUSTOM" && (
          <Space orientation="vertical" size={2}>
            <Text type="secondary">日期范围</Text>
            <RangePicker
              aria-label="股票复盘自定义日期"
              value={[dayjs(filters.startDate), dayjs(filters.endDate)]}
              allowClear={false}
              onChange={(dates) => {
                if (!dates?.[0] || !dates[1]) return;
                update({ startDate: dates[0].format("YYYY-MM-DD"), endDate: dates[1].format("YYYY-MM-DD") });
              }}
            />
          </Space>
        )}
        <Space orientation="vertical" size={2}>
          <Text type="secondary">市场</Text>
          <Select
            aria-label="股票复盘市场"
            value={filters.market ?? "all"}
            style={{ minWidth: 120 }}
            onChange={(value) => update({ market: value === "all" ? null : value as Market })}
            options={[{ value: "all", label: "全部市场" }, { value: "US", label: "美股" }, { value: "CN", label: "A 股" }, { value: "HK", label: "港股" }]}
          />
        </Space>
        <Space orientation="vertical" size={2}>
          <Text type="secondary">基准</Text>
          <Select
            aria-label="股票复盘基准模式"
            value={specifiedBenchmark ? "specified" : "auto"}
            style={{ minWidth: 150 }}
            onChange={(value) => {
              const specified = value === "specified";
              setSpecifiedBenchmark(specified);
              if (!specified) update({ benchmarkSymbol: null });
            }}
            options={[{ value: "auto", label: "自动混合基准" }, { value: "specified", label: "指定基准" }]}
          />
        </Space>
        {specifiedBenchmark && (
          <Space orientation="vertical" size={2}>
            <Text type="secondary">基准代码</Text>
            <Input
              aria-label="股票复盘指定基准代码"
              value={filters.benchmarkSymbol ?? ""}
              placeholder="例如 ^GSPC"
              style={{ width: 140 }}
              onChange={(event) => update({ benchmarkSymbol: event.target.value.trim() || null })}
            />
          </Space>
        )}
        <Space orientation="vertical" size={2}>
          <Text type="secondary">基准币种</Text>
          <Select
            aria-label="股票复盘基准币种"
            value={filters.baseCurrency}
            style={{ width: 100 }}
            onChange={(baseCurrency: Currency) => update({ baseCurrency })}
            options={["USD", "CNY", "HKD"].map((value) => ({ value, label: value }))}
          />
        </Space>
      </Flex>
      <Space wrap>
        <Button icon={<ReloadOutlined />} loading={loading} onClick={onRefresh}>刷新复盘</Button>
        <Button type="primary" icon={<RobotOutlined />} disabled={!canAskAi} onClick={onAskAi}>请 AI 深度复盘</Button>
      </Space>
    </Flex>
  );
}
