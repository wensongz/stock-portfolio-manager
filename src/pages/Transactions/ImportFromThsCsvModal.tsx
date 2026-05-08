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
const { Text, Paragraph } = Typography;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface EditableRow {
  key: string;
  selected: boolean;
  transaction_type: string; // "BUY" | "SELL"
  symbol: string;
  stock_name: string;
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

interface ImportFromThsCsvModalProps {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

// ---------------------------------------------------------------------------
// CSV parsing helpers
// ---------------------------------------------------------------------------

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

function parseNum(s: string | undefined): number {
  return parseFloat((s ?? "").replace(/,/g, "").trim());
}

/**
 * Build an ISO-8601 datetime string from THS date/time fields.
 *   date: "20260430" or "2026/04/30" or "2026-04-30"
 *   time: "14:13:09" or "141309" or "" (defaults to 09:30:00)
 */
function buildDateTime(date: string, time: string): string {
  const d = date.trim().replace(/[\/\-]/g, "");
  const datePart =
    d.length === 8
      ? `${d.slice(0, 4)}-${d.slice(4, 6)}-${d.slice(6, 8)}`
      : date.trim();

  const t = time.trim().replace(/:/g, "");
  const timePart =
    t.length === 6
      ? `${t.slice(0, 2)}:${t.slice(2, 4)}:${t.slice(4, 6)}`
      : time.trim() || "09:30:00";

  return `${datePart}T${timePart}`;
}

/**
 * Derive the SH/SZ-prefixed symbol from the 6-digit code and optional
 * exchange name column.
 */
function deriveSymbol(code: string, exchange: string): string {
  const c = code.trim();
  if (exchange.includes("上海") || exchange.toUpperCase().includes("SH")) {
    return `SH${c}`;
  }
  if (exchange.includes("深圳") || exchange.toUpperCase().includes("SZ")) {
    return `SZ${c}`;
  }
  // Heuristic: Shanghai A-shares begin with 6; Shenzhen with 0 or 3
  return c.startsWith("6") ? `SH${c}` : `SZ${c}`;
}

/**
 * Parse a 同花顺 historical-trade CSV export.
 *
 * Recognised columns (THS saves them in GB18030; the component retries with
 * that encoding when UTF-8 yields no header):
 *   成交日期  成交时间  证券代码  证券名称  交易所名称
 *   成交价格  成交数量  成交金额  发生金额
 *   手续费  印花税  附加费  过户费
 *
 * Commission is aggregated: 手续费 + 印花税 + 附加费 + 过户费.
 * Transaction type is inferred from 发生金额 sign: negative → BUY, positive → SELL.
 */
function parseThsCsv(text: string): EditableRow[] {
  // Strip UTF-8 BOM
  const stripped = text.startsWith("\uFEFF") ? text.slice(1) : text;
  const lines = stripped.split(/\r?\n/);

  // Find header row: must contain 证券代码
  let headerIdx = -1;
  for (let i = 0; i < lines.length; i++) {
    if (lines[i].includes("证券代码")) {
      headerIdx = i;
      break;
    }
  }
  if (headerIdx === -1) return [];

  const headers = splitCsvLine(lines[headerIdx]).map((h) => h.trim());
  const col = (name: string) => headers.indexOf(name);

  const iDate = col("成交日期") !== -1 ? col("成交日期") : col("交易日期");
  const iTime = col("成交时间");
  const iCode = col("证券代码");
  const iName = col("证券名称");
  const iExchange = col("交易所名称");
  const iPrice = col("成交价格");
  const iShares = col("成交数量");
  const iAmount = col("成交金额");
  const iHappen = col("发生金额");
  const iCommission = col("手续费");
  const iStamp = col("印花税");
  const iExtra = col("附加费");
  const iTransfer = col("过户费");

  if (iCode === -1 || iShares === -1) return [];

  const rows: EditableRow[] = [];
  let idx = 0;

  for (let i = headerIdx + 1; i < lines.length; i++) {
    const line = lines[i];
    if (!line.trim()) continue;

    const cols = splitCsvLine(line);
    const get = (j: number) => (j !== -1 ? cols[j] ?? "" : "");

    // THS may store short codes without leading zeros (e.g. "1" for 000001).
    // Pad to 6 digits before validation.
    const code = get(iCode).trim().replace(/^\d{1,5}$/, (s) => s.padStart(6, "0"));
    // Skip rows without a valid 6-digit numeric code
    if (!/^\d{6}$/.test(code)) continue;

    const shares = parseNum(get(iShares));
    if (isNaN(shares) || shares === 0) continue;

    const price = parseNum(get(iPrice));
    const tradeAmount = parseNum(get(iAmount));
    const happenAmt = parseNum(get(iHappen));

    const total_amount = isNaN(tradeAmount) || tradeAmount === 0
      ? Math.round(Math.abs(price) * Math.abs(shares) * 100) / 100
      : Math.abs(tradeAmount);

    // Commission = sum of the four fee columns
    const commission = Math.round(
      (
        (isNaN(parseNum(get(iCommission))) ? 0 : Math.abs(parseNum(get(iCommission)))) +
        (isNaN(parseNum(get(iStamp))) ? 0 : Math.abs(parseNum(get(iStamp)))) +
        (isNaN(parseNum(get(iExtra))) ? 0 : Math.abs(parseNum(get(iExtra)))) +
        (isNaN(parseNum(get(iTransfer))) ? 0 : Math.abs(parseNum(get(iTransfer))))
      ) * 100
    ) / 100;

    // Transaction type from 发生金额 sign
    const transaction_type =
      !isNaN(happenAmt) && happenAmt > 0 ? "SELL" : "BUY";

    const exchange = get(iExchange);
    const symbol = deriveSymbol(code, exchange);
    const stock_name = get(iName).trim();

    const date = get(iDate);
    const time = get(iTime);
    const traded_at = buildDateTime(date, time);

    rows.push({
      key: String(idx++),
      selected: true,
      transaction_type,
      symbol,
      stock_name: stock_name || symbol,
      traded_at,
      price: Math.abs(price),
      shares: Math.abs(shares),
      total_amount,
      commission,
    });
  }

  return rows;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function ImportFromThsCsvModal({
  open,
  account,
  onClose,
  onImported,
}: ImportFromThsCsvModalProps) {
  const [step, setStep] = useState(0);
  const [fileList, setFileList] = useState<UploadFile[]>([]);
  const [rows, setRows] = useState<EditableRow[]>([]);
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);
  const [parseError, setParseError] = useState<string>("");

  // ---- Row update helper -------------------------------------------------------

  const updateRow = useCallback(
    (key: string, patch: Partial<EditableRow>) => {
      setRows((prev) => prev.map((r) => (r.key === key ? { ...r, ...patch } : r)));
    },
    []
  );

  // ---- Name resolution ---------------------------------------------------------

  const resolveStockNames = useCallback(async (parsedRows: EditableRow[]) => {
    const holdingNameMap = new Map<string, string>();
    try {
      const holdings = await invoke<{ symbol: string; name: string }[]>("get_holdings", {
        accountId: null,
      });
      for (const h of holdings) {
        holdingNameMap.set(h.symbol.toUpperCase(), h.name);
      }
    } catch {
      // ignore
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
          // ignore; user can edit manually
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

  // ---- Upload handler ----------------------------------------------------------

  const attemptParse = useCallback(
    (text: string) => {
      const parsed = parseThsCsv(text);
      if (parsed.length === 0) return false;
      const withLoading = parsed.map((r) => ({ ...r, lookingUp: true }));
      setRows(withLoading);
      setStep(1);
      resolveStockNames(withLoading);
      return true;
    },
    [resolveStockNames]
  );

  const handleBeforeUpload = useCallback(
    (file: File) => {
      setParseError("");

      // Try UTF-8 first; retry with GB18030 if header not found
      const reader = new FileReader();
      reader.onload = (e) => {
        const textUtf8 = e.target?.result as string;
        if (attemptParse(textUtf8)) return;

        // Retry with GB18030 (THS sometimes saves in this encoding)
        const reader2 = new FileReader();
        reader2.onload = (e2) => {
          const textGb = e2.target?.result as string;
          if (!attemptParse(textGb)) {
            setParseError(
              "未从 CSV 中识别到交易记录。请确认文件为同花顺导出的历史成交 CSV，且包含「证券代码」列标题行。"
            );
          }
        };
        reader2.readAsText(file, "GB18030");
      };
      reader.readAsText(file, "utf-8");

      setFileList([file as unknown as UploadFile]);
      return false; // prevent antd auto-upload
    },
    [attemptParse]
  );

  // ---- Import handler ----------------------------------------------------------

  const handleImport = useCallback(async () => {
    const selected = rows.filter((r) => r.selected);
    if (selected.length === 0) {
      message.warning("请至少选择一条记录导入");
      return;
    }

    setImporting(true);
    let success = 0;
    const errors: { name: string; error: string }[] = [];

    // Import in chronological order for correct avg-cost calculation
    const sorted = [...selected].sort((a, b) => a.traded_at.localeCompare(b.traded_at));

    for (const r of sorted) {
      try {
        await invoke("create_transaction", {
          accountId: account.id,
          symbol: r.symbol.trim(),
          name: r.stock_name || r.symbol,
          market: "CN",
          transactionType: r.transaction_type,
          shares: r.shares,
          price: r.price,
          totalAmount: r.total_amount,
          commission: r.commission,
          currency: "CNY",
          tradedAt: new Date(r.traded_at).toISOString(),
        });
        success++;
        updateRow(r.key, { importOk: true, importError: undefined });
      } catch (err) {
        const msg = String(err);
        errors.push({ name: r.stock_name || r.symbol, error: msg });
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

  // ---- Reset -------------------------------------------------------------------

  const handleClose = useCallback(() => {
    setStep(0);
    setFileList([]);
    setRows([]);
    setParseError("");
    setImportResult(null);
    onClose();
  }, [onClose]);

  // ---- Table columns -----------------------------------------------------------

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
      width: 120,
      render: (_: unknown, record: EditableRow) => (
        <Input
          size="small"
          value={record.symbol}
          style={{ width: 110 }}
          onChange={(e) => updateRow(record.key, { symbol: e.target.value.trim() })}
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
          value={record.traded_at ? dayjs(record.traded_at) : null}
          onChange={(v) => {
            if (v) updateRow(record.key, { traded_at: v.format("YYYY-MM-DDTHH:mm:ss") });
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
      title: "手续费合计",
      key: "commission",
      width: 100,
      render: (_: unknown, record: EditableRow) => (
        <InputNumber
          size="small"
          value={record.commission}
          min={0}
          precision={2}
          onChange={(v) => updateRow(record.key, { commission: v ?? 0 })}
          style={{ width: 90 }}
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
            <CloseCircleOutlined style={{ color: "#ff4d4f" }} title={record.importError} />
          );
        return null;
      },
    },
  ];

  // ---- Footer ------------------------------------------------------------------

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
          onClick={() => {
            setStep(0);
            setFileList([]);
            setRows([]);
            setParseError("");
          }}
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

  // ---- Render ------------------------------------------------------------------

  return (
    <Modal
      title="从同花顺历史成交 CSV 导入交易记录（A股）"
      open={open}
      onCancel={handleClose}
      footer={footer}
      width={step === 1 ? 1020 : 560}
      destroyOnClose
    >
      <Steps
        current={step}
        items={[{ title: "上传 CSV" }, { title: "核对数据" }, { title: "导入结果" }]}
        className="mb-4"
      />

      {/* ---- Step 0: Upload ---- */}
      {step === 0 && (
        <div>
          <Alert
            type="info"
            showIcon
            className="mb-3"
            message="如何导出 CSV"
            description={
              <Paragraph className="!mb-0" style={{ fontSize: 13 }}>
                同花顺客户端导出的历史成交文件为 Excel 格式，请先用 WPS 表格、Microsoft
                Excel 或 macOS Numbers 打开，然后另存为 <strong>CSV（逗号分隔）</strong>
                格式，再上传到此处。程序将自动识别以下列：
                成交日期、成交时间、证券代码、证券名称、交易所名称、成交价格、成交数量、成交金额、发生金额、手续费、印花税、附加费、过户费。
                手续费将自动汇总（手续费 + 印花税 + 附加费 + 过户费）。
              </Paragraph>
            }
          />
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
            <p className="ant-upload-text">点击或拖拽同花顺历史成交 CSV 到此处</p>
            <p className="ant-upload-hint">支持 UTF-8 及 GB18030 编码，.csv 格式</p>
          </Dragger>
          {parseError && (
            <Alert type="error" message={parseError} className="mt-3" showIcon />
          )}
        </div>
      )}

      {/* ---- Step 1: Review ---- */}
      {step === 1 && (
        <div>
          <Text type="secondary" className="block mb-2">
            共识别 <strong>{rows.length}</strong> 条记录，请核对后点击「导入选中记录」。手续费已汇总（手续费 + 印花税 + 附加费 + 过户费）。
          </Text>
          <Table
            dataSource={rows}
            columns={columns}
            rowKey="key"
            size="small"
            pagination={false}
            scroll={{ x: 940, y: 420 }}
          />
        </div>
      )}

      {/* ---- Step 2: Result ---- */}
      {step === 2 && importResult && (
        <div>
          {importResult.success > 0 && (
            <Alert
              type="success"
              message={`成功导入 ${importResult.success} 条记录`}
              className="mb-3"
              showIcon
            />
          )}
          {importResult.failed > 0 && (
            <Alert
              type="error"
              message={`${importResult.failed} 条记录导入失败`}
              description={
                <ul className="mt-1">
                  {importResult.errors.map((e, i) => (
                    <li key={i}>
                      <strong>{e.name}</strong>: {e.error}
                    </li>
                  ))}
                </ul>
              }
              className="mb-3"
              showIcon
            />
          )}
        </div>
      )}
    </Modal>
  );
}
