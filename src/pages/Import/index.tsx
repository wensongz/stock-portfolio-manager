import { useState, useEffect } from "react";
import {
  Card,
  Upload,
  Button,
  Select,
  Table,
  Space,
  Alert,
  Steps,
  message,
  Typography,
  Tag,
  Divider,
  Form,
  Modal,
  Empty,
} from "antd";
import {
  UploadOutlined,
  DownloadOutlined,
  CheckCircleOutlined,
  ImportOutlined,
  BankOutlined,
  FileTextOutlined,
  DeleteOutlined,
} from "@ant-design/icons";
import { invoke } from "@tauri-apps/api/core";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import type { ImportPreview, ImportResult, ExportFilters, ImportOptionsResult } from "../../types";
import { useAccountStore } from "../../stores/accountStore";

const { Title, Text } = Typography;

type BrokerType = "hsbc_hk" | "everbright";

interface BrokerFileBoxProps {
  title: string;
  description: string;
  files: string[];
  accept: "pdf" | "xls";
  onChange: (files: string[]) => void;
}

function BrokerFileBox({ title, description, files, accept, onChange }: BrokerFileBoxProps) {
  const selectFiles = async () => {
    const selected = await openDialog({
      multiple: true,
      directory: false,
      filters: [{
        name: accept === "pdf" ? "PDF 文件" : "光大对账单",
        extensions: [accept],
      }],
    });
    if (!selected) return;
    onChange(Array.isArray(selected) ? selected : [selected]);
  };

  const fileName = (path: string) => path.split(/[\\/]/).pop() || path;

  return (
    <div style={{ border: "1px dashed #d9d9d9", borderRadius: 8, padding: 16 }}>
      <Space orientation="vertical" size={6} style={{ width: "100%" }}>
        <Space style={{ justifyContent: "space-between", width: "100%" }}>
          <div>
            <Text strong>{title}</Text>
            <Text type="secondary" style={{ display: "block", fontSize: 12 }}>
              {description}
            </Text>
          </div>
          <Space>
            <Button icon={<FileTextOutlined />} onClick={selectFiles}>选择文件</Button>
            {files.length > 0 && (
              <Button icon={<DeleteOutlined />} onClick={() => onChange([])} aria-label={`清空${title}`} />
            )}
          </Space>
        </Space>
        {files.length === 0 ? (
          <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="尚未选择文件" styles={{ image: { height: 26 } }} />
        ) : (
          <div style={{ maxHeight: 96, overflowY: "auto" }}>
            {files.map((path) => (
              <div key={path} style={{ lineHeight: "24px" }} title={path}>
                <FileTextOutlined style={{ marginRight: 8, color: "#1677ff" }} />
                {fileName(path)}
              </div>
            ))}
          </div>
        )}
      </Space>
    </div>
  );
}

export default function ImportPage() {
  const { accounts, fetchAccounts } = useAccountStore();
  type DataType = "holdings" | "transactions" | "options";
  const [currentStep, setCurrentStep] = useState(0);
  const [dataType, setDataType] = useState<DataType>("holdings");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);
  const [optionsImportResult, setOptionsImportResult] = useState<ImportOptionsResult | null>(null);
  const [selectedAccountId, setSelectedAccountId] = useState("");
  const [loading, setLoading] = useState(false);
  const [rawCsvContent, setRawCsvContent] = useState("");
  const [brokerModalOpen, setBrokerModalOpen] = useState(false);
  const [brokerStep, setBrokerStep] = useState(0);
  const [broker, setBroker] = useState<BrokerType>("hsbc_hk");
  const [hsbcFiles, setHsbcFiles] = useState<string[]>([]);
  const [ordinaryFiles, setOrdinaryFiles] = useState<string[]>([]);
  const [creditFiles, setCreditFiles] = useState<string[]>([]);
  const [supplementFiles, setSupplementFiles] = useState<string[]>([]);
  const [brokerConverting, setBrokerConverting] = useState(false);

  // Export state
  const [exportFilters, setExportFilters] = useState<ExportFilters>({});
  const [exportType, setExportType] = useState<DataType>("holdings");

  useEffect(() => {
    fetchAccounts();
  }, [fetchAccounts]);

  const handleDownloadTemplate = async () => {
    try {
      let content: string;
      if (dataType === "options") {
        content = "股票,交易时间,交割时间,操作,股票数量,价格,金额,佣金,费用,代码\nAAPL 16JAN26 200 C,2025-01-15 10:30:00,2025/1/16,SELL,-10,5.50,5500.00,10.00,0.00,O\n";
      } else {
        content = await invoke<string>("get_import_template", { dataType });
      }
      const blob = new Blob(["﻿" + content], { type: "text/csv;charset=utf-8;" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${dataType}_template.csv`;
      a.click();
      URL.revokeObjectURL(url);
    } catch (err) {
      message.error("下载模板失败: " + String(err));
    }
  };

  const handleFileUpload = (file: File) => {
    const reader = new FileReader();
    reader.onload = async (e) => {
      const content = e.target?.result as string;
      setRawCsvContent(content);
      setLoading(true);
      try {
        if (dataType === "options") {
          const result = await invoke<ImportPreview>("parse_options_csv", {
            csvContent: content,
          });
          setPreview(result);
        } else {
          const result = await invoke<ImportPreview>("parse_import_csv", {
            content,
            dataType,
          });
          setPreview(result);
        }
        setCurrentStep(1);
      } catch (err) {
        message.error("解析文件失败: " + String(err));
      } finally {
        setLoading(false);
      }
    };
    reader.readAsText(file, "UTF-8");
    return false;
  };

  const closeBrokerModal = () => {
    if (brokerConverting) return;
    setBrokerModalOpen(false);
    setBrokerStep(0);
  };

  const handleBrokerConvert = async () => {
    if (broker === "hsbc_hk" && hsbcFiles.length === 0) {
      message.warning("请至少选择一份汇丰电子结单");
      return;
    }
    if (broker === "everbright" && ordinaryFiles.length + creditFiles.length === 0) {
      message.warning("请至少选择一份普通账户或信用账户主对账单");
      return;
    }
    if (broker === "everbright" && supplementFiles.length > 0 && ordinaryFiles.length === 0) {
      message.warning("上传普通账户补充记录时，还需选择普通账户主对账单");
      return;
    }

    setBrokerConverting(true);
    try {
      const content = await invoke<string>("convert_broker_statements", {
        broker,
        ordinaryFiles,
        creditFiles,
        supplementFiles,
        hsbcFiles,
      });
      const result = await invoke<ImportPreview>("parse_import_csv", {
        content,
        dataType: "transactions",
      });
      setDataType("transactions");
      setRawCsvContent(content);
      setPreview(result);
      setSelectedAccountId("");
      setCurrentStep(1);
      setBrokerModalOpen(false);
      setBrokerStep(0);
      message.success(`已识别 ${result.valid_rows} 条交易，请预览后确认导入`);
    } catch (err) {
      message.error("转换券商文件失败: " + String(err));
    } finally {
      setBrokerConverting(false);
    }
  };

  const handleConfirmImport = async () => {
    if (!selectedAccountId) {
      message.warning("请先选择账户");
      return;
    }
    if (!preview) return;
    setLoading(true);
    try {
      if (dataType === "options") {
        const result = await invoke<ImportOptionsResult>("import_options_csv", {
          accountId: selectedAccountId,
          csvContent: rawCsvContent,
        });
        setOptionsImportResult(result);
        setCurrentStep(2);
        message.success(`成功导入 ${result.imported} 条记录`);
      } else {
        const result = await invoke<ImportResult>("confirm_import", {
          importData: {
            data_type: dataType,
            rows: preview.preview_data,
            column_mapping: preview.column_mapping,
            account_id: selectedAccountId,
          },
        });
        setImportResult(result);
        setCurrentStep(2);
        message.success(`成功导入 ${result.imported_count} 条记录`);
      }
    } catch (err) {
      message.error("导入失败: " + String(err));
    } finally {
      setLoading(false);
    }
  };

  const handleExport = async () => {
    setLoading(true);
    try {
      let content = "";
      if (exportType === "options") {
        if (!exportFilters.account_id) {
          message.warning("导出期权数据需要选择账户");
          setLoading(false);
          return;
        }
        content = await invoke<string>("export_options_csv", {
          accountId: exportFilters.account_id,
        });
      } else if (exportType === "holdings") {
        content = await invoke<string>("export_holdings_csv", { filters: exportFilters });
      } else {
        content = await invoke<string>("export_transactions_csv", {
          startDate: "",
          endDate: "",
          filters: exportFilters,
        });
      }
      const blob = new Blob(["﻿" + content], { type: "text/csv;charset=utf-8;" });
      const url = URL.createObjectURL(blob);
      const a = document.createElement("a");
      a.href = url;
      const date = new Date().toISOString().slice(0, 10).replace(/-/g, "");
      a.download = `${exportType}_${date}.csv`;
      a.click();
      URL.revokeObjectURL(url);
      message.success("导出成功");
    } catch (err) {
      message.error("导出失败: " + String(err));
    } finally {
      setLoading(false);
    }
  };

  const handleReset = () => {
    setCurrentStep(0);
    setPreview(null);
    setImportResult(null);
    setOptionsImportResult(null);
    setRawCsvContent("");
  };

  const previewColumns =
    preview && preview.preview_data.length > 0
      ? Object.keys(preview.preview_data[0]).map((key) => ({
          title: key,
          dataIndex: key,
          key,
          ellipsis: true,
        }))
      : [];

  const showExportAccountFilter = exportType !== "options";

  return (
    <div className="space-y-6">
      <Title level={2}>
        <ImportOutlined style={{ color: "#13c2c2" }} /> 数据导入导出
      </Title>

      {/* Export Section */}
      <Card title={<><DownloadOutlined /> 数据导出</>}>
        <Space wrap>
          <Select
            value={exportType}
            onChange={setExportType}
            style={{ width: 160 }}
            options={[
              { value: "holdings", label: "持仓数据" },
              { value: "transactions", label: "交易记录" },
              { value: "options", label: "期权记录" },
            ]}
          />
          {showExportAccountFilter && (
            <Select
              placeholder="按市场筛选"
              allowClear
              style={{ width: 140 }}
              onChange={(v) => setExportFilters((f) => ({ ...f, market: v }))}
              options={[
                { value: "US", label: "美股 (US)" },
                { value: "CN", label: "A股 (CN)" },
                { value: "HK", label: "港股 (HK)" },
              ]}
            />
          )}
          <Select
            placeholder="按账户筛选"
            allowClear
            style={{ width: 160 }}
            onChange={(v) => setExportFilters((f) => ({ ...f, account_id: v }))}
            value={exportFilters.account_id || undefined}
            options={accounts.map((a) => ({ value: a.id, label: a.name }))}
          />
          <Button
            type="primary"
            icon={<DownloadOutlined />}
            loading={loading}
            onClick={handleExport}
          >
            导出 CSV
          </Button>
        </Space>
        {exportType === "options" && (
          <Text type="secondary" style={{ display: "block", marginTop: 8 }}>
            期权记录导出须选择账户。导出格式与导入格式一致，可重新导入。
          </Text>
        )}
      </Card>

      <Divider />

      {/* Import Section */}
      <Card title={<><UploadOutlined /> 数据导入</>}>
        <Steps
          current={currentStep}
          style={{ marginBottom: 24 }}
          items={[
            { title: "上传文件" },
            { title: "预览确认" },
            { title: "导入完成" },
          ]}
        />

        {currentStep === 0 && (
          <Space orientation="vertical" style={{ width: "100%" }}>
            <Button
              icon={<BankOutlined />}
              onClick={() => setBrokerModalOpen(true)}
              style={{ alignSelf: "flex-start" }}
            >
              导入券商对账单
            </Button>
            <Space>
              <Text>数据类型：</Text>
              <Select
                value={dataType}
                onChange={(v) => setDataType(v)}
                style={{ width: 160 }}
                options={[
                  { value: "holdings", label: "持仓数据" },
                  { value: "transactions", label: "交易记录" },
                  { value: "options", label: "期权记录" },
                ]}
              />
              <Button icon={<DownloadOutlined />} onClick={handleDownloadTemplate}>
                下载模板
              </Button>
            </Space>
            <Upload.Dragger
              accept=".csv"
              beforeUpload={handleFileUpload}
              showUploadList={false}
              disabled={loading}
            >
              <p className="ant-upload-drag-icon">
                <UploadOutlined style={{ fontSize: 48 }} />
              </p>
              <p className="ant-upload-text">点击或拖拽 CSV 文件到此区域</p>
              <p className="ant-upload-hint">支持 UTF-8 编码的 CSV 文件</p>
            </Upload.Dragger>
          </Space>
        )}

        {currentStep === 1 && preview && (
          <Space orientation="vertical" style={{ width: "100%" }}>
            <Space>
              <Tag color="blue">共 {preview.total_rows} 行</Tag>
              <Tag color="green">有效 {preview.valid_rows} 行</Tag>
              {preview.error_rows.length > 0 && (
                <Tag color="red">错误 {preview.error_rows.length} 行</Tag>
              )}
            </Space>

            {preview.error_rows.length > 0 && (
              <Alert
                type="warning"
                title="发现数据错误（错误行将被跳过）"
                description={preview.error_rows
                  .slice(0, 5)
                  .map((e) => e.message)
                  .join("\n")}
                style={{ whiteSpace: "pre-line" }}
              />
            )}

            <Form layout="inline">
              <Form.Item label="导入到账户" required>
                <Select
                  placeholder="请选择账户"
                  style={{ width: 200 }}
                  value={selectedAccountId || undefined}
                  onChange={setSelectedAccountId}
                  options={accounts.map((a) => ({ value: a.id, label: a.name }))}
                />
              </Form.Item>
            </Form>

            <Table
              dataSource={preview.preview_data.slice(0, 10)}
              columns={previewColumns}
              rowKey={(_, i) => String(i)}
              size="small"
              scroll={{ x: "max-content" }}
              pagination={false}
            />

            <Space>
              <Button onClick={() => setCurrentStep(0)}>返回</Button>
              <Button
                type="primary"
                icon={<CheckCircleOutlined />}
                loading={loading}
                onClick={handleConfirmImport}
                disabled={!selectedAccountId}
              >
                确认导入
              </Button>
            </Space>
          </Space>
        )}

        {currentStep === 2 && (
          <Space orientation="vertical" style={{ width: "100%" }}>
            {importResult && (
              <Alert
                type="success"
                title="导入完成"
                description={
                  <ul>
                    <li>成功导入：{importResult.imported_count} 条</li>
                    <li>跳过：{importResult.skipped_count} 条</li>
                    {importResult.errors.length > 0 && (
                      <li>错误：{importResult.errors.length} 条</li>
                    )}
                  </ul>
                }
                icon={<CheckCircleOutlined />}
              />
            )}
            {optionsImportResult && (
              <Alert
                type="success"
                title="导入完成"
                description={
                  <ul>
                    <li>成功导入：{optionsImportResult.imported} 条</li>
                    <li>跳过：{optionsImportResult.skipped} 条</li>
                    {optionsImportResult.errors.length > 0 && (
                      <li>错误：{optionsImportResult.errors.length} 条</li>
                    )}
                  </ul>
                }
                icon={<CheckCircleOutlined />}
              />
            )}

            <Button onClick={handleReset}>
              继续导入
            </Button>
          </Space>
        )}
      </Card>

      <Modal
        open={brokerModalOpen}
        title="导入券商对账单"
        width={720}
        maskClosable={!brokerConverting}
        closable={!brokerConverting}
        onCancel={closeBrokerModal}
        footer={
          brokerStep === 0
            ? [
                <Button key="cancel" onClick={closeBrokerModal}>取消</Button>,
                <Button key="next" type="primary" onClick={() => setBrokerStep(1)}>下一步</Button>,
              ]
            : [
                <Button key="back" disabled={brokerConverting} onClick={() => setBrokerStep(0)}>上一步</Button>,
                <Button key="convert" type="primary" loading={brokerConverting} onClick={handleBrokerConvert}>
                  生成预览
                </Button>,
              ]
        }
      >
        <Steps
          current={brokerStep}
          size="small"
          style={{ marginBottom: 24 }}
          items={[{ title: "选择券商" }, { title: "上传文件" }]}
        />

        {brokerStep === 0 ? (
          <Form layout="vertical">
            <Form.Item label="券商">
              <Select<BrokerType>
                value={broker}
                onChange={setBroker}
                options={[
                  { value: "hsbc_hk", label: "香港汇丰" },
                  { value: "everbright", label: "光大证券" },
                ]}
              />
            </Form.Item>
            <Alert
              type="info"
              showIcon
              title={broker === "hsbc_hk" ? "支持汇丰投资服务综合结单（PDF）" : "支持普通账户、信用账户对账单及普通账户补充记录"}
            />
          </Form>
        ) : (
          <Space orientation="vertical" size={12} style={{ width: "100%" }}>
            {broker === "hsbc_hk" ? (
              <BrokerFileBox
                title="汇丰电子结单"
                description="PDF 格式，可一次选择多份；重叠结单中的同一笔交易会自动去重"
                files={hsbcFiles}
                accept="pdf"
                onChange={setHsbcFiles}
              />
            ) : (
              <>
                <Alert
                  type="info"
                  showIcon
                  title="普通账户与信用账户主对账单可任选一类，至少选择 1 个文件；补充记录不能单独导入。"
                />
                <BrokerFileBox
                  title="普通账户主对账单"
                  description="券商导出的 .xls 对账单，可多选"
                  files={ordinaryFiles}
                  accept="xls"
                  onChange={setOrdinaryFiles}
                />
                <BrokerFileBox
                  title="信用账户主对账单"
                  description="券商导出的 .xls 对账单，可多选"
                  files={creditFiles}
                  accept="xls"
                  onChange={setCreditFiles}
                />
                <BrokerFileBox
                  title="普通账户补充记录（可选）"
                  description="选择补充记录时，必须同时选择普通账户主对账单"
                  files={supplementFiles}
                  accept="xls"
                  onChange={setSupplementFiles}
                />
              </>
            )}
          </Space>
        )}
      </Modal>
    </div>
  );
}
