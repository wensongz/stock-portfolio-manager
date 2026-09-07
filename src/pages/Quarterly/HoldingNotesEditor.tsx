import { lazy, Suspense, useEffect, useMemo, useState } from "react";
import { Alert, Button, Descriptions, Divider, Modal, Space, Spin, Table, Tag, Typography } from "antd";
import { invoke } from "@tauri-apps/api/core";
import type { QuarterlyHoldingSnapshot } from "../../types";
import { useQuarterlyStore } from "../../stores/quarterlyStore";
import { usePnlColor } from "../../hooks/usePnlColor";
import { formatMoney } from "../../lib/formatMoney";
import { snapshotHoldingCurrency } from "./aggregateSnapshotHoldings";
import { createHoldingHistoryEditor, formatQuarterlyOperation, initialHoldingHistoryState } from "./holdingHistory";

const HoldingMarkdownEditor = lazy(() => import("./HoldingMarkdownEditor"));
const { Text } = Typography;

interface Props {
  holding: QuarterlyHoldingSnapshot | null;
  snapshotId: string;
  quarter?: string;
  open: boolean;
  onClose: () => void;
  showHistory: boolean;
}

const NOTE_TEMPLATE = `### 买入/卖出/持有理由
- 

### 当前估值判断
- 

### 后续计划
- 

### 风险提示
- `;

export default function HoldingNotesEditor(props: Props) {
  if (!props.holding) return null;
  return <ScopedHoldingNotesEditor key={JSON.stringify([props.snapshotId, props.holding.id])} {...props} holding={props.holding} />;
}

function ScopedHoldingNotesEditor({ holding, snapshotId, quarter, open, onClose, showHistory }: Omit<Props, "holding"> & { holding: QuarterlyHoldingSnapshot }) {
  const { updateHoldingNotes } = useQuarterlyStore();
  const { pnlColorDark } = usePnlColor();
  const [state, setState] = useState(() => ({ ...initialHoldingHistoryState(showHistory), notes: holding.notes ?? "" }));
  const editor = useMemo(() => createHoldingHistoryEditor(invoke, updateHoldingNotes, setState), [updateHoldingNotes]);
  const cash = holding.symbol.toUpperCase().startsWith("$CASH-");
  const currency = snapshotHoldingCurrency(holding);

  useEffect(() => {
    if (open) void editor.open({ snapshotId, holdingSnapshotId: holding.id, notes: holding.notes ?? "", showHistory });
    else editor.close();
    return () => editor.dispose();
    // Drafts reset only when opening or changing the selected snapshot row/mode.
  }, [editor, snapshotId, holding.id, open, showHistory]);

  const close = () => { editor.close(); onClose(); };
  const handleSave = async () => { if (await editor.save()) close(); };
  const rows = useMemo(() => (state.history?.rows ?? []).map(formatQuarterlyOperation), [state.history]);
  const historyColumns = [
    { title: "日期（UTC）", dataIndex: "date", key: "date", width: 160 },
    { title: "类型", dataIndex: "type", key: "type", width: 60 },
    ...(cash ? [
      { title: "来源证券", dataIndex: "security", key: "security", width: 190, ellipsis: true },
      { title: "资金变动", dataIndex: "cashDelta", key: "cashDelta", width: 140, align: "right" as const },
      { title: "费用", dataIndex: "commission", key: "commission", width: 80, align: "right" as const },
      { title: "余额", dataIndex: "runningBalance", key: "runningBalance", width: 140, align: "right" as const },
    ] : [
      { title: "数量", dataIndex: "shares", key: "shares", width: 110, align: "right" as const },
      { title: "价格", dataIndex: "price", key: "price", width: 130, align: "right" as const },
      { title: "金额", dataIndex: "amount", key: "amount", width: 130, align: "right" as const },
      { title: "费用", dataIndex: "commission", key: "commission", width: 100, align: "right" as const },
    ]),
    { title: "备注", dataIndex: "notes", key: "notes", width: 220 },
  ];

  return <Modal
    open={open}
    onCancel={close}
    width={state.mode === "history" ? 1100 : 800}
    title={<Space wrap><Text strong>{holding.symbol}</Text><Text type="secondary">{holding.name}</Text><Tag>{holding.market}</Tag><Text>{holding.account_name}</Text><Tag>{state.history?.quarter ?? quarter ?? "季度快照"}</Tag></Space>}
    footer={state.mode === "edit" ? <Space><Button onClick={close}>取消</Button><Button type="primary" loading={state.saving} disabled={state.saving} onClick={handleSave}>保存</Button></Space> : <Button onClick={close}>关闭</Button>}
  >
    <Space className="mb-3">
      <Button type={state.mode === "edit" ? "primary" : "default"} size="small" disabled={state.saving} onClick={() => void editor.setMode("edit")}>编辑思考</Button>
      <Button type={state.mode === "history" ? "primary" : "default"} size="small" disabled={state.saving} onClick={() => void editor.setMode("history")}>本季度操作</Button>
    </Space>

    <Descriptions size="small" column={3} className="mb-3">
      <Descriptions.Item label={cash ? "现金余额" : "持股数"}>{cash ? formatMoney(holding.shares, currency) : holding.shares.toLocaleString("en-US", { maximumFractionDigits: 6 })}</Descriptions.Item>
      <Descriptions.Item label="均成本">{formatMoney(holding.avg_cost, currency, 4)}</Descriptions.Item>
      <Descriptions.Item label="收盘价">{formatMoney(holding.close_price, currency, 4)}</Descriptions.Item>
      <Descriptions.Item label="盈亏%"><Text style={{ color: holding.pnl_percent != null ? pnlColorDark(holding.pnl_percent) : undefined }}>{holding.pnl_percent != null ? `${holding.pnl_percent >= 0 ? "+" : ""}${holding.pnl_percent.toFixed(2)}%` : "—"}</Text></Descriptions.Item>
      <Descriptions.Item label="类别">{holding.category_name}</Descriptions.Item>
      <Descriptions.Item label="仓位">{holding.weight.toFixed(2)}%</Descriptions.Item>
    </Descriptions>
    <Divider />

    {open && state.mode === "edit" && <>
      {state.saveError && <Alert type="error" showIcon title="保存失败，草稿已保留" description={state.saveError} className="mb-3" />}
      {!state.notes && <Button size="small" className="mb-2" disabled={state.saving} onClick={() => editor.setNotes(NOTE_TEMPLATE)}>使用模板</Button>}
      <div aria-busy={state.saving} style={state.saving ? { pointerEvents: "none", opacity: 0.7 } : undefined}>
        <Suspense fallback={<Spin size="small" />}><HoldingMarkdownEditor value={state.notes} onChange={(notes) => editor.setNotes(notes)} /></Suspense>
      </div>
    </>}

    {state.mode === "history" && <>
      <Space wrap className="mb-3">
        <Text strong>{cash ? "本季度资金变动" : "本季度操作"}</Text>
        <Text>{holding.account_name}</Text>
        {state.history && <Text type="secondary">{state.history.quarter} · {state.history.start_date} 至 {state.history.end_date}（UTC）</Text>}
        <Button size="small" disabled={state.historyLoading} onClick={() => void editor.retry()}>重新加载</Button>
      </Space>
      {state.historyError && <Alert type="error" showIcon title="操作记录加载失败" description={state.historyError} action={<Button size="small" onClick={() => void editor.retry()}>重试</Button>} className="mb-3" />}
      {state.historyLoading ? <Spin /> : state.history && <Table dataSource={rows} columns={historyColumns} rowKey="id" size="small" pagination={false} scroll={{ x: 1000, y: 450 }} locale={{ emptyText: "本季度暂无操作记录" }} />}
    </>}
  </Modal>;
}
