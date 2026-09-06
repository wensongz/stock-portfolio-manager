import ImportBatchPanel from "./ImportBatchPanel.tsx";
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

export default function ImportWizard<Row extends ImportRow>({
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

  const close = () => {
    if (wizard.importing) return;
    wizard.reset();
    onClose();
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
          <Button type="primary" loading={wizard.importing} onClick={() => void startImport()}>
            检查 {selectedCount} 条记录
          </Button>
        </Space>
      : <Button type="primary" disabled={wizard.importing} onClick={close}>完成</Button>;

  return (
    <Modal open={open} title={title} width={width} onCancel={close} footer={footer} closable={!wizard.importing} maskClosable={!wizard.importing} keyboard={!wizard.importing} destroyOnHidden>
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
          onImported={onImported} onBusyChange={wizard.setImporting} />
      )}
    </Modal>
  );
}
