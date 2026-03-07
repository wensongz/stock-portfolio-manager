import { Card, Table, Tabs, Tag, Typography } from "antd";
import type { HoldingChangeItem, HoldingChanges } from "../../types";

const { Text } = Typography;

interface Props {
  changes: HoldingChanges;
  quarter1: string;
  quarter2: string;
}

function fmt(v: number) {
  return v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

const MARKET_LABELS: Record<string, string> = {
  US: "🇺🇸 美股",
  CN: "🇨🇳 A股",
  HK: "🇭🇰 港股",
};

function buildColumns(quarter1: string, quarter2: string, type: string) {
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
        render: (v: number | null) => (v != null ? fmt(v) : "—"),
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
        render: (v: number | null) => (v != null ? fmt(v) : "—"),
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
        <Text style={{ color: v > 0 ? "#3f8600" : v < 0 ? "#cf1322" : undefined }}>
          {v > 0 ? "+" : ""}
          {v.toLocaleString()}
        </Text>
      ),
    },
    {
      title: "市值变化",
      dataIndex: "value_change",
      key: "value_change",
      render: (v: number) => (
        <Text style={{ color: v > 0 ? "#3f8600" : v < 0 ? "#cf1322" : undefined }}>
          {v > 0 ? "+" : ""}
          {fmt(v)}
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
}: {
  data: HoldingChangeItem[];
  quarter1: string;
  quarter2: string;
  type: string;
}) {
  if (data.length === 0) {
    return <Text type="secondary">无</Text>;
  }
  return (
    <Table
      dataSource={data}
      columns={buildColumns(quarter1, quarter2, type)}
      rowKey="symbol"
      size="small"
      pagination={false}
    />
  );
}

export default function HoldingChangesTable({ changes, quarter1, quarter2 }: Props) {
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
        />
      ),
    },
  ];

  return (
    <Card size="small" title="持仓变动明细">
      <Tabs items={tabs} size="small" />
    </Card>
  );
}
