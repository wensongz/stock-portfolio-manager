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
import type { Account, Market, Currency } from "../../types";

const { Dragger } = Upload;
const { Text } = Typography;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface EditableRow {
  key: string;
  selected: boolean;
  transaction_type: string; // "BUY" | "SELL"
  stock_name: string;       // editable display name (defaults to symbol)
  symbol: string;
  traded_at: string;        // ISO-8601
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

// ---------------------------------------------------------------------------
// Props
// ---------------------------------------------------------------------------

interface ImportFromIbCsvModalProps {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

// ---------------------------------------------------------------------------
// CSV parsing helpers
// ---------------------------------------------------------------------------

/** IB account IDs look like U1234567 or DU1234567 (paper). Non-account values like "Stocks", "USD" fail this. */
function isValidAcctId(acctId: string): boolean {
  return /^[A-Z]{1,3}\d+$/.test(acctId.trim());
}

function parseNum(s: string | undefined): number {
  return parseFloat((s ?? "").replace(/,/g, ""));
}

/**
 * Parse IB Activity Statement or Flex Query CSV.
 *
 * Handles the following column name variants (IB export can differ by format/region):
 *   Date column   : "Trade Date/Time"  OR  "Date/Time"
 *   Price column  : "Price"  OR  "T. Price"
 *   Commission    : "Comm" + "Fee" (separate)  OR  "Comm/Fee" (combined)  OR  "Comm in USD"
 *   Direction     : "Type" column (BUY/SELL)  OR  sign of Quantity
 *   Filter        : "Acct ID" (skip non-account-number values like "Stocks", "USD")
 *
 * Two structural layouts:
 *   A) Section/Header/Data layout – rows prefixed with "Trades,Header,..." and "Trades,Data,..."
 *   B) Flat layout – standalone header row containing "Symbol"
 */
function parseIbCsv(text: string, market: Market): EditableRow[] {
  const lines = text.split(/\r?\n/);

  // --- Try layout A: look for "Trades,Header,..." row ---
  for (let i = 0; i < lines.length; i++) {
    const cols = splitCsvLine(lines[i]);
    if (cols[0].trim() === "Trades" && cols[1].trim() === "Header") {
      const rows = parseStructured(lines, i, market);
      if (rows.length > 0) return rows;
    }
  }

  // --- Fallback layout B: find any header row containing "Symbol" ---
  for (let i = 0; i < lines.length; i++) {
    const cols = splitCsvLine(lines[i]).map((c) => c.trim());
    if (cols.includes("Symbol")) {
      const rows = parseFlat(lines, i, market);
      if (rows.length > 0) return rows;
    }
  }

  return [];
}

/** Layout A parser: rows are prefixed "Trades,Data,..." */
function parseStructured(lines: string[], headerLineIdx: number, market: Market): EditableRow[] {
  const headerCols = splitCsvLine(lines[headerLineIdx]).map((c) => c.trim());
  const col = (name: string) => headerCols.indexOf(name);

  const iSymbol   = col("Symbol");
  const iDateTime = col("Trade Date/Time") !== -1 ? col("Trade Date/Time") : col("Date/Time");
  const iQuantity = col("Quantity");
  const iPrice    = col("Price") !== -1 ? col("Price") : col("T. Price");
  const iProceeds = col("Proceeds");
  const iType     = col("Type");
  const iAcctId   = col("Acct ID");
  // Commission: separate Comm+Fee, or combined Comm/Fee, or Comm in USD
  const iComm     = col("Comm");
  const iFee      = col("Fee");
  const iCommFee  = col("Comm/Fee") !== -1 ? col("Comm/Fee") : col("Comm in USD");

  if (iSymbol === -1 || iDateTime === -1 || iQuantity === -1 || iPrice === -1) return [];

  const rows: EditableRow[] = [];
  for (let i = headerLineIdx + 1; i < lines.length; i++) {
    const cols = splitCsvLine(lines[i]);
    if (cols[0].trim() !== "Trades" || cols[1].trim() !== "Data") continue;

    const symbol = (cols[iSymbol] ?? "").trim();
    if (!symbol || symbol.startsWith("Total")) continue;

    // Skip grouping rows where Acct ID is not a real account number
    if (iAcctId !== -1 && !isValidAcctId(cols[iAcctId] ?? "")) continue;

    const quantity = parseNum(cols[iQuantity]);
    const price    = parseNum(cols[iPrice]);

    if (isNaN(quantity) || isNaN(price)) continue;

    // Direction: from Type column if present, otherwise from Quantity sign
    let transaction_type: string;
    if (iType !== -1) {
      const t = (cols[iType] ?? "").trim().toUpperCase();
      transaction_type = t === "SELL" ? "SELL" : "BUY";
    } else {
      transaction_type = quantity >= 0 ? "BUY" : "SELL";
    }

    const shares       = Math.abs(quantity);
    const proceeds     = parseNum(cols[iProceeds]);
    const total_amount = Math.abs(isNaN(proceeds) ? price * shares : proceeds);

    // Commission: prefer separate Comm+Fee; fall back to combined column
    let commission = 0;
    if (iComm !== -1 || iFee !== -1) {
      const comm = iComm !== -1 ? parseNum(cols[iComm]) : 0;
      const fee  = iFee  !== -1 ? parseNum(cols[iFee])  : 0;
      commission = Math.abs(isNaN(comm) ? 0 : comm) + Math.abs(isNaN(fee) ? 0 : fee);
    } else if (iCommFee !== -1) {
      const cf = parseNum(cols[iCommFee]);
      commission = Math.abs(isNaN(cf) ? 0 : cf);
    }

    const traded_at = parseIbDateTime(cols[iDateTime] ?? "");

    rows.push({
      key: String(i),
      selected: true,
      transaction_type,
      stock_name: symbol,
      symbol: formatSymbol(symbol, market),
      traded_at,
      price: Math.abs(price),
      shares,
      total_amount,
      commission,
    });
  }

  return rows;
}

/** Layout B parser: standalone header row (no Section/Data prefix). */
function parseFlat(lines: string[], headerLineIdx: number, market: Market): EditableRow[] {
  const headerCols = splitCsvLine(lines[headerLineIdx]).map((c) => c.trim());
  const col = (name: string) => headerCols.indexOf(name);

  const iSymbol   = col("Symbol");
  const iDateTime = col("Trade Date/Time") !== -1 ? col("Trade Date/Time") : col("Date/Time");
  const iQuantity = col("Quantity");
  const iPrice    = col("Price") !== -1 ? col("Price") : col("T. Price");
  const iProceeds = col("Proceeds");
  const iType     = col("Type");
  const iAcctId   = col("Acct ID");
  const iComm     = col("Comm");
  const iFee      = col("Fee");
  const iCommFee  = col("Comm/Fee") !== -1 ? col("Comm/Fee") : col("Comm in USD");

  if (iSymbol === -1 || iDateTime === -1 || iQuantity === -1 || iPrice === -1) return [];

  const rows: EditableRow[] = [];
  for (let i = headerLineIdx + 1; i < lines.length; i++) {
    const cols = splitCsvLine(lines[i]);
    if (cols.length < 3) continue;

    const symbol = (cols[iSymbol] ?? "").trim();
    if (!symbol || symbol.startsWith("Total") || symbol === "Symbol") continue;

    // Skip grouping rows where Acct ID is not a real account number
    if (iAcctId !== -1 && !isValidAcctId(cols[iAcctId] ?? "")) continue;

    const quantity = parseNum(cols[iQuantity]);
    const price    = parseNum(cols[iPrice]);

    if (isNaN(quantity) || isNaN(price)) continue;

    let transaction_type: string;
    if (iType !== -1) {
      const t = (cols[iType] ?? "").trim().toUpperCase();
      transaction_type = t === "SELL" ? "SELL" : "BUY";
    } else {
      transaction_type = quantity >= 0 ? "BUY" : "SELL";
    }

    const shares       = Math.abs(quantity);
    const proceeds     = parseNum(cols[iProceeds]);
    const total_amount = Math.abs(isNaN(proceeds) ? price * shares : proceeds);

    let commission = 0;
    if (iComm !== -1 || iFee !== -1) {
      const comm = iComm !== -1 ? parseNum(cols[iComm]) : 0;
      const fee  = iFee  !== -1 ? parseNum(cols[iFee])  : 0;
      commission = Math.abs(isNaN(comm) ? 0 : comm) + Math.abs(isNaN(fee) ? 0 : fee);
    } else if (iCommFee !== -1) {
      const cf = parseNum(cols[iCommFee]);
      commission = Math.abs(isNaN(cf) ? 0 : cf);
    }

    const traded_at = parseIbDateTime(cols[iDateTime] ?? "");

    rows.push({
      key: String(i),
      selected: true,
      transaction_type,
      stock_name: symbol,
      symbol: formatSymbol(symbol, market),
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
 * Parse IB date/time strings.
 * Formats encountered: "2024-03-15, 09:30:00" or "2024/3/15" or "2024-03-15"
 */
function parseIbDateTime(raw: string): string {
  const cleaned = raw.trim();
  // "YYYY-MM-DD, HH:MM:SS"
  const m1 = cleaned.match(/^(\d{4}-\d{2}-\d{2}),?\s*(\d{2}:\d{2}:\d{2})/);
  if (m1) return `${m1[1]}T${m1[2]}`;

  // "YYYY/M/DD" or "YYYY-MM-DD"
  const d = dayjs(cleaned, ["YYYY/M/DD", "YYYY-M-D", "YYYY-MM-DD"], true);
  if (d.isValid()) return d.format("YYYY-MM-DDTHH:mm:ss");

  // Last resort: let dayjs try
  const fallback = dayjs(cleaned);
  if (fallback.isValid()) return fallback.format("YYYY-MM-DDTHH:mm:ss");
  // Return empty string to signal an unparseable date; callers can surface this to the user
  return "";
}

/**
 * Format symbol for the given market.
 * HK stocks on IB are numeric (e.g. "700" for Tencent); keep as-is.
 * US stocks are already tickers like "AAPL".
 */
function formatSymbol(symbol: string, market: Market): string {
  if (market === "HK") {
    // Pad HK symbols to 5 digits
    const digits = symbol.replace(/\D/g, "");
    if (digits) return digits.padStart(5, "0");
  }
  return symbol.toUpperCase();
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

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function ImportFromIbCsvModal({
  open,
  account,
  onClose,
  onImported,
}: ImportFromIbCsvModalProps) {
  const [step, setStep] = useState(0);
  const [fileList, setFileList] = useState<UploadFile[]>([]);
  const [rows, setRows] = useState<EditableRow[]>([]);
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);
  const [parseError, setParseError] = useState<string>("");

  const market: Market = account.market as Market;
  const currency: Currency = market === "HK" ? "HKD" : "USD";

  // ---- Step 0 helpers -------------------------------------------------------

  // ---- Step 1 helpers -------------------------------------------------------

  const updateRow = useCallback(
    (key: string, patch: Partial<EditableRow>) => {
      setRows((prev) =>
        prev.map((r) => (r.key === key ? { ...r, ...patch } : r))
      );
    },
    []
  );

  /**
   * After parsing the CSV, try to resolve a human-readable stock name for each
   * unique symbol.
   *
   * Resolution order:
   *   1. Look up in the user's existing holdings (any account) – fastest, no
   *      network call needed.
   *   2. Query Xueqiu `lookup_stock_name_by_symbol` for any symbol not found in
   *      step 1.
   *
   * All rows are marked `lookingUp: true` before the async work begins and
   * `lookingUp: false` once it completes, letting the table show a spinner.
   */
  const resolveStockNames = useCallback(async (parsedRows: EditableRow[]) => {
    // 1. Build symbol→name map from existing holdings (all accounts)
    const holdingNameMap = new Map<string, string>();
    try {
      const holdings = await invoke<{ symbol: string; name: string }[]>(
        "get_holdings",
        { accountId: null }
      );
      for (const h of holdings) {
        holdingNameMap.set(h.symbol.toUpperCase(), h.name);
      }
    } catch {
      // ignore — will fall back to Xueqiu for all symbols
    }

    // 2. Collect unique symbols that still need a name from Xueqiu
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
          const name = await invoke<string | null>("lookup_stock_name_by_symbol", {
            symbol: sym,
          });
          if (name) symbolNameMap.set(sym, name);
        } catch {
          // ignore individual lookup failures; user can edit manually
        }
      })
    );

    // 3. Apply resolved names; clear lookingUp flag
    setRows((prev) =>
      prev.map((r) => {
        const resolved = symbolNameMap.get(r.symbol.toUpperCase());
        return { ...r, stock_name: resolved ?? r.stock_name, lookingUp: false };
      })
    );
  }, []);

  const handleBeforeUpload = useCallback(
    (file: File) => {
      setParseError("");
      const reader = new FileReader();
      reader.onload = (e) => {
        const text = e.target?.result as string;
        try {
          const parsed = parseIbCsv(text, market);
          if (parsed.length === 0) {
            setParseError(
              "未从 CSV 中识别到交易记录。请确认文件为 Interactive Brokers 活动报表导出的 CSV，且包含 Trades 部分。"
            );
            return;
          }
          // Mark all rows as looking up their names before async resolution begins
          const withLoading = parsed.map((r) => ({ ...r, lookingUp: true }));
          setRows(withLoading);
          setStep(1);
          // Fire-and-forget: resolve names from holdings then Xueqiu
          resolveStockNames(withLoading);
        } catch (err) {
          setParseError(`CSV 解析失败: ${String(err)}`);
        }
      };
      reader.readAsText(file, "utf-8");
      setFileList([file as unknown as UploadFile]);
      return false; // prevent antd default upload
    },
    [market, resolveStockNames]
  );

  // ---- Step 2 helpers -------------------------------------------------------

  const handleImport = useCallback(async () => {
    const selected = rows.filter((r) => r.selected);
    if (selected.length === 0) {
      message.warning("请至少选择一条记录导入");
      return;
    }

    setImporting(true);
    let success = 0;
    const errors: { name: string; error: string }[] = [];

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
      title: "股票代码",
      key: "symbol",
      width: 110,
      render: (_: unknown, record: EditableRow) => (
        <Input
          size="small"
          value={record.symbol}
          style={{ width: 100 }}
          onChange={(e) =>
            updateRow(record.key, { symbol: market === "HK" ? e.target.value.trim() : e.target.value.trim().toUpperCase() })
          }
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
            onChange={(e) =>
              updateRow(record.key, { stock_name: e.target.value })
            }
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

  // ---- Footer ---------------------------------------------------------------

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
        <Button key="back" onClick={() => { setStep(0); setFileList([]); setRows([]); setParseError(""); }}>
          返回
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

  // ---- Render ---------------------------------------------------------------

  return (
    <Modal
      title={`从 CSV 导入交易记录（${market === "HK" ? "港股" : "美股"}）`}
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
            <p className="ant-upload-text">
              点击或拖拽 Interactive Brokers 活动报表 CSV 到此处
            </p>
            <p className="ant-upload-hint">
              在 IB TWS / 客户端 → 报告 → 活动报表 → 导出 CSV
            </p>
          </Dragger>
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
            message="仅支持 Interactive Brokers 活动报表（Activity Statement）CSV 格式"
            description={
              <span>
                账户市场：<strong>{market === "HK" ? "港股 (HKD)" : "美股 (USD)"}</strong>
                。导出路径：TWS → Reports → Activity Statement → CSV
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
            共解析 {rows.length} 条记录，已勾选{" "}
            {rows.filter((r) => r.selected).length} 条
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
