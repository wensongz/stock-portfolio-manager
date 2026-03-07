import { useState } from "react";
import { Button, Space, Table, Tag, Typography } from "antd";
import { EditOutlined, HistoryOutlined } from "@ant-design/icons";
import type { QuarterlyHoldingSnapshot } from "../../types";
import HoldingNotesEditor from "./HoldingNotesEditor";

const { Text } = Typography;

interface Props {
  holdings: QuarterlyHoldingSnapshot[];
  snapshotId: string;
  loading?: boolean;
}

function fmtPct(v: number) {
  return `${v >= 0 ? "+" : ""}${v.toFixed(2)}%`;
}

function fmt(v: number) {
  return v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
}

const MARKET_LABELS: Record<string, string> = {
  US: "🇺🇸 美股",
  CN: "🇨🇳 A股",
  HK: "🇭🇰 港股",
};

export default function SnapshotHoldingsTable({ holdings, snapshotId, loading }: Props) {
  const [notesTarget, setNotesTarget] = useState<QuarterlyHoldingSnapshot | null>(null);
  const [historySymbol, setHistorySymbol] = useState<string | null>(null);

  const columns = [
    {
      title: "市场",
      dataIndex: "market",
      key: "market",
      render: (m: string) => <Tag>{MARKET_LABELS[m] ?? m}</Tag>,
      filters: [
        { text: "美股", value: "US" },
        { text: "A股", value: "CN" },
        { text: "港股", value: "HK" },
      ],
      onFilter: (value: unknown, record: QuarterlyHoldingSnapshot) => record.market === value,
    },
    {
      title: "代码",
      dataIndex: "symbol",
      key: "symbol",
      render: (s: string) => <Text strong>{s}</Text>,
    },
    {
      title: "名称",
      dataIndex: "name",
      key: "name",
    },
    {
      title: "类别",
      dataIndex: "category_name",
      key: "category_name",
      render: (name: string, record: QuarterlyHoldingSnapshot) => (
        <Tag color={record.category_color}>{name}</Tag>
      ),
    },
    {
      title: "账户",
      dataIndex: "account_name",
      key: "account_name",
    },
    {
      title: "持股数",
      dataIndex: "shares",
      key: "shares",
      render: (v: number) => v.toLocaleString(),
    },
    {
      title: "均成本",
      dataIndex: "avg_cost",
      key: "avg_cost",
      render: (v: number) => fmt(v),
    },
    {
      title: "收盘价",
      dataIndex: "close_price",
      key: "close_price",
      render: (v: number) => fmt(v),
    },
    {
      title: "市值",
      dataIndex: "market_value",
      key: "market_value",
      render: (v: number) => <Text strong>{fmt(v)}</Text>,
      sorter: (a: QuarterlyHoldingSnapshot, b: QuarterlyHoldingSnapshot) =>
        a.market_value - b.market_value,
    },
    {
      title: "盈亏",
      dataIndex: "pnl",
      key: "pnl",
      render: (v: number) => (
        <Text style={{ color: v >= 0 ? "#3f8600" : "#cf1322" }}>
          {v >= 0 ? "+" : ""}{fmt(v)}
        </Text>
      ),
      sorter: (a: QuarterlyHoldingSnapshot, b: QuarterlyHoldingSnapshot) => a.pnl - b.pnl,
    },
    {
      title: "盈亏%",
      dataIndex: "pnl_percent",
      key: "pnl_percent",
      render: (v: number) => (
        <Text style={{ color: v >= 0 ? "#3f8600" : "#cf1322" }}>{fmtPct(v)}</Text>
      ),
      sorter: (a: QuarterlyHoldingSnapshot, b: QuarterlyHoldingSnapshot) =>
        a.pnl_percent - b.pnl_percent,
    },
    {
      title: "仓位%",
      dataIndex: "weight",
      key: "weight",
      render: (v: number) => `${v.toFixed(2)}%`,
      sorter: (a: QuarterlyHoldingSnapshot, b: QuarterlyHoldingSnapshot) => a.weight - b.weight,
    },
    {
      title: "操作思考",
      key: "notes",
      render: (_: unknown, record: QuarterlyHoldingSnapshot) => (
        <Space>
          <Button
            size="small"
            icon={<EditOutlined />}
            onClick={() => setNotesTarget(record)}
          >
            {record.notes ? "编辑" : "记录"}
          </Button>
          <Button
            size="small"
            icon={<HistoryOutlined />}
            onClick={() => setHistorySymbol(record.symbol)}
          >
            历史
          </Button>
        </Space>
      ),
    },
  ];

  return (
    <>
      <Table
        dataSource={holdings}
        columns={columns}
        rowKey="id"
        loading={loading}
        size="small"
        pagination={{ pageSize: 20 }}
        scroll={{ x: "max-content" }}
      />

      {/* Notes editor modal */}
      {notesTarget && (
        <HoldingNotesEditor
          holding={notesTarget}
          snapshotId={snapshotId}
          open={!!notesTarget}
          onClose={() => setNotesTarget(null)}
          showHistory={false}
        />
      )}

      {/* Notes history modal */}
      {historySymbol && (
        <HoldingNotesEditor
          holding={holdings.find((h) => h.symbol === historySymbol) ?? null}
          snapshotId={snapshotId}
          open={!!historySymbol}
          onClose={() => setHistorySymbol(null)}
          showHistory={true}
        />
      )}
    </>
  );
}
