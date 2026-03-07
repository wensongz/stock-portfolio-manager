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
} from "antd";
import {
  UploadOutlined,
  DownloadOutlined,
  CheckCircleOutlined,
} from "@ant-design/icons";
import { invoke } from "@tauri-apps/api/core";
import type { ImportPreview, ImportResult, ExportFilters } from "../../types";
import { useAccountStore } from "../../stores/accountStore";

const { Title, Text } = Typography;

export default function ImportPage() {
  const { accounts, fetchAccounts } = useAccountStore();
  const [currentStep, setCurrentStep] = useState(0);
  const [dataType, setDataType] = useState<"holdings" | "transactions">("holdings");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);
  const [selectedAccountId, setSelectedAccountId] = useState("");
  const [loading, setLoading] = useState(false);

  // Export state
  const [exportFilters, setExportFilters] = useState<ExportFilters>({});
  const [exportType, setExportType] = useState<"holdings" | "transactions">("holdings");

  useEffect(() => {
    fetchAccounts();
  }, [fetchAccounts]);

  const handleDownloadTemplate = async () => {
    try {
      const content = await invoke<string>("get_import_template", { dataType });
      const blob = new Blob(["\uFEFF" + content], { type: "text/csv;charset=utf-8;" });
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
      setLoading(true);
      try {
        const result = await invoke<ImportPreview>("parse_import_csv", {
          content,
          dataType,
        });
        setPreview(result);
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

  const handleConfirmImport = async () => {
    if (!selectedAccountId) {
      message.warning("请先选择账户");
      return;
    }
    if (!preview) return;
    setLoading(true);
    try {
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
      if (exportType === "holdings") {
        content = await invoke<string>("export_holdings_csv", { filters: exportFilters });
      } else {
        content = await invoke<string>("export_transactions_csv", {
          startDate: "",
          endDate: "",
          filters: exportFilters,
        });
      }
      const blob = new Blob(["\uFEFF" + content], { type: "text/csv;charset=utf-8;" });
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

  return (
    <div className="space-y-6">
      <Title level={2}>数据导入导出</Title>

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
            ]}
          />
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
          <Select
            placeholder="按账户筛选"
            allowClear
            style={{ width: 160 }}
            onChange={(v) => setExportFilters((f) => ({ ...f, account_id: v }))}
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
      </Card>

      <Divider />

      {/* Import Section */}
      <Card title={<><UploadOutlined /> 数据导入</>}>
        <Steps current={currentStep} style={{ marginBottom: 24 }}>
          <Steps.Step title="上传文件" />
          <Steps.Step title="预览确认" />
          <Steps.Step title="导入完成" />
        </Steps>

        {currentStep === 0 && (
          <Space direction="vertical" style={{ width: "100%" }}>
            <Space>
              <Text>数据类型：</Text>
              <Select
                value={dataType}
                onChange={(v) => setDataType(v)}
                style={{ width: 160 }}
                options={[
                  { value: "holdings", label: "持仓数据" },
                  { value: "transactions", label: "交易记录" },
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
          <Space direction="vertical" style={{ width: "100%" }}>
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
                message="发现数据错误（错误行将被跳过）"
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

        {currentStep === 2 && importResult && (
          <Space direction="vertical" style={{ width: "100%" }}>
            <Alert
              type="success"
              message="导入完成"
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
            <Button onClick={handleReset}>
              继续导入
            </Button>
          </Space>
        )}
      </Card>
    </div>
  );
}
