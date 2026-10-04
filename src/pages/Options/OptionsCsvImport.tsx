import { useEffect, useRef, useState } from "react";
import { Alert, Button, Modal, Space, Table, Tag, Typography, Upload, message } from "antd";
import { UploadOutlined } from "@ant-design/icons";
import type { ColumnsType } from "antd/es/table";
import { useOptionStore } from "../../stores/optionStore";
import type { OptionsCsvPreview, OptionsCsvPreviewRow } from "../../types";

const formatNumber = (value: number) => value.toLocaleString("en-US", { maximumFractionDigits: 20 });

const previewColumns: ColumnsType<OptionsCsvPreviewRow> = [
  { title: "CSV 行号", dataIndex: "row_number", width: 68, fixed: "left" },
  { title: "期权合约", dataIndex: "option_symbol", width: 180, fixed: "left" },
  {
    title: "交易时间", dataIndex: "traded_at", width: 160,
    render: (value: string | null) => value || "—",
  },
  {
    title: "方向", dataIndex: "action", width: 60,
    render: (value: string) => <Tag color={value === "BUY" ? "green" : "red"} style={{ marginInlineEnd: 0 }}>
      {value === "BUY" ? "买入" : "卖出"}
    </Tag>,
  },
  { title: "合约数量", dataIndex: "quantity", width: 76, align: "right", render: formatNumber },
  { title: "价格", dataIndex: "price", align: "right", render: formatNumber },
  { title: "金额", dataIndex: "amount", align: "right", render: formatNumber },
  { title: "佣金", dataIndex: "commission", align: "right", render: formatNumber },
  { title: "费用", dataIndex: "fee", align: "right", render: formatNumber },
  { title: "代码", dataIndex: "code" },
];

interface OptionsCsvImportProps {
  accountId: string;
  accountName: string;
  onImported: (accountId: string) => void;
}

interface PendingImport {
  fileName: string;
  csvContent: string;
  preview: OptionsCsvPreview;
}

export default function OptionsCsvImport(props: OptionsCsvImportProps) {
  // Each account owns a separate confirmation session, including in-flight previews.
  return <OptionsCsvImportSession key={props.accountId} {...props} />;
}

function OptionsCsvImportSession({ accountId, accountName, onImported }: OptionsCsvImportProps) {
  const { previewOptionsCsv, importOptionsCsv } = useOptionStore();
  const [pending, setPending] = useState<PendingImport | null>(null);
  const [previewing, setPreviewing] = useState(false);
  const [importing, setImporting] = useState(false);
  const busy = useRef(false);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  const previewFile = async (file: File) => {
    if (!accountId || busy.current || pending) return false;
    busy.current = true;
    setPreviewing(true);
    try {
      const csvContent = await file.text();
      if (!mounted.current) return false;
      const preview = await previewOptionsCsv(accountId, csvContent);
      if (mounted.current) setPending({ fileName: file.name, csvContent, preview });
    } catch (error) {
      if (mounted.current) message.error(`CSV 预览失败：${error}`);
    } finally {
      busy.current = false;
      if (mounted.current) setPreviewing(false);
    }
    return false;
  };

  const confirmImport = async () => {
    if (!pending || !pending.preview.importable || busy.current) return;
    busy.current = true;
    setImporting(true);
    try {
      const result = await importOptionsCsv(accountId, pending.csvContent);
      if (!mounted.current) return;
      setPending(null);
      const skipped = result.skipped + result.errors.length;
      if (skipped > 0) {
        message.warning(`导入完成：成功 ${result.imported} 条，跳过 ${skipped} 条`);
      } else {
        message.success(`导入成功：${result.imported} 条记录`);
      }
      onImported(accountId);
    } catch (error) {
      if (mounted.current) message.error(`导入失败：${error}`);
    } finally {
      busy.current = false;
      if (mounted.current) setImporting(false);
    }
  };

  return <>
    <Upload accept=".csv" showUploadList={false} beforeUpload={previewFile}
      disabled={!accountId || previewing || importing || !!pending}>
      <Button icon={<UploadOutlined />} loading={previewing} disabled={!accountId || importing || !!pending}>
        导入CSV
      </Button>
    </Upload>
    <Modal title="确认导入期权 CSV" width={1000} centered style={{ maxWidth: "calc(100vw - 32px)" }}
      styles={{
        container: { display: "flex", flexDirection: "column", maxHeight: "calc(100dvh - 48px)", boxSizing: "border-box", padding: "16px 20px" },
        header: { flexShrink: 0, marginBottom: 8 },
        body: { flex: "1 1 auto", minHeight: 0, overflowY: "auto" },
        footer: { flexShrink: 0, marginTop: 8 },
      }}
      open={!!pending} onOk={confirmImport}
      onCancel={() => { if (!busy.current) setPending(null); }}
      okText="确认导入" cancelText="取消" confirmLoading={importing}
      okButtonProps={{ disabled: !pending?.preview.importable }}
      cancelButtonProps={{ disabled: importing }} closable={!importing}
      keyboard={!importing} mask={{ closable: false }}>
      {pending && <Space orientation="vertical" size={8} style={{ width: "100%" }}>
        <div style={{ display: "flex", alignItems: "baseline", gap: 16 }}>
          <Typography.Text ellipsis title={accountName} style={{ maxWidth: "40%", flexShrink: 0 }}>
            目标账户：<Typography.Text strong>{accountName}</Typography.Text>
          </Typography.Text>
          <Typography.Text ellipsis title={pending.fileName} style={{ minWidth: 0, flex: 1 }}>
            文件：{pending.fileName}
          </Typography.Text>
        </div>
        <Space wrap size="large">
          <div>识别记录：{pending.preview.total_rows} 条</div>
          <div>可导入：{pending.preview.importable} 条</div>
          <div>将跳过：{pending.preview.skipped} 条</div>
        </Space>
        <Alert showIcon style={{ padding: "6px 10px" }} type={pending.preview.importable ? "info" : "warning"}
          title={pending.preview.importable
            ? "确认后才会写入记录，取消不会导入。"
            : "没有可导入的记录，请检查文件后重新选择。"}
          description={pending.preview.skipped > 0 ? "汇总行、缺少期权代码及校验未通过的记录将被跳过。" : undefined} />
        <Table<OptionsCsvPreviewRow>
          rowKey="row_number"
          size="small"
          tableLayout="auto"
          style={{ whiteSpace: "nowrap" }}
          styles={{
            header: { cell: { paddingInline: 6, paddingBlock: 4, fontSize: 13 } },
            body: { cell: { paddingInline: 6, paddingBlock: 4, fontSize: 13 } },
            pagination: { root: { marginBlock: 8 } },
          }}
          columns={previewColumns}
          dataSource={pending.preview.rows}
          pagination={{
            defaultPageSize: 20,
            showSizeChanger: true,
            showTotal: (total) => `共 ${total} 条可导入记录`,
          }}
          scroll={{ x: "max-content", y: 280 }}
          locale={{ emptyText: "没有可导入的记录" }}
        />
        {pending.preview.errors.length > 0 && <div>
          <Typography.Text strong>校验未通过的记录（{pending.preview.errors.length} 条）</Typography.Text>
          <ul style={{ maxHeight: 120, overflow: "auto", paddingLeft: 20, margin: "4px 0 0", overflowWrap: "anywhere" }}>
            {pending.preview.errors.map((error, index) => <li key={index}>{error}</li>)}
          </ul>
        </div>}
      </Space>}
    </Modal>
  </>;
}
