import { useState, useCallback } from "react";
import {
  Modal,
  Steps,
  Button,
  Upload,
  Table,
  Space,
  Select,
  Input,
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
import ImportBatchPanel from "../../features/imports/ImportBatchPanel.tsx";
import type { ImportBatch } from "../../features/imports/batchTypes.ts";
import { batchPreviewRequest, transactionBatchData } from "../../features/imports/batchAdapters.ts";
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
  raw: ParsedTradeRow;
  symbol: string;
  selected: boolean;
  lookingUp: boolean;
  importError?: string;
  importOk?: boolean;
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
  const [batch, setBatch] = useState<ImportBatch | null>(null);
  const [requestId, setRequestId] = useState(() => crypto.randomUUID());
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
        raw: { ...r },
        key: String(idx),
        symbol: "",
        selected: true,
        lookingUp: false,
      }));
      setRequestId(crypto.randomUUID());
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
      setRequestId(crypto.randomUUID());
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
    if (targets.length === 0) return;

    // Mark all targets as "looking up" immediately so UI updates before awaiting
    targets.forEach((r) => updateRow(r.key, { lookingUp: true }));

    // Deduplicate: query each distinct stock name only once, then propagate the
    // result to all rows that share that name.
    const uniqueNames = [...new Set(targets.map((r) => r.stock_name))];
    const codeByName = new Map<string, string | null>();
    await Promise.all(
      uniqueNames.map(async (name) => {
        try {
          const code = await invoke<string | null>("lookup_cn_stock_code", { name });
          codeByName.set(name, code);
        } catch {
          codeByName.set(name, null);
        }
      })
    );

    // Apply results to all affected rows
    targets.forEach((r) => {
      const code = codeByName.get(r.stock_name) ?? null;
      updateRow(r.key, { lookingUp: false, symbol: code ?? "" });
      if (!code) {
        message.warning(`未找到「${r.stock_name}」的股票代码，请手动填写`);
      }
    });
  }, [rows, updateRow]);

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
    try {
      const request = batchPreviewRequest({ requestId, accountId: account.id,
        source: "ths-ocr", kind: "transactions", fileName: fileList[0]?.name ?? "screenshot",
        sourceContent: imageBase64, rows: [...rows].sort((a,b) => a.traded_at.localeCompare(b.traded_at)),
        toData: row => transactionBatchData(row, market) });
      setBatch(await invoke<ImportBatch>("preview_import_batch", { request }));
      setStep(2);
    } catch (error) { message.error(String(error)); }
    finally { setImporting(false); }
  }, [rows, account.id, market, requestId, fileList, imageBase64]);

  // ---- Reset ----------------------------------------------------------------

  const handleClose = useCallback(() => {
    if (importing || parsing) return;
    setStep(0);
    setFileList([]);
    setImageBase64("");
    setPreviewUrl("");
    setRows([]);
    setParseError("");
    setBatch(null);
    setRequestId(crypto.randomUUID());
    onClose();
  }, [onClose, importing, parsing]);

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
      width: 110,
      render: (_: unknown, record: EditableRow) => (
        <Input
          size="small"
          value={record.stock_name}
          style={{ width: 100 }}
          onChange={(e) =>
            updateRow(record.key, { stock_name: e.target.value.trim() })
          }
        />
      ),
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
        if (record.importOk) return <CheckCircleOutlined style={{ color: "var(--color-success)" }} />;
        if (record.importError)
          return (
            <CloseCircleOutlined
              style={{ color: "var(--color-error)" }}
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
        <Button key="cancel" disabled={parsing} onClick={handleClose}>
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
        <Button key="back" disabled={importing} onClick={() => setStep(0)}>
          返回
        </Button>,
        <Button key="lookup-all" disabled={importing} onClick={handleLookupAll}>
          批量查询代码
        </Button>,
        <Button
          key="import"
          type="primary"
          loading={importing}
          onClick={handleImport}
        >
          检查选中记录
        </Button>,
      ];
    }
    return [
      <Button key="close" disabled={importing} type="primary" onClick={handleClose}>
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
      width={step >= 1 ? 1100 : 520}
      closable={!importing && !parsing}
      maskClosable={!importing && !parsing}
      keyboard={!importing && !parsing}
      destroyOnHidden
    >
      <Steps
        current={step}
        items={[
          { title: "上传截图" },
          { title: "核对数据" },
          { title: "批次核对与导入" },
        ]}
        className="mb-4"
      />

      {/* ---- Step 0: Upload ---- */}
      {step === 0 && (
        <div>
          <Dragger
            disabled={parsing}
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
              title={parseError}
              className="mt-3"
              showIcon
            />
          )}
          <Alert
            type="info"
            className="mt-3"
            showIcon
            title="需要系统已安装 Tesseract OCR 及中文语言包"
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
        <div inert={importing}>
          <Alert
            type="info"
            showIcon
            title={`账户「${account.name}」[${market}]，市场和货币已自动设置为 ${market} / ${currency}`}
            className="mb-3"
          />
          <Alert
            type="warning"
            showIcon
            title="请核对以下识别结果，尤其是股票代码。查询不到时可手动填写。"
            className="mb-3"
          />
          <Table
            dataSource={rows}
            columns={columns}
            rowKey="key"
            size="small"
            pagination={false}
            scroll={{ x: 870, y: 380 }}
          />
        </div>
      )}

      {/* ---- Step 2: Result ---- */}
      {step === 2 && batch && (
        <ImportBatchPanel batch={batch} onChange={setBatch} onImported={onImported} onBusyChange={setImporting} />
      )}
    </Modal>
  );
}
