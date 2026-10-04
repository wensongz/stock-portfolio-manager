import ImportBatchPanel from "./ImportBatchPanel.tsx";
import { useRef, useState } from "react";
import { InboxOutlined } from "@ant-design/icons";
import { Alert, Button, Modal, Space, Steps, Table, Upload, message } from "antd";
import type { ColumnsType } from "antd/es/table";
import type { ImportRow } from "./types.ts";
import { useImportWizard, type ImportAdapter } from "./useImportWizard.ts";

const { Dragger } = Upload;

interface ImportWizardProps<Row extends ImportRow> {
  open: boolean;
  title: string;
  accountName: string;
  uploadTitle: string;
  uploadDescription: string;
  adapter: ImportAdapter<Row>;
  columns: (updateRow: (key: string, patch: Partial<Row>) => void, step: number) => ColumnsType<Row>;
  onClose: () => void;
  onImported: () => void;
  width?: number;
}

export default function ImportWizard<Row extends ImportRow>(props: ImportWizardProps<Row>) {
  // The wizard owns state above Modal's destroyOnHidden boundary. Unmount that
  // state even when the parent closes after import, and isolate each account.
  if (!props.open) return null;
  const { accountId, source, kind } = props.adapter;
  return <ImportWizardSession key={JSON.stringify([accountId, source, kind])} {...props} />;
}

function ImportWizardSession<Row extends ImportRow>({
  open,
  title,
  accountName,
  uploadTitle,
  uploadDescription,
  adapter,
  columns,
  onClose,
  onImported,
  width = 1100,
}: ImportWizardProps<Row>) {
  const wizard = useImportWizard(adapter);
  const selectedCount = wizard.rows.filter((row) => row.selected).length;
  const [confirmIncompleteClose, setConfirmIncompleteClose] = useState(false);
  const changedAccount = useRef(false);
  if (wizard.batch?.status === "applied") changedAccount.current = true;

  const finish = () => {
    if (wizard.importing) return;
    wizard.reset();
    if (changedAccount.current) onImported();
    onClose();
  };
  const close = () => {
    if (wizard.importing || wizard.problem) return;
    if (wizard.batch?.rows.some((row) => row.status === "failed")) setConfirmIncompleteClose(true);
    else finish();
  };

  const startImport = async () => {
    if (!(await wizard.importRows())) message.warning("请至少选择一条记录导入");
  };

  const tableColumns = columns(wizard.updateRow, wizard.importing ? 2 : wizard.step);

  const footer = wizard.step === 0
    ? <Button disabled={wizard.importing} onClick={close}>取消</Button>
    : wizard.step === 1
      ? <Space>
          <Button onClick={() => wizard.setStep(0)} disabled={wizard.importing}>上一步</Button>
          <Button type="primary" loading={wizard.importing} disabled={!!wizard.problem} onClick={() => void startImport()}>
            检查 {selectedCount} 条记录
          </Button>
        </Space>
      : <Button type="primary" disabled={wizard.importing} onClick={close}>完成</Button>;

  return (
    <><Modal open={open} title={title} width={width} onCancel={close} footer={footer} closable={!wizard.importing && !wizard.problem} maskClosable={!wizard.importing && !wizard.problem} keyboard={!wizard.importing && !wizard.problem} destroyOnHidden>
      <Steps
        current={wizard.step}
        items={[{ title: "上传文件" }, { title: "确认数据" }, { title: "批次核对与导入" }]}
        style={{ marginBottom: 24 }}
      />

      {wizard.step === 0 && (
        <Space orientation="vertical" size="middle" style={{ width: "100%" }}>
          <Alert type="info" showIcon message={`目标账户：${accountName}`} />
          <Dragger
            disabled={wizard.importing}
            accept=".csv,.txt"
            maxCount={1}
            fileList={wizard.fileList}
            beforeUpload={wizard.beforeUpload}
            onRemove={() => { wizard.reset(); return true; }}
          >
            <p className="ant-upload-drag-icon"><InboxOutlined /></p>
            <p className="ant-upload-text">{uploadTitle}</p>
            <p className="ant-upload-hint">{uploadDescription}</p>
          </Dragger>
          {wizard.parseError && <Alert type="error" showIcon message={wizard.parseError} />}
        </Space>
      )}

      {wizard.step === 1 && (
        <Space orientation="vertical" size="middle" style={{ width: "100%" }}>
          {wizard.parseError && <Alert type="error" showIcon message={wizard.parseError} />}
          {wizard.warnings.map((warning) => <Alert key={warning} type="warning" showIcon message={warning} />)}
          {wizard.parseIssues.length > 0 && <Alert type="warning" showIcon
            message={`${wizard.parseIssues.length} 条解析失败的记录尚未导入；修正原文件后可重新上传。`} />}
          <Alert
            type="info"
            showIcon
            message={`识别到 ${wizard.rows.length} 条记录，已选择 ${selectedCount} 条；可在导入前直接修改。`}
          />
          <Table<Row>
            rowKey="key"
            size="small"
            pagination={{ defaultPageSize: 20, showSizeChanger: true }}
            scroll={{ x: "max-content", y: 480 }}
            columns={tableColumns}
            dataSource={wizard.rows}
          />
        </Space>
      )}

      {wizard.step === 2 && wizard.batch && (
        <ImportBatchPanel batch={wizard.batch} onChange={wizard.setBatch}
          onImported={onImported} onBusyChange={wizard.setImporting} onReviseFailed={wizard.reviseFailedRows} />
      )}
    </Modal>
    <Modal open={wizard.problem !== null} width={800} closable={false} maskClosable={false} keyboard={false}
      title={wizard.problem === "preview" ? "批次检查失败，如何处理？"
        : wizard.problem === "prepare" ? "证券信息补全失败，如何处理？"
        : wizard.parseIssues.length ? `有 ${wizard.parseIssues.length} 条记录解析失败，如何处理？` : "解析未完成，如何处理？"}
      footer={<Space wrap>
        <Button disabled={wizard.importing} onClick={finish}>取消导入</Button>
        <Button disabled={wizard.importing} onClick={wizard.reset}>重新选择文件</Button>
        {wizard.rows.length > 0 && <Button type="primary" disabled={wizard.importing} onClick={wizard.dismissProblem}>
          {wizard.problem === "parse" ? "跳过错误行，继续检查" : "返回检查"}
        </Button>}
      </Space>}>
      <Space orientation="vertical" style={{ width: "100%" }}>
        {wizard.parseError && <Alert type="error" showIcon title={wizard.parseError} />}
        <p>{wizard.rows.length > 0 ? `已识别 ${wizard.rows.length} 条记录。你可以修正文件后重新上传，或明确跳过错误行后继续检查。` : "没有可继续导入的记录，请检查文件内容或选择其他文件。"}</p>
        <div style={{ maxHeight: 360, overflow: "auto" }}>
          {wizard.parseIssues.map((issue, index) => <div key={`${issue.line}-${index}`}>
            <strong>第 {issue.line} 行：{issue.message}</strong>
            <pre style={{ whiteSpace: "pre-wrap", overflowWrap: "anywhere" }}>{issue.raw}</pre>
          </div>)}
        </div>
      </Space>
    </Modal>
    <Modal open={confirmIncompleteClose} title="仍有记录导入失败，如何处理？" closable={false} maskClosable={false} keyboard={false}
      footer={<Space>
        <Button onClick={() => setConfirmIncompleteClose(false)}>继续处理</Button>
        <Button onClick={finish}>暂时跳过并完成</Button>
      </Space>}>
      成功记录已保留。失败记录尚未导入，你可以继续处理，或暂时跳过并稍后从导入批次历史中查看。
    </Modal></>
  );
}
