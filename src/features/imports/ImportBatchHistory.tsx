import { useEffect, useRef, useState } from "react";
import { Alert, Button, Card, Modal, Table, Tag } from "antd";
import { invoke } from "@tauri-apps/api/core";
import type { ImportBatch } from "./batchTypes";
import ImportBatchPanel from "./ImportBatchPanel";

export default function ImportBatchHistory({ accountId = null, refreshKey = 0 }: { accountId?: string | null; refreshKey?: number }) {
  const [batches, setBatches] = useState<ImportBatch[]>([]);
  const [selected, setSelected] = useState<ImportBatch | null>(null);
  const [loading, setLoading] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [reload, setReload] = useState(0);
  const openRequest = useRef(0);
  useEffect(() => {
    let active = true;
    setLoading(true); setError(null);
    invoke<ImportBatch[]>("list_import_batches", {accountId}).then((result) => {if(active) setBatches(result);}).catch((cause) => {if(active) setError(String(cause));}).finally(()=>{if(active) setLoading(false);});
    return () => {active = false;};
  }, [accountId, refreshKey, reload]);
  const open = async (id: string) => {
    const request = ++openRequest.current;
    setLoading(true); setError(null);
    try { const batch = await invoke<ImportBatch>("get_import_batch",{batchId:id}); if(request === openRequest.current) setSelected(batch); }
    catch(cause) { if(request === openRequest.current) setError(String(cause)); }
    finally {if(request === openRequest.current) setLoading(false);}
  };
  return <Card title="导入批次历史" extra={<Button disabled={loading || busy} onClick={()=>setReload((value)=>value+1)}>刷新</Button>}>
    {error && <Alert type="error" title="无法读取导入批次" description={error} showIcon />}
    <Table rowKey="id" size="small" loading={loading} dataSource={batches} pagination={{pageSize:10}} scroll={{x:"max-content"}} columns={[
      {title:"创建时间",dataIndex:"created_at"}, {title:"文件 / 来源",key:"source",render:(_,row)=>row.file_name || row.source},
      {title:"账户",dataIndex:"account_id"},
      {title:"状态",dataIndex:"status",render:(status:string)=><Tag>{({preview:"待导入",applied:"已提交",undone:"已撤销"} as Record<string,string>)[status] || status}</Tag>},
      {title:"操作",key:"actions",render:(_,row)=><Button disabled={busy || loading} onClick={()=>open(row.id)}>查看 / 重试 / 核对</Button>},
    ]} />
    <Modal title="导入批次详情" open={!!selected} footer={null} width={1100} maskClosable={!busy} closable={!busy} keyboard={!busy} onCancel={()=>{if(!busy) setSelected(null);}} destroyOnHidden>
      {selected && <ImportBatchPanel key={selected.id} batch={selected} onChange={(batch)=>{setSelected(batch); setReload((value)=>value+1);}} onBusyChange={setBusy} />}
    </Modal>
  </Card>;
}
