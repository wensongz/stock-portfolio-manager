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

interface ImportFromMoomooCsvModalProps {
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
 * Format a moomoo stock code for the target market.
 * HK stocks are plain numbers (e.g. "9992") → "9992.HK"
 * US stocks are already tickers (e.g. "AAPL") → uppercase
 * CN stocks are 6-digit codes (e.g. "600036") → kept as-is
 */
function formatMoomooSymbol(code: string, market: Market): string {
  const c = code.trim();
  if (market === "HK") {
    const digits = c.replace(/\D/g, "");
    if (digits) return `${parseInt(digits, 10)}.HK`;
  }
  return c.toUpperCase();
}

/**
 * Parse a moomoo date/time string.
 * Formats: "2026/03/25", "2026/03/25 09:30", "2026/03/25 09:30:00"
 */
function parseMoomooDate(raw: string): string {
  const cleaned = raw.trim();
  const m = cleaned.match(/^(\d{4})\/(\d{2})\/(\d{2})(?:\s+(\d{2}:\d{2}(?::\d{2})?))?/);
  if (m) {
    const datePart = `${m[1]}-${m[2]}-${m[3]}`;
    const timePart = m[4] ? (m[4].length === 5 ? m[4] + ":00" : m[4]) : "09:30:00";
    return `${datePart}T${timePart}`;
  }
  const d = dayjs(cleaned);
  return d.isValid() ? d.format("YYYY-MM-DDTHH:mm:ss") : "";
}

/**
 * Infer market from the moomoo market string column.
 */
function detectMarket(marketStr: string, fallback: Market): Market {
  const s = marketStr.trim();
  if (s.includes("港") || s.toUpperCase().includes("HK")) return "HK";
  if (s.includes("美") || s.toUpperCase().includes("US")) return "US";
  if (s.includes("A股") || s.includes("沪") || s.includes("深") || s.toUpperCase().includes("CN")) return "CN";
  return fallback;
}

/**
 * Parse a moomoo trade-history CSV export.
 *
 * The CSV has a single header row starting with "方向". Subsequent data rows
 * where 方向 is "买入" or "卖出" represent a new order; rows where 方向 is
 * empty are sub-executions of the preceding order and are merged into it.
 *
 * Key columns used:
 *   方向       – direction (买入 / 卖出 / empty for sub-execution)
 *   代码       – stock code
 *   名称       – stock name
 *   市场       – market (港股 / 美股 / A股)
 *   成交数量   – executed shares
 *   成交价格   – executed price
 *   成交金额   – executed amount
 *   成交时间   – execution datetime
 *   合计费用   – total commission (older exports may use 合计手续费)
 */
function parseMoomooCsv(text: string, defaultMarket: Market): EditableRow[] {
  // Strip UTF-8 BOM if present
  const stripped = text.startsWith("\uFEFF") ? text.slice(1) : text;
  const lines = stripped.split(/\r?\n/);

  // Locate header row: first column must be "方向"
  let headerIdx = -1;
  for (let i = 0; i < lines.length; i++) {
    const cols = splitCsvLine(lines[i]).map((c) => c.trim());
    if (cols[0] === "方向") {
      headerIdx = i;
      break;
    }
  }
  if (headerIdx === -1) return [];

  const headerCols = splitCsvLine(lines[headerIdx]).map((c) => c.trim());
  const col = (name: string) => headerCols.indexOf(name);

  const iDir = col("方向");
  const iCode = col("代码");
  const iName = col("名称");
  const iMarket = col("市场");
  const iShares = col("成交数量");
  const iPrice = col("成交价格");
  const iAmount = col("成交金额");
  const iTime = col("成交时间");
  // "合计费用" is the total-fee column; fall back to "合计手续费" for older exports
  const iCommission = col("合计费用") !== -1 ? col("合计费用") : col("合计手续费");

  if (iCode === -1 || iShares === -1 || iPrice === -1) return [];

  interface SubRow {
    shares: number;
    price: number;
    amount: number;
    time: string;
    commission: number;
  }

  interface Group {
    direction: string;
    code: string;
    name: string;
    market: Market;
    subRows: SubRow[];
  }

  const rows: EditableRow[] = [];
  let currentGroup: Group | null = null;
  let groupIdx = 0;

  const finalizeGroup = () => {
    if (!currentGroup || currentGroup.subRows.length === 0) return;
    const totalShares = currentGroup.subRows.reduce((s, r) => s + r.shares, 0);
    const totalAmount = currentGroup.subRows.reduce((s, r) => s + r.amount, 0);
    const avgPrice = totalShares > 0 ? totalAmount / totalShares : currentGroup.subRows[0].price;
    const commission = currentGroup.subRows.reduce((s, r) => s + r.commission, 0);
    const traded_at = currentGroup.subRows[0].time;

    rows.push({
      key: String(groupIdx++),
      selected: true,
      transaction_type: currentGroup.direction,
      stock_name: currentGroup.name,
      symbol: formatMoomooSymbol(currentGroup.code, currentGroup.market),
      traded_at,
      price: Math.round(avgPrice * 10000) / 10000,
      shares: totalShares,
      total_amount: Math.round(totalAmount * 100) / 100,
      commission: Math.round(commission * 100) / 100,
    });
  };

  for (let i = headerIdx + 1; i < lines.length; i++) {
    const line = lines[i];
    if (!line.trim()) continue;

    const cols = splitCsvLine(line);
    const direction = (iDir !== -1 ? cols[iDir] ?? "" : "").trim();
    const code = (iCode !== -1 ? cols[iCode] ?? "" : "").trim();

    const isMainRow = direction === "买入" || direction === "卖出";
    // Sub-execution rows have an empty direction; they belong to the previous order
    // and may also have an empty code column in the moomoo export.
    const isSubRow = direction === "" && currentGroup !== null;

    if (!isMainRow && !isSubRow) continue;

    const shares = parseNum(cols[iShares]);
    const price = parseNum(cols[iPrice]);
    const amount = parseNum(iAmount !== -1 ? cols[iAmount] : undefined);
    const timeStr = (iTime !== -1 ? cols[iTime] ?? "" : "").trim();
    const commRaw = iCommission !== -1 ? parseNum(cols[iCommission]) : NaN;

    if (isNaN(shares) || isNaN(price)) continue;

    const subRow: SubRow = {
      shares: Math.abs(shares),
      price: Math.abs(price),
      amount: Math.abs(isNaN(amount) ? Math.abs(price) * Math.abs(shares) : amount),
      time: parseMoomooDate(timeStr),
      commission: isNaN(commRaw) ? 0 : Math.abs(commRaw),
    };

    if (isMainRow) {
      if (!code) continue; // main rows must have a stock code
      finalizeGroup();
      const marketStr = iMarket !== -1 ? (cols[iMarket] ?? "").trim() : "";
      const market = marketStr ? detectMarket(marketStr, defaultMarket) : defaultMarket;
      const name = (iName !== -1 ? cols[iName] ?? "" : "").trim();
      currentGroup = {
        direction: direction === "卖出" ? "SELL" : "BUY",
        code,
        name: name || code,
        market,
        subRows: [subRow],
      };
    } else {
      // Sub-execution of the current order
      currentGroup!.subRows.push(subRow);
    }
  }

  finalizeGroup();
  return rows;
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function ImportFromMoomooCsvModal({
  open,
  account,
  onClose,
  onImported,
}: ImportFromMoomooCsvModalProps) {
  const [step, setStep] = useState(0);
  const [fileList, setFileList] = useState<UploadFile[]>([]);
  const [rows, setRows] = useState<EditableRow[]>([]);
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);
  const [parseError, setParseError] = useState<string>("");

  const defaultMarket: Market = account.market as Market;
  const currency: Currency = defaultMarket === "HK" ? "HKD" : defaultMarket === "CN" ? "CNY" : "USD";

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
      // ignore – will fall back to Xueqiu
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
        // Prefer resolved name; only fall back to CSV name when name is just the raw code
        return { ...r, stock_name: resolved ?? r.stock_name, lookingUp: false };
      })
    );
  }, []);

  // ---- Upload handler ----------------------------------------------------------

  const tryParse = useCallback(
    (text: string): EditableRow[] | null => {
      try {
        const parsed = parseMoomooCsv(text, defaultMarket);
        return parsed.length > 0 ? parsed : null;
      } catch {
        return null;
      }
    },
    [defaultMarket]
  );

  const handleBeforeUpload = useCallback(
    (file: File) => {
      setParseError("");

      const attemptParse = (text: string) => {
        const parsed = tryParse(text);
        if (!parsed) {
          setParseError(
            "未从 CSV 中识别到交易记录。请确认文件为 moomoo（富途）客户端导出的交易记录 CSV，且包含「方向」列标题行。"
          );
          return;
        }
        const withLoading = parsed.map((r) => ({ ...r, lookingUp: true }));
        setRows(withLoading);
        setStep(1);
        resolveStockNames(withLoading);
      };

      // Try UTF-8 first; if header not found retry with GB18030
      const reader = new FileReader();
      reader.onload = (e) => {
        const textUtf8 = e.target?.result as string;
        if (parseMoomooCsv(textUtf8, defaultMarket).length > 0 || textUtf8.includes("方向")) {
          attemptParse(textUtf8);
        } else {
          // Retry with GB18030 encoding
          const reader2 = new FileReader();
          reader2.onload = (e2) => {
            attemptParse(e2.target?.result as string);
          };
          reader2.readAsText(file, "GB18030");
        }
      };
      reader.readAsText(file, "utf-8");

      setFileList([file as unknown as UploadFile]);
      return false;
    },
    [defaultMarket, tryParse, resolveStockNames]
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

    const sorted = [...selected].sort((a, b) => a.traded_at.localeCompare(b.traded_at));

    for (const r of sorted) {
      const rowMarket: Market =
        r.symbol.endsWith(".HK") ? "HK" : defaultMarket;
      const rowCurrency: Currency =
        rowMarket === "HK" ? "HKD" : rowMarket === "CN" ? "CNY" : "USD";

      try {
        await invoke("create_transaction", {
          accountId: account.id,
          symbol: r.symbol.trim(),
          name: r.stock_name,
          market: rowMarket,
          transactionType: r.transaction_type,
          shares: r.shares,
          price: r.price,
          totalAmount: r.total_amount,
          commission: r.commission,
          currency: rowCurrency,
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
  }, [rows, account.id, defaultMarket, updateRow, onImported]);

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
      width: 110,
      render: (_: unknown, record: EditableRow) => (
        <Input
          size="small"
          value={record.symbol}
          style={{ width: 100 }}
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

  const marketLabel = defaultMarket === "HK" ? "港股" : defaultMarket === "US" ? "美股" : "A股";

  return (
    <Modal
      title={`从 moomoo CSV 导入交易记录（${marketLabel}）`}
      open={open}
      onCancel={handleClose}
      footer={footer}
      width={step === 1 ? 980 : 520}
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
            <p className="ant-upload-text">点击或拖拽 moomoo 交易记录 CSV 到此处</p>
            <p className="ant-upload-hint">
              moomoo 客户端 → 账户 → 选择账户 → 持仓 / 订单 / 历史 → 导出
            </p>
          </Dragger>
          {parseError && (
            <Alert type="error" message={parseError} className="mt-3" showIcon />
          )}
          <Alert
            type="info"
            className="mt-3"
            showIcon
            message="仅支持 moomoo（富途）客户端导出的交易记录 CSV"
            description={
              <span>
                账户市场：<strong>{marketLabel}（{currency}）</strong>
                。同一订单的多次分批成交将被自动合并为一条记录。
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
            message={`账户「${account.name}」[${defaultMarket}]，市场和货币已自动识别`}
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
