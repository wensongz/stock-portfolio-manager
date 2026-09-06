import { useEffect, useRef, useState } from "react";
import { Alert, Button, Input, InputNumber, Modal, Space, Table, Tag, Typography, message } from "antd";
import { invoke } from "@tauri-apps/api/core";
import type { ImportBatch } from "./batchTypes";
import { addExpectedBalance, reconciliationDisplayRows, reconciliationMatches, batchReconciliationArgs, batchApplyArgs, initialBatchSelection, selectableBatchRow, selectionAfterBatchResponse } from "./batchPanelState";

interface Props {
  batch: ImportBatch;
  onChange: (batch: ImportBatch) => void;
  onImported?: () => void;
  onBusyChange?: (busy: boolean) => void;
}
const statusLabels: Record<string, string> = {ready:"待导入", suspected:"疑似重复", duplicate:"明确重复", failed:"失败", imported:"已导入"};
export default function ImportBatchPanel({batch, onChange, onImported, onBusyChange}: Props) {
  const [selected, setSelected] = useState<string[]>(() => initialBatchSelection(batch));
  const [balances, setBalances] = useState<Record<string, number | null>>({});
  const [newSymbol, setNewSymbol] = useState("");
  const [newExpected, setNewExpected] = useState<number | null>(null);
  const dirtyBalances = useRef(new Set<string>());
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    setSelected(initialBatchSelection(batch));
    setError(null);
    dirtyBalances.current.clear();
    setNewSymbol("");
    setNewExpected(null);
  }, [batch.id]);
  useEffect(() => {
    setBalances((previous) => {
      const saved = Object.fromEntries(batch.reconciliation.map((row) => [row.symbol, row.expected_shares]));
      for (const symbol of dirtyBalances.current) {
        if (Object.prototype.hasOwnProperty.call(previous, symbol)) saved[symbol] = previous[symbol];
      }
      return saved;
    });
    setSelected((previous) => selectionAfterBatchResponse(batch, previous));
  }, [batch]);
  const run = async (command: string, args: Record<string, unknown>, mutated = false) => {
    setBusy(true); onBusyChange?.(true); setError(null);
    try {
      const updated = await invoke<ImportBatch>(command, args);
      if (command === "reconcile_import_batch" || command === "undo_import_batch") dirtyBalances.current.clear();
      onChange(updated);
      if (mutated) onImported?.();
      if (updated.conflict) setError(updated.conflict);
      else if (command === "apply_import_batch" && updated.rows.some((row) => row.status === "failed")) message.warning("部分行导入失败，请查看原因后重试");
      else message.success(command === "undo_import_batch" ? "已撤销批次" : command === "reconcile_import_batch" ? "已保存核对余额" : "批次已更新");
    } catch (cause) { setError(String(cause)); }
    finally { setBusy(false); onBusyChange?.(false); }
  };
  const apply = () => run("apply_import_batch", batchApplyArgs(batch, selected), true);
  const undone = batch.status === "undone";
  const reconciliationRows = reconciliationDisplayRows(batch, balances);
  const addBalance = () => {
    try {
      const updated = addExpectedBalance(batch, balances, newSymbol, newExpected);
      dirtyBalances.current.add(newSymbol.trim().toUpperCase());
      setBalances(updated);
      setNewSymbol("");
      setNewExpected(null);
    } catch (cause) { message.warning(cause instanceof Error ? cause.message : String(cause)); }
  };
  return <Space orientation="vertical" style={{width:"100%"}} size="middle">
    <Typography.Text>批次：{batch.file_name || batch.source} · {batch.id}</Typography.Text>
    {undone && <Alert type="info" title="此批次已撤销，不可再次提交" />}
    {(error || batch.conflict) && <Alert type="error" title="批次操作受阻" description={error || batch.conflict} showIcon />}
    <Alert type="info" title="明确重复和已导入行不可选择；疑似重复默认不选，勾选即确认仍需导入。失败行可勾选重试。" />
    <Table rowKey="key" size="small" dataSource={batch.rows} scroll={{x:"max-content"}} pagination={{defaultPageSize:10}}
      rowSelection={{selectedRowKeys:selected, onChange:(keys)=>setSelected(keys.map(String)), getCheckboxProps:(row)=>({disabled:busy || undone || !selectableBatchRow(row)})}}
      columns={[
        {title:"证券", key:"symbol", render:(_, row)=>String(row.data.symbol ?? "")},
        {title:"名称", key:"name", render:(_, row)=>String(row.data.name ?? "")},
        {title:"日期 / 操作", key:"operation", render:(_, row)=>[row.data.traded_at,row.data.transaction_type].filter(Boolean).join(" / ")},
        {title:"数量", key:"shares", render:(_, row)=>String(row.data.shares ?? "")},
        {title:"币种", key:"currency", render:(_, row)=>String(row.data.currency ?? "")},
        {title:"成交编号", dataIndex:"external_id"},
        {title:"价格 / 成本", key:"price", render:(_, row)=>String(row.data.price ?? row.data.avg_cost ?? "")},
        {title:"状态", dataIndex:"status", render:(status:string)=><Tag color={status === "failed" ? "red" : status === "imported" ? "green" : status === "suspected" ? "orange" : "default"}>{statusLabels[status] || status}</Tag>},
        {title:"说明", dataIndex:"error"},
      ]} />
    <Space wrap>
      <Button type="primary" loading={busy} disabled={undone || !!batch.conflict || !batchApplyArgs(batch,selected).rowKeys.length} onClick={apply}>
        {batch.status === "applied" ? "导入 / 重试所选行" : "导入所选行"}（{batchApplyArgs(batch,selected).rowKeys.length}）
      </Button>
      <Button disabled={busy || undone} onClick={()=>setSelected(batch.rows.filter((row)=>row.status === "failed" || row.status === "ready").map((row)=>row.key))}>选择待导入和失败行</Button>
      <Button disabled={busy} onClick={()=>run("get_import_batch", {batchId:batch.id})}>刷新已保存批次状态</Button>
      <Button danger disabled={busy || !batch.can_undo || undone} onClick={()=>Modal.confirm({title:"撤销此导入批次？", content:"将移除此批次写入的交易并恢复导入前的持仓和现金。账户在导入后有其他变更时会拒绝撤销。", okText:"确认撤销", cancelText:"取消", okButtonProps:{danger:true}, onOk:()=>run("undo_import_batch",{batchId:batch.id},true)})}>撤销批次</Button>
    </Space>
    <Typography.Title level={5}>持仓与现金核对</Typography.Title>
    <Typography.Text type="secondary">此处使用批次最后一次提交的持仓与现金快照。填写券商显示的数量或现金余额；未填写的项目保持未核对。差额为批次导入后数量减券商余额。券商持有而批次快照中缺失的证券可手动添加，其快照数量按 0 核对。</Typography.Text>
    <Space wrap>
      <Input aria-label="添加券商证券或现金代码" placeholder="证券代码或 $CASH-USD" value={newSymbol} disabled={busy || undone} onChange={(event)=>setNewSymbol(event.target.value)} style={{width:230}} />
      <InputNumber aria-label="新增券商余额" placeholder="券商数量 / 余额" value={newExpected} disabled={busy || undone} onChange={(value)=>setNewExpected(value === null ? null : Number(value))} style={{width:180}} />
      <Button disabled={busy || undone} onClick={addBalance}>添加券商证券/现金</Button>
    </Space>
    <Table rowKey="symbol" size="small" dataSource={reconciliationRows} pagination={false} scroll={{x:"max-content"}} columns={[
      {title:"证券 / 现金", dataIndex:"symbol"}, {title:"币种",dataIndex:"currency"}, {title:"批次快照：导入前",dataIndex:"before_shares"}, {title:"批次快照：导入后",dataIndex:"after_shares"},
      {title:"变化",key:"change",render:(_,row)=>row.after_shares-row.before_shares},
      {title:"券商余额",key:"expected",render:(_,row)=><InputNumber aria-label={`${row.symbol} 券商余额`} value={balances[row.symbol] ?? null} disabled={busy || undone} onChange={(value)=>{dirtyBalances.current.add(row.symbol); setBalances((previous)=>({...previous,[row.symbol]:value === null ? null : Number(value)}));}} />},
      {title:"已保存核对",key:"difference",render:(_,row)=>row.expected_shares == null ? <Tag>未核对</Tag> : <Tag color={reconciliationMatches(row) ? "green":"orange"}>{reconciliationMatches(row) ? `一致（差额 ${row.difference}）` : `差额 ${row.difference}`}</Tag>},
    ]} />
    <Button disabled={busy || undone || !reconciliationRows.length} onClick={()=>run("reconcile_import_batch",batchReconciliationArgs(batch, balances))}>保存核对余额</Button>
  </Space>;
}
