import { useState, useCallback } from "react";
import {
  Modal,
  Steps,
  Button,
  Upload,
  Table,
  Space,
  Select,
  InputNumber,
  DatePicker,
  Spin,
  Alert,
  Typography,
  message,
  Tag,
} from "antd";
import {
  InboxOutlined,
  SearchOutlined,
  CheckCircleOutlined,
  CloseCircleOutlined,
} from "@ant-design/icons";
import type { UploadFile } from "antd/es/upload";
import dayjs from "dayjs";
import { invoke } from "@tauri-apps/api/core";
import type { Account, Market, Currency } from "../../types";

const { Dragger } = Upload;
const { Text } = Typography;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface ParsedTradeRow {
  transaction_type: string; // "BUY" | "SELL"
  stock_name: string;
  traded_at: string; // ISO-8601
  price: number;
  shares: number;
  total_amount: number;
  commission: number;
}

interface EditableRow extends ParsedTradeRow {
  key: string;
  symbol: string;
  selected: boolean;
  lookingUp: boolean;
  importError?: string;
  importOk?: boolean;
}

interface ImportResult {
  success: number;
  failed: number;
  errors: { name: string; error: string }[];
}

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface ImportFromImageModalProps {
  open: boolean;
  account: Account;
  onClose: () => void;
  /** Called after import completes so the caller can refresh the list */
  onImported: () => void;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function ImportFromImageModal({
  open,
  account,
  onClose,
  onImported,
}: ImportFromImageModalProps) {
  const [step, setStep] = useState(0);
  const [fileList, setFileList] = useState<UploadFile[]>([]);
  const [imageBase64, setImageBase64] = useState<string>("");
  const [previewUrl, setPreviewUrl] = useState<string>("");
  const [parsing, setParsing] = useState(false);
  const [rows, setRows] = useState<EditableRow[]>([]);
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);
  const [parseError, setParseError] = useState<string>("");

  const market: Market = account.market as Market;
  const currency: Currency =
    market === "CN" ? "CNY" : market === "HK" ? "HKD" : "USD";

  // ---- Step 0 helpers -------------------------------------------------------

  const handleBeforeUpload = useCallback(
    (file: File) => {
      const reader = new FileReader();
      reader.onload = (e) => {
        const dataUrl = e.target?.result as string;
        setPreviewUrl(dataUrl);
        // Strip the data:...;base64, prefix before sending to Rust
        const b64 = dataUrl.split("base64,")[1] ?? dataUrl;
        setImageBase64(b64);
      };
      reader.readAsDataURL(file);
      setFileList([file as unknown as UploadFile]);
      return false; // prevent antd from uploading
    },
    []
  );

  const handleRecognise = useCallback(async () => {
    if (!imageBase64) {
      message.warning("请先选择截图");
      return;
    }
    setParsing(true);
    setParseError("");
    try {
      const parsed = await invoke<ParsedTradeRow[]>("parse_trade_image", {
        imageBase64,
      });
      if (parsed.length === 0) {
        setParseError(
          "未识别到交易记录。请确认截图为同花顺交易记录页面，且图片清晰。"
        );
        return;
      }
      const editableRows: EditableRow[] = parsed.map((r, idx) => ({
        ...r,
        key: String(idx),
        symbol: "",
        selected: true,
        lookingUp: false,
      }));
      setRows(editableRows);
      setStep(1);
    } catch (err) {
      setParseError(String(err));
    } finally {
      setParsing(false);
    }
  }, [imageBase64]);

  // ---- Step 1 helpers -------------------------------------------------------

  const updateRow = useCallback(
    (key: string, patch: Partial<EditableRow>) => {
      setRows((prev) =>
        prev.map((r) => (r.key === key ? { ...r, ...patch } : r))
      );
    },
    []
  );

  const handleLookup = useCallback(
    async (key: string, name: string) => {
      updateRow(key, { lookingUp: true });
      try {
        const code = await invoke<string | null>("lookup_cn_stock_code", {
          name,
        });
        updateRow(key, {
          lookingUp: false,
          symbol: code ?? "",
        });
        if (!code) {
          message.warning(`未找到「${name}」的股票代码，请手动填写`);
        }
      } catch {
        updateRow(key, { lookingUp: false });
        message.error("查询股票代码失败，请手动填写");
      }
    },
    [updateRow]
  );

  const handleLookupAll = useCallback(async () => {
    const targets = rows.filter((r) => r.selected && !r.symbol);
    for (const r of targets) {
      await handleLookup(r.key, r.stock_name);
    }
  }, [rows, handleLookup]);

  // ---- Step 2 helpers -------------------------------------------------------

  const handleImport = useCallback(async () => {
    const selected = rows.filter((r) => r.selected);
    if (selected.length === 0) {
      message.warning("请至少选择一条记录导入");
      return;
    }
    const missing = selected.filter((r) => !r.symbol.trim());
    if (missing.length > 0) {
      message.error(
        `以下股票缺少代码，请补全后再导入：${missing
          .map((r) => r.stock_name)
          .join("、")}`
      );
      return;
    }

    setImporting(true);
    let success = 0;
    const errors: { name: string; error: string }[] = [];

    // Sort chronologically before importing
    const sorted = [...selected].sort((a, b) =>
      a.traded_at.localeCompare(b.traded_at)
    );

    for (const r of sorted) {
      try {
        await invoke("create_transaction", {
          accountId: account.id,
          symbol: r.symbol.trim(),
          name: r.stock_name,
          market,
          transactionType: r.transaction_type,
          shares: r.shares,
          price: r.price,
          totalAmount: r.total_amount,
          commission: r.commission,
          currency,
          tradedAt: new Date(r.traded_at).toISOString(),
        });
        success++;
        updateRow(r.key, { importOk: true, importError: undefined });
      } catch (err) {
        const msg = String(err);
        errors.push({ name: r.stock_name, error: msg });
        updateRow(r.key, { importError: msg, importOk: false });
      }
    }

    setImportResult({ success, failed: errors.length, errors });
    setImporting(false);
    setStep(2);

    if (success > 0) {
      onImported();
    }
  }, [rows, account.id, market, currency, updateRow, onImported]);

  // ---- Reset ----------------------------------------------------------------

  const handleClose = useCallback(() => {
    setStep(0);
    setFileList([]);
    setImageBase64("");
    setPreviewUrl("");
    setRows([]);
    setParseError("");
    setImportResult(null);
    onClose();
  }, [onClose]);

  // ---- Table columns (Step 1) -----------------------------------------------

  const columns = [
    {
      title: "",
      dataIndex: "selected",
      key: "selected",
      width: 40,
      render: (_: unknown, record: EditableRow) => (
        <input
          type="checkbox"
          checked={record.selected}
          onChange={(e) =>
            updateRow(record.key, { selected: e.target.checked })
          }
        />
      ),
    },
    {
      title: "类型",
      dataIndex: "transaction_type",
      key: "type",
      width: 80,
      render: (_: unknown, record: EditableRow) => (
        <Select
          size="small"
          value={record.transaction_type}
          onChange={(v) => updateRow(record.key, { transaction_type: v })}
          style={{ width: 70 }}
        >
          <Select.Option value="BUY">
            <Tag color="green">买入</Tag>
          </Select.Option>
          <Select.Option value="SELL">
            <Tag color="red">卖出</Tag>
          </Select.Option>
        </Select>
      ),
    },
    {
      title: "股票名称",
      dataIndex: "stock_name",
      key: "name",
      width: 100,
    },
    {
      title: "股票代码",
      key: "symbol",
      width: 130,
      render: (_: unknown, record: EditableRow) => (
        <Space size={4}>
          <input
            style={{
              width: 72,
              border: "1px solid #d9d9d9",
              borderRadius: 4,
              padding: "2px 6px",
            }}
            value={record.symbol}
            placeholder="000001"
            onChange={(e) =>
              updateRow(record.key, { symbol: e.target.value.trim() })
            }
          />
          <Button
            size="small"
            icon={
              record.lookingUp ? (
                <Spin size="small" />
              ) : (
                <SearchOutlined />
              )
            }
            onClick={() => handleLookup(record.key, record.stock_name)}
            disabled={record.lookingUp}
          />
        </Space>
      ),
    },
    {
      title: "成交时间",
      key: "traded_at",
      width: 175,
      render: (_: unknown, record: EditableRow) => (
        <DatePicker
          size="small"
          showTime
          value={dayjs(record.traded_at)}
          onChange={(v) => {
            if (v) {
              updateRow(record.key, {
                traded_at: v.format("YYYY-MM-DDTHH:mm:ss"),
              });
            }
          }}
          style={{ width: 165 }}
        />
      ),
    },
    {
      title: "价格",
      key: "price",
      width: 90,
      render: (_: unknown, record: EditableRow) => (
        <InputNumber
          size="small"
          value={record.price}
          min={0}
          precision={4}
          onChange={(v) => updateRow(record.key, { price: v ?? 0 })}
          style={{ width: 85 }}
        />
      ),
    },
    {
      title: "数量",
      key: "shares",
      width: 90,
      render: (_: unknown, record: EditableRow) => (
        <InputNumber
          size="small"
          value={record.shares}
          min={1}
          precision={0}
          onChange={(v) => updateRow(record.key, { shares: v ?? 1 })}
          style={{ width: 85 }}
        />
      ),
    },
    {
      title: "总额",
      key: "total_amount",
      width: 100,
      render: (_: unknown, record: EditableRow) => (
        <InputNumber
          size="small"
          value={record.total_amount}
          min={0}
          precision={2}
          onChange={(v) => updateRow(record.key, { total_amount: v ?? 0 })}
          style={{ width: 95 }}
        />
      ),
    },
    {
      title: "手续费",
      key: "commission",
      width: 85,
      render: (_: unknown, record: EditableRow) => (
        <InputNumber
          size="small"
          value={record.commission}
          min={0}
          precision={2}
          onChange={(v) => updateRow(record.key, { commission: v ?? 0 })}
          style={{ width: 80 }}
        />
      ),
    },
    {
      title: "状态",
      key: "status",
      width: 40,
      render: (_: unknown, record: EditableRow) => {
        if (record.importOk) return <CheckCircleOutlined style={{ color: "#52c41a" }} />;
        if (record.importError)
          return (
            <CloseCircleOutlined
              style={{ color: "#ff4d4f" }}
              title={record.importError}
            />
          );
        return null;
      },
    },
  ];

  // ---- Render ---------------------------------------------------------------

  const footer = (() => {
    if (step === 0) {
      return [
        <Button key="cancel" onClick={handleClose}>
          取消
        </Button>,
        <Button
          key="recognise"
          type="primary"
          loading={parsing}
          disabled={!imageBase64}
          onClick={handleRecognise}
        >
          识别
        </Button>,
      ];
    }
    if (step === 1) {
      return [
        <Button key="back" onClick={() => setStep(0)}>
          返回
        </Button>,
        <Button key="lookup-all" onClick={handleLookupAll}>
          批量查询代码
        </Button>,
        <Button
          key="import"
          type="primary"
          loading={importing}
          onClick={handleImport}
        >
          导入选中记录
        </Button>,
      ];
    }
    return [
      <Button key="close" type="primary" onClick={handleClose}>
        完成
      </Button>,
    ];
  })();

  return (
    <Modal
      title="从同花顺截图导入交易记录"
      open={open}
      onCancel={handleClose}
      footer={footer}
      width={step === 1 ? 900 : 520}
      destroyOnClose
    >
      <Steps
        current={step}
        items={[
          { title: "上传截图" },
          { title: "核对数据" },
          { title: "导入结果" },
        ]}
        className="mb-4"
      />

      {/* ---- Step 0: Upload ---- */}
      {step === 0 && (
        <div>
          <Dragger
            fileList={fileList}
            beforeUpload={handleBeforeUpload}
            accept="image/*"
            maxCount={1}
            showUploadList={false}
          >
            {previewUrl ? (
              <img
                src={previewUrl}
                alt="截图预览"
                style={{
                  maxWidth: "100%",
                  maxHeight: 320,
                  objectFit: "contain",
                }}
              />
            ) : (
              <>
                <p className="ant-upload-drag-icon">
                  <InboxOutlined />
                </p>
                <p className="ant-upload-text">点击或拖拽同花顺交易记录截图到此处</p>
                <p className="ant-upload-hint">
                  支持 PNG / JPEG，仅限同花顺 APP 交易记录截图
                </p>
              </>
            )}
          </Dragger>
          {previewUrl && (
            <div className="mt-2 text-center">
              <Text type="secondary">
                已选图片。点击"识别"开始 OCR 解析。
              </Text>
            </div>
          )}
          {parseError && (
            <Alert
              type="error"
              message={parseError}
              className="mt-3"
              showIcon
            />
          )}
          <Alert
            type="info"
            className="mt-3"
            showIcon
            message="需要系统已安装 Tesseract OCR 及中文语言包"
            description={
              <span>
                macOS: <code>brew install tesseract tesseract-lang</code>
                <br />
                Ubuntu: <code>sudo apt install tesseract-ocr tesseract-ocr-chi-sim</code>
              </span>
            }
          />
        </div>
      )}

      {/* ---- Step 1: Edit ---- */}
      {step === 1 && (
        <div>
          <Alert
            type="info"
            showIcon
            message={`账户「${account.name}」[${market}]，市场和货币已自动设置为 ${market} / ${currency}`}
            className="mb-3"
          />
          <Alert
            type="warning"
            showIcon
            message="请核对以下识别结果，尤其是股票代码。查询不到时可手动填写。"
            className="mb-3"
          />
          <Table
            dataSource={rows}
            columns={columns}
            rowKey="key"
            size="small"
            pagination={false}
            scroll={{ x: 850 }}
          />
        </div>
      )}

      {/* ---- Step 2: Result ---- */}
      {step === 2 && importResult && (
        <div>
          {importResult.success > 0 && (
            <Alert
              type="success"
              showIcon
              message={`成功导入 ${importResult.success} 条交易记录`}
              className="mb-3"
            />
          )}
          {importResult.failed > 0 && (
            <Alert
              type="error"
              showIcon
              message={`${importResult.failed} 条导入失败`}
              description={
                <ul className="mt-1 pl-4">
                  {importResult.errors.map((e, i) => (
                    <li key={i}>
                      <strong>{e.name}</strong>: {e.error}
                    </li>
                  ))}
                </ul>
              }
              className="mb-3"
            />
          )}
        </div>
      )}
    </Modal>
  );
}
