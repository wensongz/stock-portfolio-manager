import { useMemo, useState } from "react";
import { Button, Space, Table, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { EditOutlined, HistoryOutlined } from "@ant-design/icons";
import type { QuarterlyHoldingSnapshot, QuarterlySnapshot } from "../../types";
import { usePnlColor } from "../../hooks/usePnlColor";
import HoldingNotesEditor from "./HoldingNotesEditor";
import { aggregateSnapshotHoldings, parseSnapshotExchangeRates, type AggregatedSnapshotHolding } from "./aggregateSnapshotHoldings";

const { Text } = Typography;

interface Props {
  holdings: QuarterlyHoldingSnapshot[];
  snapshotId: string;
  loading?: boolean;
  snap?: QuarterlySnapshot;
}

const MARKET_PREFIX: Record<string, string> = { US: "$", CN: "¥", HK: "HK$" };
const fmt = (v: number) => v.toLocaleString("en-US", { minimumFractionDigits: 2, maximumFractionDigits: 2 });
const fmtPct = (v: number | null) => v == null ? "-" : `${v >= 0 ? "+" : ""}${v.toFixed(2)}%`;

export default function SnapshotHoldingsTable({ holdings, snapshotId, loading, snap }: Props) {
  const [notesTarget, setNotesTarget] = useState<QuarterlyHoldingSnapshot | null>(null);
  const [historyTarget, setHistoryTarget] = useState<QuarterlyHoldingSnapshot | null>(null);
  const { pnlColorDark } = usePnlColor();

  const snapshotRates = useMemo(() => parseSnapshotExchangeRates(snap?.exchange_rates), [snap?.exchange_rates]);
  const rows = useMemo(() => aggregateSnapshotHoldings(holdings, snapshotRates), [holdings, snapshotRates]);
  const totalValue = useMemo(() => rows.reduce((sum, row) => sum + row.market_value_base, 0), [rows]);

  const stockColumns: ColumnsType<AggregatedSnapshotHolding> = [
    { title: "代码", dataIndex: "symbol", key: "symbol", fixed: "left", width: 100, sorter: (a, b) => a.symbol.localeCompare(b.symbol), render: (v: string) => <Text strong>{v}</Text> },
    { title: "名称", dataIndex: "name", key: "name", width: 140, ellipsis: true },
    { title: "类别", dataIndex: "category_name", key: "category_name", width: 60, render: (v: string, row) => <Tag color={row.category_color}>{v}</Tag> },
    { title: "持仓数量", dataIndex: "shares", key: "shares", width: 90, align: "right", sorter: (a, b) => a.shares - b.shares, render: (v: number) => v.toLocaleString() },
    { title: "均价", dataIndex: "avg_cost", key: "avg_cost", width: 90, align: "right", render: (v: number) => v.toLocaleString("en-US", { minimumFractionDigits: 3, maximumFractionDigits: 3 }) },
    { title: "收盘价", dataIndex: "close_price", key: "close_price", width: 100, align: "right", render: fmt },
    { title: "市值", dataIndex: "market_value", key: "market_value", width: 140, align: "right", defaultSortOrder: "descend", sorter: (a, b) => a.market_value_base - b.market_value_base, render: (v: number, row) => `${MARKET_PREFIX[row.market] ?? ""}${fmt(v)}` },
    { title: "仓位%", key: "weight", width: 70, align: "right", sorter: (a, b) => a.market_value_base - b.market_value_base, render: (_, row) => `${(totalValue > 0 ? row.market_value_base / totalValue * 100 : 0).toFixed(2)}%` },
    { title: "盈亏金额", dataIndex: "pnl", key: "pnl", width: 140, align: "right", sorter: (a, b) => a.pnl - b.pnl, render: (v: number, row) => <Text style={{ color: pnlColorDark(v) }}>{v >= 0 ? "+" : "-"}{MARKET_PREFIX[row.market] ?? ""}{fmt(Math.abs(v))}</Text> },
    { title: "盈亏比例", dataIndex: "pnl_percent", key: "pnl_percent", width: 80, align: "right", render: (v: number | null) => <Text style={{ color: v == null ? undefined : pnlColorDark(v) }}>{fmtPct(v)}</Text> },
  ];

  const accountColumns: ColumnsType<QuarterlyHoldingSnapshot> = [
    { title: "账户", dataIndex: "account_name", key: "account_name", width: 160 },
    { title: "持仓数量", dataIndex: "shares", key: "shares", width: 90, align: "right", render: (v: number) => v.toLocaleString() },
    { title: "均价", dataIndex: "avg_cost", key: "avg_cost", width: 90, align: "right", render: (v: number) => v.toLocaleString("en-US", { minimumFractionDigits: 3, maximumFractionDigits: 3 }) },
    { title: "市值", dataIndex: "market_value", key: "market_value", width: 140, align: "right", render: (v: number, row) => `${MARKET_PREFIX[row.market] ?? ""}${fmt(v)}` },
    { title: "仓位", dataIndex: "weight", key: "weight", width: 70, align: "right", render: (v: number) => `${v.toFixed(2)}%` },
    { title: "盈亏金额", dataIndex: "pnl", key: "pnl", width: 150, align: "right", render: (v: number, row) => <Text style={{ color: pnlColorDark(v) }}>{v >= 0 ? "+" : "-"}{MARKET_PREFIX[row.market] ?? ""}{fmt(Math.abs(v))}</Text> },
    { title: "盈亏比例", dataIndex: "pnl_percent", key: "pnl_percent", width: 80, align: "right", render: (v: number | null) => <Text style={{ color: v == null ? undefined : pnlColorDark(v) }}>{fmtPct(v)}</Text> },
    { title: "操作思考", key: "notes", width: 150, align: "center", render: (_, row) => <Space size={0}><Button type="link" size="small" icon={<EditOutlined />} style={{ paddingInline: 4 }} onClick={() => setNotesTarget(row)}>{row.notes ? "编辑" : "记录"}</Button><Button type="link" size="small" icon={<HistoryOutlined />} style={{ paddingInline: 4 }} onClick={() => setHistoryTarget(row)}>历史</Button></Space> },
  ];

  return <>
    <Table dataSource={rows} columns={stockColumns} rowKey="symbol" loading={loading} size="small" className="account-detail-table" pagination={{ pageSize: 20, showSizeChanger: true }} scroll={{ x: 1100 }} expandable={{ expandedRowRender: (row) => <Table columns={accountColumns} dataSource={row.accountRows} rowKey="id" size="small" pagination={false} className="account-sub-table quarterly-holding-sub-table" />, rowExpandable: (row) => row.accountRows.length > 0 }} />
    {notesTarget && <HoldingNotesEditor holding={notesTarget} snapshotId={snapshotId} open onClose={() => setNotesTarget(null)} showHistory={false} />}
    {historyTarget && <HoldingNotesEditor holding={historyTarget} snapshotId={snapshotId} open onClose={() => setHistoryTarget(null)} showHistory />}
  </>;
}
