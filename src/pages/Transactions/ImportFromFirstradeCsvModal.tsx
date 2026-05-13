import { useState, useCallback } from "react";
import {
  Modal,
  Steps,
  Button,
  Upload,
  Table,
  Select,
  Input,
  InputNumber,
  DatePicker,
  Alert,
  Typography,
  message,
  Tag,
  Spin,
} from "antd";
import { InboxOutlined, CheckCircleOutlined, CloseCircleOutlined } from "@ant-design/icons";
import type { UploadFile } from "antd/es/upload";
import dayjs from "dayjs";
import { invoke } from "@tauri-apps/api/core";
import type { Account } from "../../types";

const { Dragger } = Upload;
const { Text } = Typography;

function shareInputProps() {
  return { min: 0.000001, precision: 6 };
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface EditableRow {
  key: string;
  selected: boolean;
  transaction_type: string; // "BUY" | "SELL"
  stock_name: string;
  symbol: string;
  traded_at: string; // ISO-8601
  price: number;
  shares: number;
  total_amount: number;
  commission: number;
  lookingUp?: boolean;
  importOk?: boolean;
  importError?: string;
}

interface ImportResult {
  success: number;
  failed: number;
  errors: { name: string; error: string }[];
}

interface ImportFromFirstradeCsvModalProps {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

// ---------------------------------------------------------------------------
// CSV parsing helpers
// ---------------------------------------------------------------------------

function parseNum(s: string | undefined): number {
  return parseFloat((s ?? "").replace(/,/g, ""));
}

/**
 * Split a single CSV line respecting double-quoted fields.
 */
function splitCsvLine(line: string): string[] {
  const result: string[] = [];
  let current = "";
  let inQuotes = false;

  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === '"') {
      if (inQuotes && line[i + 1] === '"') {
        current += '"';
        i++;
      } else {
        inQuotes = !inQuotes;
      }
    } else if (ch === "," && !inQuotes) {
      result.push(current);
      current = "";
    } else {
      current += ch;
    }
  }
  result.push(current);
  return result;
}

/**
 * Parse a Firstrade trade-history CSV export.
 *
 * Expected columns (flat header row):
 *   Symbol, Quantity, Price, Action, Description, TradeDate, SettledDate,
 *   Interest, Amount, Commission, Fee, CUSIP, RecordType
 *
 * Only rows where Action is "BUY" or "SELL" are imported.
 * Commission = Commission + Fee (both are separate columns in the Firstrade CSV).
 * Total amount = |Amount| (Amount is negative for buys, positive for sells).
 */
function parseFirstradeCsv(text: string): EditableRow[] {
  // Strip UTF-8 BOM if present
  const stripped = text.startsWith("\uFEFF") ? text.slice(1) : text;
  const lines = stripped.split(/\r?\n/);

  // Locate header row: must contain "Symbol" and "Action"
  let headerIdx = -1;
  for (let i = 0; i < lines.length; i++) {
    const cols = splitCsvLine(lines[i]).map((c) => c.trim());
    if (cols.includes("Symbol") && cols.includes("Action")) {
      headerIdx = i;
      break;
    }
  }
  if (headerIdx === -1) return [];

  const headerCols = splitCsvLine(lines[headerIdx]).map((c) => c.trim());
  const col = (name: string) => headerCols.indexOf(name);

  const iSymbol     = col("Symbol");
  const iQuantity   = col("Quantity");
  const iPrice      = col("Price");
  const iAction     = col("Action");
  const iTradeDate  = col("TradeDate");
  const iAmount     = col("Amount");
  const iCommission = col("Commission");
  const iFee        = col("Fee");

  if (iSymbol === -1 || iAction === -1 || iQuantity === -1 || iPrice === -1) return [];

  const rows: EditableRow[] = [];

  for (let i = headerIdx + 1; i < lines.length; i++) {
    const line = lines[i];
    if (!line.trim()) continue;

    const cols = splitCsvLine(line);

    const action = (cols[iAction] ?? "").trim().toUpperCase();
    if (action !== "BUY" && action !== "SELL") continue;

    const symbol = (cols[iSymbol] ?? "").trim().toUpperCase();
    if (!symbol) continue;

    const quantity = parseNum(cols[iQuantity]);
    const price    = parseNum(cols[iPrice]);

    if (isNaN(quantity) || isNaN(price) || price <= 0) continue;

    const shares = Math.abs(quantity);
    if (shares === 0) continue;

    const amount     = parseNum(iAmount     !== -1 ? cols[iAmount]     : undefined);
    const commRaw    = parseNum(iCommission !== -1 ? cols[iCommission] : undefined);
    const feeRaw     = parseNum(iFee        !== -1 ? cols[iFee]        : undefined);

    const total_amount = Math.abs(isNaN(amount) ? price * shares : amount);
    const commission   =
      (isNaN(commRaw) ? 0 : Math.abs(commRaw)) +
      (isNaN(feeRaw)  ? 0 : Math.abs(feeRaw));

    const tradeDateStr = (iTradeDate !== -1 ? cols[iTradeDate] ?? "" : "").trim();
    const traded_at    = parseFirstradeDate(tradeDateStr);

    rows.push({
      key: String(i),
      selected: true,
      transaction_type: action,
      stock_name: symbol, // will be resolved to full name later
      symbol,
      traded_at,
      price: Math.abs(price),
      shares,
      total_amount,
      commission,
    });
  }

  return rows;
}

/**
 * Parse Firstrade date strings.
 * Observed format: "2026/3/31" (YYYY/M/DD or YYYY/MM/DD)
 */
function parseFirstradeDate(raw: string): string {
  const cleaned = raw.trim();

  // "YYYY/M/D" or "YYYY/MM/DD"
  const m = cleaned.match(/^(\d{4})\/(\d{1,2})\/(\d{1,2})$/);
  if (m) {
    const d = dayjs(`${m[1]}-${m[2].padStart(2, "0")}-${m[3].padStart(2, "0")}`);
    if (d.isValid()) return d.hour(10).minute(30).second(0).format("YYYY-MM-DDTHH:mm:ss");
  }

  // Fallback: let dayjs try
  const d = dayjs(cleaned);
  return d.isValid() ? d.hour(10).minute(30).second(0).format("YYYY-MM-DDTHH:mm:ss") : "";
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function ImportFromFirstradeCsvModal({
  open,
  account,
  onClose,
  onImported,
}: ImportFromFirstradeCsvModalProps) {
  const [step, setStep] = useState(0);
  const [fileList, setFileList] = useState<UploadFile[]>([]);
  const [rows, setRows] = useState<EditableRow[]>([]);
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);
  const [parseError, setParseError] = useState<string>("");

  const market = "US";
  const currency = "USD";

  // ---- Row update helper ----------------------------------------------------

  const updateRow = useCallback(
    (key: string, patch: Partial<EditableRow>) => {
      setRows((prev) => prev.map((r) => (r.key === key ? { ...r, ...patch } : r)));
    },
    []
  );

  // ---- Name resolution -------------------------------------------------------

  const resolveStockNames = useCallback(async (parsedRows: EditableRow[]) => {
    // 1. Build symbol→name map from existing holdings (all accounts)
    const holdingNameMap = new Map<string, string>();
    try {
      const holdings = await invoke<{ symbol: string; name: string }[]>("get_holdings", {
        accountId: null,
      });
      for (const h of holdings) {
        holdingNameMap.set(h.symbol.toUpperCase(), h.name);
      }
    } catch {
      // ignore — will fall back to lookup for all symbols
    }

    const uniqueSymbols = [...new Set(parsedRows.map((r) => r.symbol.toUpperCase()))];
    const symbolNameMap = new Map<string, string>();

    for (const sym of uniqueSymbols) {
      const name = holdingNameMap.get(sym);
      if (name) symbolNameMap.set(sym, name);
    }

    const needLookup = uniqueSymbols.filter((s) => !symbolNameMap.has(s));
    await Promise.all(
      needLookup.map(async (sym) => {
        try {
          const name = await invoke<string | null>("lookup_stock_name_by_symbol", { symbol: sym });
          if (name) symbolNameMap.set(sym, name);
        } catch {
          // ignore individual lookup failures; user can edit manually
        }
      })
    );

    setRows((prev) =>
      prev.map((r) => {
        const resolved = symbolNameMap.get(r.symbol.toUpperCase());
        return { ...r, stock_name: resolved ?? r.stock_name, lookingUp: false };
      })
    );
  }, []);

  // ---- Upload handler --------------------------------------------------------

  const handleBeforeUpload = useCallback(
    (file: File) => {
      setParseError("");
      const reader = new FileReader();
      reader.onload = (e) => {
        const text = e.target?.result as string;
        try {
          const parsed = parseFirstradeCsv(text);
          if (parsed.length === 0) {
            setParseError(
              "未从 CSV 中识别到 BUY/SELL 交易记录。请确认文件为 Firstrade 导出的交易记录 CSV，且包含 Symbol、Action 等列标题。"
            );
            return;
          }
          const withLoading = parsed.map((r) => ({ ...r, lookingUp: true }));
          setRows(withLoading);
          setStep(1);
          resolveStockNames(withLoading);
        } catch (err) {
          setParseError(`CSV 解析失败: ${String(err)}`);
        }
      };
      reader.readAsText(file, "utf-8");
      setFileList([file as unknown as UploadFile]);
      return false; // prevent antd default upload
    },
    [resolveStockNames]
  );

  // ---- Import ----------------------------------------------------------------

  const handleImport = useCallback(async () => {
    const selected = rows.filter((r) => r.selected);
    if (selected.length === 0) {
      message.warning("请至少选择一条记录导入");
      return;
    }

    setImporting(true);
    let success = 0;
    const errors: { name: string; error: string }[] = [];

    const sorted = [...selected].sort((a, b) => a.traded_at.localeCompare(b.traded_at));

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
  }, [rows, account.id, updateRow, onImported]);

  // ---- Reset -----------------------------------------------------------------

  const handleClose = useCallback(() => {
    setStep(0);
    setFileList([]);
    setRows([]);
    setParseError("");
    setImportResult(null);
    onClose();
  }, [onClose]);

  // ---- Table columns (Step 1) ------------------------------------------------

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
          onChange={(e) => updateRow(record.key, { selected: e.target.checked })}
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
      title: "股票代码",
      key: "symbol",
      width: 110,
      render: (_: unknown, record: EditableRow) => (
        <Input
          size="small"
          value={record.symbol}
          style={{ width: 100 }}
          onChange={(e) => updateRow(record.key, { symbol: e.target.value.trim().toUpperCase() })}
        />
      ),
    },
    {
      title: "股票名称",
      key: "stock_name",
      width: 120,
      render: (_: unknown, record: EditableRow) => (
        <Spin spinning={!!record.lookingUp} size="small">
          <Input
            size="small"
            value={record.stock_name}
            style={{ width: 110 }}
            onChange={(e) => updateRow(record.key, { stock_name: e.target.value })}
          />
        </Spin>
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
              updateRow(record.key, { traded_at: v.format("YYYY-MM-DDTHH:mm:ss") });
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
          {...shareInputProps()}
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
        if (record.importOk)
          return <CheckCircleOutlined style={{ color: "#52c41a" }} />;
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

  // ---- Footer ----------------------------------------------------------------

  const footer = (() => {
    if (step === 0) {
      return [
        <Button key="cancel" onClick={handleClose}>
          取消
        </Button>,
      ];
    }
    if (step === 1) {
      return [
        <Button
          key="back"
          onClick={() => { setStep(0); setFileList([]); setRows([]); setParseError(""); }}
        >
          返回
        </Button>,
        <Button key="import" type="primary" loading={importing} onClick={handleImport}>
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

  // ---- Render ----------------------------------------------------------------

  return (
    <Modal
      title="从 CSV 导入交易记录（Firstrade 美股）"
      open={open}
      onCancel={handleClose}
      footer={footer}
      width={step === 1 ? 980 : 520}
      destroyOnClose
    >
      <Steps
        current={step}
        items={[
          { title: "上传 CSV" },
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
            accept=".csv,text/csv"
            maxCount={1}
            showUploadList={false}
          >
            <p className="ant-upload-drag-icon">
              <InboxOutlined />
            </p>
            <p className="ant-upload-text">点击或拖拽 Firstrade 交易记录 CSV 到此处</p>
            <p className="ant-upload-hint">
              在 Firstrade 网站 → History → Transaction History → Download CSV
            </p>
          </Dragger>
          {parseError && (
            <Alert type="error" message={parseError} className="mt-3" showIcon />
          )}
          <Alert
            type="info"
            className="mt-3"
            showIcon
            message="仅支持 Firstrade 交易记录 CSV 格式"
            description={
              <span>
                仅导入 <strong>BUY</strong>/<strong>SELL</strong> 类型的交易记录，
                其他类型（利息、分红等）将自动忽略。
                手续费 = Commission + Fee。
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
            message={`账户「${account.name}」[US]，市场和货币已自动设置为 US / USD`}
            className="mb-3"
          />
          <Alert
            type="warning"
            showIcon
            message="请核对以下数据，可直接在表格中编辑。确认后点击「导入选中记录」。"
            className="mb-3"
          />
          <Table
            dataSource={rows}
            columns={columns}
            rowKey="key"
            size="small"
            pagination={false}
            scroll={{ x: 950, y: 380 }}
          />
          <Text type="secondary" className="mt-2 block">
            共解析 {rows.length} 条记录，已勾选 {rows.filter((r) => r.selected).length} 条
          </Text>
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
                  {importResult.errors.map((e, idx) => (
                    <li key={idx}>
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
