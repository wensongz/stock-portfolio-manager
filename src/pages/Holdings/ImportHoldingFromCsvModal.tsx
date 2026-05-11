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
} from "antd";
import { InboxOutlined } from "@ant-design/icons";
import type { UploadFile } from "antd/es/upload";
import type { Account } from "../../types";
import { useHoldingStore } from "../../stores/holdingStore";
import { useCategoryStore } from "../../stores/categoryStore";

const { Dragger } = Upload;
const { Text, Paragraph } = Typography;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

interface ParsedRow {
  key: string;
  selected: boolean;
  isCash: boolean;
  symbol: string;    // e.g. sh600519 or $CASH-CNY
  name: string;
  shares: number;    // for cash: amount in CNY; for stock: share count
  avgCost: number;   // for cash: 1; for stock: average cost price
  importOk?: boolean;
  importError?: string;
}

interface ParseResult {
  rows: ParsedRow[];
  warnings: string[];
}

interface ImportHoldingFromCsvModalProps {
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
      if (inQuotes && line[i + 1] === '"') { current += '"'; i++; }
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
 * Derive the sh/sz-prefixed symbol from the 6-digit code and an optional
 * exchange label (交易市场 column value, e.g. "上海A股" / "深圳A股").
 */
function deriveSymbol(code: string, exchange: string): string {
  const c = code.trim();
  if (exchange.includes("上海") || exchange.toUpperCase().startsWith("SH")) return `sh${c}`;
  if (exchange.includes("深圳") || exchange.toUpperCase().startsWith("SZ")) return `sz${c}`;
  // Heuristic: SH codes start with 5 or 6; SZ codes start with 0–4.
  return c.startsWith("6") || c.startsWith("5") ? `sh${c}` : `sz${c}`;
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
 * Parse a CN-broker position CSV export.
 *
 * Supported formats
 * -----------------
 * Format 1 – 同花顺 (THS) holding export
 *   Summary rows at top (first row: 市种 | 余额 | 可用 | …
 *                        second row: 人民币 | 279.08 | 279.08 | …)
 *   Holdings header row: 证券代码 | 证券名称 | 昨日余额 | 参考持股 | … | 成本价 | …
 *   Key columns: 参考持股 (shares), 成本价 (avg cost)
 *   No explicit exchange column → use code-based heuristic.
 *
 * Format 2 – Generic CN-broker holding export
 *   Holdings header row: … | 证券代码 | 证券名称 | 持仓数量 | … | 参考成本价 | … | 交易市场 | …
 *   Key columns: 持仓数量 (shares), 参考成本价 (avg cost)
 *   Exchange column: 交易市场 → "深圳A股" / "上海A股".
 */
function parseCsv(text: string): ParseResult {
  const stripped = text.startsWith("\uFEFF") ? text.slice(1) : text;
  const lines = stripped.split(/\r?\n/);

  const warnings: string[] = [];
  let cashAmount: number | undefined;
  let headerIdx = -1;

  // ── Scan the first 20 lines for the cash summary row (Format 1) and the
  //    holdings header row.
  for (let i = 0; i < Math.min(lines.length, 20); i++) {
    const rawLine = lines[i];
    if (!rawLine.trim()) continue;

    const cols = splitLine(rawLine).map((c) => c.trim());

    // Format 1 cash header: look for row containing both "市种" and "可用"
    if (cols.some((c) => c === "市种") && cols.some((c) => c === "可用")) {
      const availIdx = cols.indexOf("可用");
      // The very next non-empty line should contain the 人民币 data row
      for (let j = i + 1; j < Math.min(lines.length, i + 5); j++) {
        const dataCols = splitLine(lines[j]).map((c) => c.trim());
        if (dataCols[0] === "人民币" || dataCols.some((c) => c === "人民币")) {
          const rmbIdx = dataCols.findIndex((c) => c === "人民币");
          const offset = rmbIdx >= 0 ? rmbIdx : 0;
          const val = parseNum(dataCols[availIdx + offset]);
          if (!isNaN(val) && val > 0) cashAmount = val;
          break;
        }
      }
    }

    // Holdings header row: must contain both "证券代码" and "证券名称"
    if (cols.some((c) => c === "证券代码") && cols.some((c) => c === "证券名称")) {
      headerIdx = i;
      break;
    }
  }

  // Broader search for the header row if not found yet
  if (headerIdx === -1) {
    for (let i = 0; i < lines.length; i++) {
      const cols = splitLine(lines[i]).map((c) => c.trim());
      if (cols.some((c) => c === "证券代码") && cols.some((c) => c === "证券名称")) {
        headerIdx = i;
        break;
      }
    }
  }

  if (headerIdx === -1) {
    return {
      rows: [],
      warnings: ["未找到持仓数据，请确认CSV格式是否正确（需含\u201c证券代码\u201d和\u201c证券名称\u201d列）"],
    };
  }

  const headers = splitLine(lines[headerIdx]).map((h) => h.trim());
  const col = (name: string) => headers.indexOf(name);

  const iCode = col("证券代码");
  const iName = col("证券名称");

  // Format 1 key columns
  const iSharesF1 = col("参考持股");
  const iCostF1 = col("成本价");

  // Format 2 key columns
  const iSharesF2 = col("持仓数量");
  const iCostF2 = col("参考成本价");
  const iMarketF2 = col("交易市场");

  let iShares = -1;
  let iCost = -1;
  let iMarket = -1;

  if (iSharesF1 !== -1 && iCostF1 !== -1) {
    iShares = iSharesF1;
    iCost = iCostF1;
    // No explicit exchange column in Format 1
  } else if (iSharesF2 !== -1 && iCostF2 !== -1) {
    iShares = iSharesF2;
    iCost = iCostF2;
    iMarket = iMarketF2;
  }

  if (iCode === -1 || iName === -1 || iShares === -1 || iCost === -1) {
    return {
      rows: [],
      warnings: [
        "无法识别CSV格式，请确认文件包含以下列之一：\n" +
          "• 证券代码、证券名称、参考持股、成本价（同花顺格式）\n" +
          "• 证券代码、证券名称、持仓数量、参考成本价（通用格式）",
      ],
    };
  }

  const rows: ParsedRow[] = [];
  let idx = 0;

  for (let i = headerIdx + 1; i < lines.length; i++) {
    const line = lines[i].trim();
    if (!line) continue;

    const cols = splitLine(line);
    const get = (j: number) => (j !== -1 ? (cols[j] ?? "").trim() : "");

    // Normalise code to 6 digits (THS sometimes omits leading zeros for short codes)
    const rawCode = get(iCode);
    const code = /^\d+$/.test(rawCode) ? rawCode.padStart(6, "0") : rawCode;
    if (!/^\d{6}$/.test(code)) continue;

    const shares = parseNum(get(iShares));
    if (isNaN(shares) || shares <= 0) continue;

    const cost = parseNum(get(iCost));
    // Accept cost=0 (some brokers export 0 for certain instruments) but skip NaN
    if (isNaN(cost)) continue;

    const name = get(iName);
    const exchange = iMarket !== -1 ? get(iMarket) : "";
    const symbol = deriveSymbol(code, exchange);

    rows.push({
      key: String(idx++),
      selected: true,
      isCash: false,
      symbol,
      name: name || symbol,
      shares,
      avgCost: cost,
    });
  }

  // Prepend a cash row if we detected one
  if (cashAmount !== undefined && cashAmount > 0) {
    rows.unshift({
      key: `cash-${idx}`,
      selected: true,
      isCash: true,
      symbol: "$CASH-CNY",
      name: "现金 (CNY)",
      shares: cashAmount,
      avgCost: 1,
    });
  }

  return { rows, warnings };
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function ImportHoldingFromCsvModal({
  open,
  account,
  onClose,
  onImported,
}: ImportHoldingFromCsvModalProps) {
  const [step, setStep] = useState(0);
  const [rows, setRows] = useState<ParsedRow[]>([]);
  const [parseWarnings, setParseWarnings] = useState<string[]>([]);
  const [fileList, setFileList] = useState<UploadFile[]>([]);
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<{
    success: number;
    failed: number;
    errors: { name: string; error: string }[];
  } | null>(null);

  const { createHolding } = useHoldingStore();
  const { categories } = useCategoryStore();

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

  const handleFileParse = useCallback(async (file: File) => {
    const readAs = (encoding: string): Promise<string> =>
      new Promise((resolve, reject) => {
        const reader = new FileReader();
        reader.onload = (e) => resolve(e.target?.result as string);
        reader.onerror = reject;
        reader.readAsText(file, encoding);
      });

    // Try UTF-8 first; if no rows found, retry with GB18030 (THS default encoding)
    let text = await readAs("utf-8");
    let result = parseCsv(text);

    if (result.rows.length === 0) {
      text = await readAs("gb18030");
      result = parseCsv(text);
    }

    setRows(result.rows);
    setParseWarnings(result.warnings);
    setStep(1);
  }, []);

  const handleImport = async () => {
    setImporting(true);
    const result = {
      success: 0,
      failed: 0,
      errors: [] as { name: string; error: string }[],
    };

    const cashCategory = categories.find((c) => c.name === "现金类");
    const selected = rows.filter((r) => r.selected);

    for (const row of selected) {
      try {
        if (row.isCash) {
          await createHolding({
            accountId: account.id,
            symbol: "$CASH-CNY",
            name: "现金 (CNY)",
            market: "CN",
            categoryId: cashCategory?.id,
            shares: row.shares,
            avgCost: 1,
            currency: "CNY",
          });
        } else {
          await createHolding({
            accountId: account.id,
            symbol: row.symbol,
            name: row.name,
            market: "CN",
            shares: row.shares,
            avgCost: row.avgCost,
            currency: "CNY",
          });
        }
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

  // Columns for review and result tables
  const baseColumns = [
    {
      title: "导入",
      key: "selected",
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
      key: "type",
      width: 70,
      render: (_: unknown, record: ParsedRow) =>
        record.isCash ? (
          <Tag color="gold">💵 现金</Tag>
        ) : (
          <Tag color="red">CN</Tag>
        ),
    },
    {
      title: "证券代码",
      key: "symbol",
      width: 130,
      render: (_: unknown, record: ParsedRow) =>
        record.isCash || step === 2 ? (
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
      title: "证券名称",
      key: "name",
      width: 130,
      render: (_: unknown, record: ParsedRow) =>
        record.isCash || step === 2 ? (
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
      title: "持仓数量/金额",
      key: "shares",
      width: 140,
      render: (_: unknown, record: ParsedRow) =>
        step === 2 ? (
          <Text>{record.shares.toLocaleString()}</Text>
        ) : (
          <InputNumber
            size="small"
            value={record.shares}
            min={0}
            precision={record.isCash ? 2 : 0}
            onChange={(v) => updateRow(record.key, "shares", v ?? 0)}
            style={{ width: "100%" }}
          />
        ),
    },
    {
      title: "平均成本",
      key: "avgCost",
      width: 120,
      render: (_: unknown, record: ParsedRow) =>
        record.isCash ? (
          <Text type="secondary">—</Text>
        ) : step === 2 ? (
          <Text>{record.avgCost.toFixed(4)}</Text>
        ) : (
          <InputNumber
            size="small"
            value={record.avgCost}
            min={0}
            precision={4}
            onChange={(v) => updateRow(record.key, "avgCost", v ?? 0)}
            style={{ width: "100%" }}
          />
        ),
    },
  ];

  const resultStatusColumn = {
    title: "状态",
    key: "status",
    width: 90,
    render: (_: unknown, record: ParsedRow) => {
      if (!record.selected) return <Text type="secondary">已跳过</Text>;
      if (record.importOk) return <Text type="success">✓ 成功</Text>;
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

  return (
    <Modal
      title={`从CSV导入持仓 — ${account.name}`}
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
          <Alert
            type="info"
            className="mb-4"
            message="支持以下A股券商导出格式"
            description={
              <ul style={{ paddingLeft: 16, marginTop: 4, marginBottom: 0 }}>
                <li>
                  <b>同花顺客户端</b>：持仓页面 → 右键导出 / 另存为CSV（含资金汇总和持仓列表）
                </li>
                <li>
                  <b>通用格式</b>：含"证券代码"、"证券名称"、"持仓数量"、"参考成本价"列的CSV
                </li>
              </ul>
            }
          />
          <Dragger
            accept=".csv,.txt"
            maxCount={1}
            fileList={fileList}
            beforeUpload={(file) => {
              setFileList([{ uid: `${file.name}-${Date.now()}`, name: file.name, originFileObj: file } as UploadFile]);
              handleFileParse(file);
              return false;
            }}
            onChange={({ fileList: fl }) => setFileList(fl)}
          >
            <p className="ant-upload-drag-icon">
              <InboxOutlined />
            </p>
            <p className="ant-upload-text">点击或将CSV文件拖拽到此处</p>
            <p className="ant-upload-hint">支持UTF-8和GB18030编码（同花顺默认GB18030）</p>
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
              message="未能解析到任何持仓数据，请检查CSV文件格式是否符合要求"
            />
          ) : (
            <>
              <Paragraph type="secondary" className="mb-2">
                共解析到 <b>{rows.length}</b> 条记录（含现金）。请确认数据后点击导入；可取消勾选不需要导入的行，也可直接编辑证券代码/名称/数量/成本。
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
                disabled={rows.length === 0 || selectedCount === 0}
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
