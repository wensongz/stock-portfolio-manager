import { useState, useCallback } from "react";
import {
  Modal,
  Steps,
  Button,
  Upload,
  Table,
  Alert,
  Typography,
  InputNumber,
  Input,
  Space,
  Checkbox,
  Tag,
  Spin,
} from "antd";
import { InboxOutlined } from "@ant-design/icons";
import type { UploadFile } from "antd/es/upload";
import type { Account, Market, Currency } from "../../types";
import { useHoldingStore } from "../../stores/holdingStore";
import { invoke } from "@tauri-apps/api/core";

const { Dragger } = Upload;
const { Text, Paragraph } = Typography;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface ParsedRow {
  key: string;
  selected: boolean;
  symbol: string;
  name: string;
  shares: number;
  avgCost: number;
  lookingUp?: boolean;
  importOk?: boolean;
  importError?: string;
}

interface ParseResult {
  rows: ParsedRow[];
  warnings: string[];
}

interface ImportHoldingFromIbCsvModalProps {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

// ---------------------------------------------------------------------------
// CSV/TSV parsing helpers
// ---------------------------------------------------------------------------

/** Split a single CSV line respecting double-quoted fields. */
function splitCsvLine(line: string): string[] {
  const result: string[] = [];
  let current = "";
  let inQuotes = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === '"') {
      if (inQuotes && i + 1 < line.length && line[i + 1] === '"') { current += '"'; i++; }
      else { inQuotes = !inQuotes; }
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
 * Auto-detect whether a line uses tabs or commas as the separator,
 * then split accordingly.
 */
function splitLine(line: string): string[] {
  const tabCount = (line.match(/\t/g) ?? []).length;
  const commaCount = (line.match(/,/g) ?? []).length;
  if (tabCount > commaCount) return line.split("\t");
  return splitCsvLine(line);
}

/**
 * Format the IB symbol for the target market.
 * HK stocks on IB are numeric (e.g. "1211"); convert to "1211.HK".
 * US stocks are already tickers like "AAPL".
 */
function formatSymbol(symbol: string, market: Market): string {
  if (market === "HK") {
    const digits = symbol.replace(/\D/g, "");
    if (digits) {
      const num = parseInt(digits, 10);
      if (!isNaN(num)) return `${num}.HK`;
    }
  }
  return symbol.toUpperCase();
}

// Words/tokens that appear as section or summary labels in IB reports and
// should never be treated as stock symbols.
const SKIP_RE = /^(Stocks|Bonds|Options|Futures|Forex|Total|USD|HKD|CNY|EUR|GBP|JPY|CAD|AUD|CHF|NZD|SGD)$/i;

/**
 * Layout A parser: full IB activity statement CSV where rows are prefixed
 * "Open Positions,Header,..." and "Open Positions,Data,...".
 */
function parseStructured(lines: string[], headerLineIdx: number, market: Market): ParsedRow[] {
  const headerCols = splitCsvLine(lines[headerLineIdx]).map((c) => c.trim());
  const col = (name: string) => headerCols.indexOf(name);

  const iSymbol    = col("Symbol");
  const iQty       = col("Quantity");
  const iCostPrice = col("Cost Price");

  if (iSymbol === -1 || iQty === -1 || iCostPrice === -1) return [];

  const rows: ParsedRow[] = [];
  let idx = 0;

  for (let i = headerLineIdx + 1; i < lines.length; i++) {
    const cols = splitCsvLine(lines[i]);
    if (cols[0].trim() !== "Open Positions" || cols[1].trim() !== "Data") continue;

    const raw = (cols[iSymbol] ?? "").trim();
    if (!raw || SKIP_RE.test(raw)) continue;

    const qty       = parseNum(cols[iQty]);
    const costPrice = parseNum(cols[iCostPrice]);
    if (isNaN(qty) || qty <= 0 || isNaN(costPrice)) continue;

    rows.push({
      key:      String(idx++),
      selected: true,
      symbol:   formatSymbol(raw, market),
      name:     raw,
      shares:   qty,
      avgCost:  costPrice,
    });
  }
  return rows;
}

/**
 * Layout B parser: flat CSV / TSV with a header row containing "Symbol"
 * and "Cost Price" (or "Avg Cost").
 */
function parseFlat(lines: string[], headerLineIdx: number, market: Market): ParsedRow[] {
  const headers    = splitLine(lines[headerLineIdx]).map((c) => c.trim());
  const col        = (name: string) => headers.indexOf(name);

  const iSymbol    = col("Symbol");
  const iQty       = col("Quantity");
  const iCostPrice = col("Cost Price") !== -1 ? col("Cost Price") : col("Avg Cost");

  if (iSymbol === -1 || iQty === -1 || iCostPrice === -1) return [];

  const rows: ParsedRow[] = [];
  let idx = 0;

  for (let i = headerLineIdx + 1; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line) continue;

    const cols = splitLine(line);
    const raw  = (cols[iSymbol] ?? "").trim();
    if (!raw || SKIP_RE.test(raw)) continue;

    const qty       = parseNum(cols[iQty]);
    const costPrice = parseNum(cols[iCostPrice]);
    if (isNaN(qty) || qty <= 0 || isNaN(costPrice)) continue;

    rows.push({
      key:      String(idx++),
      selected: true,
      symbol:   formatSymbol(raw, market),
      name:     raw,
      shares:   qty,
      avgCost:  costPrice,
    });
  }
  return rows;
}

/**
 * Parse an IB Open Positions CSV.
 *
 * Two formats are supported:
 *   A) Full IB activity statement – rows prefixed "Open Positions,Header,..."
 *      and "Open Positions,Data,...".
 *   B) Extracted flat table – a header row containing "Symbol" and
 *      "Cost Price" (or "Avg Cost").
 */
function parseIbOpenPositionsCsv(text: string, market: Market): ParseResult {
  const stripped = text.startsWith("\uFEFF") ? text.slice(1) : text;
  const lines    = stripped.split(/\r?\n/);
  const warnings: string[] = [];

  // --- Try Layout A ---
  for (let i = 0; i < lines.length; i++) {
    const cols = splitCsvLine(lines[i]);
    if (cols[0].trim() === "Open Positions" && cols[1].trim() === "Header") {
      const rows = parseStructured(lines, i, market);
      if (rows.length > 0) return { rows, warnings };
    }
  }

  // --- Fallback Layout B ---
  for (let i = 0; i < lines.length; i++) {
    const cols = splitLine(lines[i]).map((c) => c.trim());
    if (
      cols.includes("Symbol") &&
      (cols.includes("Cost Price") || cols.includes("Avg Cost")) &&
      cols.includes("Quantity")
    ) {
      const rows = parseFlat(lines, i, market);
      if (rows.length > 0) return { rows, warnings };
    }
  }

  return {
    rows: [],
    warnings: [
      "未找到持仓数据。请确认 CSV 格式符合要求：IB 活动报表 CSV（含 Open Positions 段落），或包含 Symbol、Quantity、Cost Price 列的扁平表格",
    ],
  };
}

// ---------------------------------------------------------------------------
// Returns true if the account name suggests Interactive Brokers.
// ---------------------------------------------------------------------------
function isIbAccount(accountName: string): boolean {
  return /\b(ib|ibkr|interactive|盈透)\b/i.test(accountName);
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function ImportHoldingFromIbCsvModal({
  open,
  account,
  onClose,
  onImported,
}: ImportHoldingFromIbCsvModalProps) {
  const [step, setStep]               = useState(0);
  const [rows, setRows]               = useState<ParsedRow[]>([]);
  const [parseWarnings, setParseWarnings] = useState<string[]>([]);
  const [fileList, setFileList]       = useState<UploadFile[]>([]);
  const [importing, setImporting]     = useState(false);
  const [importResult, setImportResult] = useState<{
    success: number;
    failed: number;
    errors: { name: string; error: string }[];
  } | null>(null);

  const { createHolding } = useHoldingStore();

  const market: Market     = account.market as Market;
  const currency: Currency = market === "HK" ? "HKD" : "USD";
  const marketLabel        = market === "HK" ? "港股" : "美股";
  const showIbHint         = isIbAccount(account.name);

  const resetModal = useCallback(() => {
    setStep(0);
    setRows([]);
    setParseWarnings([]);
    setFileList([]);
    setImporting(false);
    setImportResult(null);
  }, []);

  const handleClose = () => {
    resetModal();
    onClose();
  };

  const handleFileParse = useCallback(
    async (file: File) => {
      const readAs = (encoding: string): Promise<string> =>
        new Promise((resolve, reject) => {
          const reader = new FileReader();
          reader.onload  = (e) => resolve(e.target?.result as string);
          reader.onerror = reject;
          reader.readAsText(file, encoding);
        });

      const text   = await readAs("utf-8");
      const result = parseIbOpenPositionsCsv(text, market);
      // Mark every row as looking-up so the name column shows a spinner
      const withLookup = result.rows.map((r) => ({ ...r, lookingUp: true }));
      setRows(withLookup);
      setParseWarnings(result.warnings);
      setStep(1);
      // Resolve names asynchronously (does not block navigation to step 1)
      resolveStockNames(withLookup);
    },
    [market], // eslint-disable-line react-hooks/exhaustive-deps
  );

  /**
   * Resolve stock names for parsed rows:
   *  1. Look up from existing holdings in the DB (all accounts) — no network.
   *  2. For any symbol still unknown, call `lookup_stock_name_by_symbol` (Xueqiu).
   *  3. Fall back to the symbol itself if both fail.
   */
  const resolveStockNames = useCallback(async (parsedRows: ParsedRow[]) => {
    // Step 1 – build symbol→name map from local holdings cache
    const holdingNameMap = new Map<string, string>();
    try {
      const holdings = await invoke<{ symbol: string; name: string }[]>(
        "get_holdings",
        { accountId: null },
      );
      for (const h of holdings) {
        holdingNameMap.set(h.symbol.toUpperCase(), h.name);
      }
    } catch {
      // ignore – will fall back to Xueqiu for all symbols
    }

    const uniqueSymbols = [...new Set(parsedRows.map((r) => r.symbol.toUpperCase()))];
    const symbolNameMap = new Map<string, string>();
    for (const sym of uniqueSymbols) {
      const cached = holdingNameMap.get(sym);
      if (cached) symbolNameMap.set(sym, cached);
    }

    // Step 2 – query Xueqiu for symbols not found in the local cache
    const needLookup = uniqueSymbols.filter((s) => !symbolNameMap.has(s));
    await Promise.all(
      needLookup.map(async (sym) => {
        try {
          const name = await invoke<string | null>("lookup_stock_name_by_symbol", {
            symbol: sym,
          });
          if (name) symbolNameMap.set(sym, name);
        } catch {
          // ignore – user can edit manually
        }
      }),
    );

    // Step 3 – apply resolved names and clear the spinner flag
    setRows((prev) =>
      prev.map((r) => {
        const resolved = symbolNameMap.get(r.symbol.toUpperCase());
        return { ...r, name: resolved ?? r.symbol, lookingUp: false };
      }),
    );
  }, []);

  const handleImport = async () => {
    setImporting(true);
    const result = {
      success: 0,
      failed:  0,
      errors:  [] as { name: string; error: string }[],
    };

    for (const row of rows.filter((r) => r.selected)) {
      try {
        await createHolding({
          accountId: account.id,
          symbol:    row.symbol,
          name:      row.name || row.symbol,
          market,
          shares:    row.shares,
          avgCost:   row.avgCost,
          currency,
        });
        setRows((prev) =>
          prev.map((r) => (r.key === row.key ? { ...r, importOk: true } : r)),
        );
        result.success++;
      } catch (err) {
        setRows((prev) =>
          prev.map((r) =>
            r.key === row.key
              ? { ...r, importOk: false, importError: String(err) }
              : r,
          ),
        );
        result.failed++;
        result.errors.push({ name: row.name, error: String(err) });
      }
    }

    setImportResult(result);
    setImporting(false);
    setStep(2);
    if (result.success > 0) onImported();
  };

  const updateRow = (key: string, field: keyof ParsedRow, value: unknown) => {
    setRows((prev) =>
      prev.map((r) => (r.key === key ? { ...r, [field]: value } : r)),
    );
  };

  const tagColor = market === "HK" ? "green" : "blue";

  const baseColumns = [
    {
      title: "导入",
      key:   "selected",
      width: 55,
      render: (_: unknown, record: ParsedRow) => (
        <Checkbox
          checked={record.selected}
          disabled={step === 2}
          onChange={(e) => updateRow(record.key, "selected", e.target.checked)}
        />
      ),
    },
    {
      title: "类型",
      key:   "type",
      width: 65,
      render: () => <Tag color={tagColor}>{marketLabel}</Tag>,
    },
    {
      title: "股票代码",
      key:   "symbol",
      width: 130,
      render: (_: unknown, record: ParsedRow) =>
        step === 2 ? (
          <Text>{record.symbol}</Text>
        ) : (
          <Input
            size="small"
            value={record.symbol}
            onChange={(e) => updateRow(record.key, "symbol", e.target.value)}
          />
        ),
    },
    {
      title: "名称",
      key:   "name",
      width: 130,
      render: (_: unknown, record: ParsedRow) =>
        record.lookingUp ? (
          <Spin size="small" />
        ) : step === 2 ? (
          <Text>{record.name}</Text>
        ) : (
          <Input
            size="small"
            value={record.name}
            onChange={(e) => updateRow(record.key, "name", e.target.value)}
          />
        ),
    },
    {
      title: "持仓数量",
      key:   "shares",
      width: 120,
      render: (_: unknown, record: ParsedRow) =>
        step === 2 ? (
          <Text>{record.shares.toLocaleString()}</Text>
        ) : (
          <InputNumber
            size="small"
            value={record.shares}
            min={0}
            precision={0}
            onChange={(v) => updateRow(record.key, "shares", v ?? 0)}
            style={{ width: "100%" }}
          />
        ),
    },
    {
      title: "平均成本",
      key:   "avgCost",
      width: 120,
      render: (_: unknown, record: ParsedRow) =>
        step === 2 ? (
          <Text>{record.avgCost.toFixed(6)}</Text>
        ) : (
          <InputNumber
            size="small"
            value={record.avgCost}
            min={0}
            precision={6}
            onChange={(v) => updateRow(record.key, "avgCost", v ?? 0)}
            style={{ width: "100%" }}
          />
        ),
    },
  ];

  const resultStatusColumn = {
    title: "状态",
    key:   "status",
    width: 90,
    render: (_: unknown, record: ParsedRow) => {
      if (!record.selected) return <Text type="secondary">已跳过</Text>;
      if (record.importOk)  return <Text type="success">✓ 成功</Text>;
      if (record.importOk === false)
        return (
          <Text type="danger" title={record.importError}>
            ✗ 失败
          </Text>
        );
      return null;
    },
  };

  const stepItems = [
    { title: "上传文件" },
    { title: "确认数据" },
    { title: "导入完成" },
  ];

  const selectedCount = rows.filter((r) => r.selected).length;
  const isLookingUp   = rows.some((r) => r.lookingUp);

  return (
    <Modal
      title={`从CSV导入持仓（${marketLabel}）— ${account.name}`}
      open={open}
      onCancel={handleClose}
      footer={null}
      width={820}
      destroyOnClose
    >
      <Steps current={step} items={stepItems} className="mb-6" />

      {/* ── Step 0: Upload ── */}
      {step === 0 && (
        <div>
          {showIbHint ? (
            <Alert
              type="info"
              className="mb-4"
              message="Interactive Brokers 持仓导入"
              description={
                <div>
                  <p style={{ marginBottom: 4 }}>
                    请将 IB 活动报表（Activity Statement）中的{" "}
                    <b>Open Positions</b> 部分提取到 CSV 文件：
                  </p>
                  <ol style={{ paddingLeft: 20, marginBottom: 0 }}>
                    <li>
                      在 IB TWS 或网页客户端中生成<b>活动报表</b>，导出格式选择
                      {" "}<b>CSV</b>；
                    </li>
                    <li>
                      用文本编辑器或 Excel 打开 CSV，找到{" "}
                      <b>Open Positions</b> 段落，将含表头和数据行的内容复制并另存为新的 CSV 文件；
                    </li>
                    <li>
                      或将整个活动报表 CSV 直接上传——本工具会自动提取 Open Positions 段落。
                    </li>
                  </ol>
                </div>
              }
            />
          ) : (
            <Alert
              type="info"
              className="mb-4"
              message={`${marketLabel}持仓 CSV 导入`}
              description={
                <p style={{ marginBottom: 0 }}>
                  请上传包含 <b>Symbol</b>、<b>Quantity</b>、<b>Cost Price</b>{" "}
                  列的 CSV 文件。支持 Interactive Brokers Open Positions 导出格式及兼容格式。
                </p>
              }
            />
          )}
          <Dragger
            accept=".csv,.txt"
            maxCount={1}
            fileList={fileList}
            beforeUpload={(file) => {
              setFileList([
                {
                  uid:           `${file.name}-${Date.now()}`,
                  name:          file.name,
                  originFileObj: file,
                } as UploadFile,
              ]);
              handleFileParse(file);
              return false;
            }}
            onChange={({ fileList: fl }) => setFileList(fl)}
          >
            <p className="ant-upload-drag-icon">
              <InboxOutlined />
            </p>
            <p className="ant-upload-text">点击或将 CSV 文件拖拽到此处</p>
            <p className="ant-upload-hint">
              支持 IB 活动报表 CSV 或手动提取的 Open Positions 表格
            </p>
          </Dragger>
        </div>
      )}

      {/* ── Step 1: Review ── */}
      {step === 1 && (
        <div>
          {parseWarnings.map((w, i) => (
            <Alert key={i} type="warning" message={w} className="mb-2" showIcon />
          ))}
          {rows.length === 0 ? (
            <Alert
              type="error"
              message="未能解析到任何持仓数据，请检查CSV文件格式是否正确"
            />
          ) : (
            <>
              <Paragraph type="secondary" className="mb-2">
                共解析到 <b>{rows.length}</b> 条记录。请确认数据后点击导入；可取消勾选不需要导入的行，也可直接编辑代码/名称/数量/成本。
              </Paragraph>
              <Table
                dataSource={rows}
                columns={baseColumns}
                rowKey="key"
                size="small"
                pagination={false}
                scroll={{ y: 380 }}
              />
            </>
          )}
          <div className="mt-4 flex justify-end">
            <Space>
              <Button onClick={() => setStep(0)}>上一步</Button>
              <Button
                type="primary"
                disabled={rows.length === 0 || selectedCount === 0 || isLookingUp}
                loading={importing}
                onClick={handleImport}
              >
                导入（{selectedCount} 条）
              </Button>
            </Space>
          </div>
        </div>
      )}

      {/* ── Step 2: Done ── */}
      {step === 2 && importResult && (
        <div>
          <Alert
            type={importResult.failed === 0 ? "success" : "warning"}
            message={`导入完成：成功 ${importResult.success} 条，失败 ${importResult.failed} 条`}
            className="mb-4"
          />
          {importResult.errors.length > 0 && (
            <div className="mb-4">
              {importResult.errors.map((e, i) => (
                <Alert
                  key={i}
                  type="error"
                  message={`${e.name}: ${e.error}`}
                  className="mb-1"
                  showIcon
                />
              ))}
            </div>
          )}
          <Table
            dataSource={rows.filter((r) => r.selected)}
            columns={[...baseColumns, resultStatusColumn]}
            rowKey="key"
            size="small"
            pagination={false}
            scroll={{ y: 360 }}
          />
          <div className="mt-4 flex justify-end">
            <Button type="primary" onClick={handleClose}>
              完成
            </Button>
          </div>
        </div>
      )}
    </Modal>
  );
}
