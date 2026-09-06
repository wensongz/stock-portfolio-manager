import { useState, useEffect, useRef } from "react";
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
} from "antd";
import {
  UploadOutlined,
  DownloadOutlined,
  CheckCircleOutlined,
  ImportOutlined,
} from "@ant-design/icons";
import { invoke } from "@tauri-apps/api/core";
import type { ImportPreview, ExportFilters, ImportOptionsResult } from "../../types";
import ImportBatchPanel from "../../features/imports/ImportBatchPanel";
import ImportBatchHistory from "../../features/imports/ImportBatchHistory";
import type { ImportBatch } from "../../features/imports/batchTypes";
import { useAccountStore } from "../../stores/accountStore";

const { Title, Text } = Typography;

export default function ImportPage() {
  const { accounts, fetchAccounts } = useAccountStore();
  type DataType = "holdings" | "transactions" | "options";
  const [currentStep, setCurrentStep] = useState(0);
  const [dataType, setDataType] = useState<DataType>("holdings");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [batch, setBatch] = useState<ImportBatch | null>(null);
  const [batchBusy, setBatchBusy] = useState(false);
  const [historyRefresh, setHistoryRefresh] = useState(0);
  const [fileName, setFileName] = useState("");
  const previewRequest = useRef<{ fingerprint: string; id: string } | null>(null);
  const [optionsImportResult, setOptionsImportResult] = useState<ImportOptionsResult | null>(null);
  const [selectedAccountId, setSelectedAccountId] = useState("");
  const [loading, setLoading] = useState(false);
  const [rawCsvContent, setRawCsvContent] = useState("");

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
    setLoading(true);
    const reader = new FileReader();
    reader.onload = async (e) => {
      const content = e.target?.result as string;
      setRawCsvContent(content);
      setFileName(file.name);
      previewRequest.current = null;
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
    reader.onerror = () => {
      setLoading(false);
      message.error("读取文件失败，请重新选择文件");
    };
    reader.readAsText(file, "UTF-8");
    return false;
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
        const fingerprint = JSON.stringify([rawCsvContent, dataType, selectedAccountId, fileName]);
        if (previewRequest.current?.fingerprint !== fingerprint) {
          previewRequest.current = { fingerprint, id: crypto.randomUUID() };
        }
        const result = await invoke<ImportBatch>("preview_csv_import_batch", {
          content: rawCsvContent,
          dataType,
          accountId: selectedAccountId,
          fileName,
          requestId: previewRequest.current.id,
        });
        setBatch(result);
        setCurrentStep(2);
        setHistoryRefresh((value) => value + 1);
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
    setBatch(null);
    setFileName("");
    previewRequest.current = null;
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
            { title: dataType === "options" ? "导入完成" : "批次导入与核对" },
          ]}
        />

        {currentStep === 0 && (
          <Space orientation="vertical" style={{ width: "100%" }}>
            <Space>
              <Text>数据类型：</Text>
              <Select
                value={dataType}
                disabled={loading}
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
                  disabled={loading}
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
              <Button disabled={loading} onClick={() => setCurrentStep(0)}>返回</Button>
              <Button
                type="primary"
                icon={<CheckCircleOutlined />}
                loading={loading}
                onClick={handleConfirmImport}
                disabled={!selectedAccountId || preview.valid_rows === 0}
              >
                {dataType === "options" ? "确认导入" : "创建批次并选择导入行"}
              </Button>
            </Space>
          </Space>
        )}

        {currentStep === 2 && (
          <Space orientation="vertical" style={{ width: "100%" }}>
            {batch && <ImportBatchPanel batch={batch} onChange={(updated) => {
              setBatch(updated);
              setHistoryRefresh((value) => value + 1);
            }} onBusyChange={setBatchBusy} />}
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

            <Button disabled={batchBusy || loading} onClick={handleReset}>
              继续导入
            </Button>
          </Space>
        )}
      </Card>
      <ImportBatchHistory refreshKey={historyRefresh} />
    </div>
  );
}
