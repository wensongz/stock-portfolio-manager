import { Card, Table, Tabs, Tag, Typography } from "antd";
import type { HoldingChangeItem, HoldingChanges } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";
import { formatQuarterlyMoney } from "./formatMoney";

const { Text } = Typography;

interface Props {
  changes: HoldingChanges;
  quarter1: string;
  quarter2: string;
  title?: string;
}

function formatMarketValue(value: number, market: string) {
  const currency = market === "CN" ? "CNY" : market === "HK" ? "HKD" : "USD";
  return formatQuarterlyMoney(value, currency);
}

const MARKET_LABELS: Record<string, string> = {
  US: "🇺🇸 美股",
  CN: "🇨🇳 A股",
  HK: "🇭🇰 港股",
};

function buildColumns(quarter1: string, quarter2: string, type: string, colorFn: (v: number) => string) {
  const base = [
    {
      title: "市场",
      dataIndex: "market",
      key: "market",
      render: (m: string) => <Tag>{MARKET_LABELS[m] ?? m}</Tag>,
    },
    {
      title: "代码",
      dataIndex: "symbol",
      key: "symbol",
      render: (s: string) => <Text strong>{s}</Text>,
    },
    { title: "名称", dataIndex: "name", key: "name" },
    { title: "类别", dataIndex: "category_name", key: "category_name" },
  ];

  if (type === "new") {
    return [
      ...base,
      {
        title: `${quarter2} 持股`,
        dataIndex: "q2_shares",
        key: "q2_shares",
        render: (v: number | null) => (v != null ? v.toLocaleString() : "—"),
      },
      {
        title: `${quarter2} 市值`,
        dataIndex: "q2_value",
        key: "q2_value",
        render: (v: number | null, row: HoldingChangeItem) => (v != null ? formatMarketValue(v, row.market) : "—"),
      },
    ];
  }

  if (type === "closed") {
    return [
      ...base,
      {
        title: `${quarter1} 持股`,
        dataIndex: "q1_shares",
        key: "q1_shares",
        render: (v: number | null) => (v != null ? v.toLocaleString() : "—"),
      },
      {
        title: `${quarter1} 市值`,
        dataIndex: "q1_value",
        key: "q1_value",
        render: (v: number | null, row: HoldingChangeItem) => (v != null ? formatMarketValue(v, row.market) : "—"),
      },
    ];
  }

  return [
    ...base,
    {
      title: `${quarter1} 持股`,
      dataIndex: "q1_shares",
      key: "q1_shares",
      render: (v: number | null) => (v != null ? v.toLocaleString() : "—"),
    },
    {
      title: `${quarter2} 持股`,
      dataIndex: "q2_shares",
      key: "q2_shares",
      render: (v: number | null) => (v != null ? v.toLocaleString() : "—"),
    },
    {
      title: "股数变化",
      dataIndex: "shares_change",
      key: "shares_change",
      render: (v: number) => (
        <Text style={{ color: v > 0 ? colorFn(v) : v < 0 ? colorFn(v) : undefined }}>
          {v > 0 ? "+" : ""}
          {v.toLocaleString()}
        </Text>
      ),
    },
    {
      title: "市值变化",
      dataIndex: "value_change",
      key: "value_change",
      render: (v: number, row: HoldingChangeItem) => (
        <Text style={{ color: v > 0 ? colorFn(v) : v < 0 ? colorFn(v) : undefined }}>
          {v > 0 ? "+" : ""}
          {formatMarketValue(v, row.market)}
        </Text>
      ),
    },
  ];
}

function HoldingTable({
  data,
  quarter1,
  quarter2,
  type,
  colorFn,
}: {
  data: HoldingChangeItem[];
  quarter1: string;
  quarter2: string;
  type: string;
  colorFn: (v: number) => string;
}) {
  if (data.length === 0) {
    return <Text type="secondary">无</Text>;
  }
  return (
    <Table
      dataSource={data}
      columns={buildColumns(quarter1, quarter2, type, colorFn)}
      rowKey="symbol"
      size="small"
      pagination={false}
    />
  );
}

export default function HoldingChangesTable({ changes, quarter1, quarter2, title }: Props) {
  const { pnlColorDark } = usePnlColor();
  const tabs = [
    {
      key: "new",
      label: (
        <span>
          🟢 新增持仓{" "}
          <Tag color="green">{changes.new_holdings.length}</Tag>
        </span>
      ),
      children: (
        <HoldingTable
          data={changes.new_holdings}
          quarter1={quarter1}
          quarter2={quarter2}
          type="new"
          colorFn={pnlColorDark}
        />
      ),
    },
    {
      key: "closed",
      label: (
        <span>
          🔴 清仓{" "}
          <Tag color="red">{changes.closed_holdings.length}</Tag>
        </span>
      ),
      children: (
        <HoldingTable
          data={changes.closed_holdings}
          quarter1={quarter1}
          quarter2={quarter2}
          type="closed"
          colorFn={pnlColorDark}
        />
      ),
    },
    {
      key: "increased",
      label: (
        <span>
          📈 加仓{" "}
          <Tag color="blue">{changes.increased.length}</Tag>
        </span>
      ),
      children: (
        <HoldingTable
          data={changes.increased}
          quarter1={quarter1}
          quarter2={quarter2}
          type="change"
          colorFn={pnlColorDark}
        />
      ),
    },
    {
      key: "decreased",
      label: (
        <span>
          📉 减仓{" "}
          <Tag color="orange">{changes.decreased.length}</Tag>
        </span>
      ),
      children: (
        <HoldingTable
          data={changes.decreased}
          quarter1={quarter1}
          quarter2={quarter2}
          type="change"
          colorFn={pnlColorDark}
        />
      ),
    },
    {
      key: "unchanged",
      label: (
        <span>
          ➡️ 不变{" "}
          <Tag>{changes.unchanged.length}</Tag>
        </span>
      ),
      children: (
        <HoldingTable
          data={changes.unchanged}
          quarter1={quarter1}
          quarter2={quarter2}
          type="change"
          colorFn={pnlColorDark}
        />
      ),
    },
  ];

  return (
    <Card size="small" title={title ?? "持仓变动明细"}>
      <Tabs items={tabs} size="small" />
    </Card>
  );
}
