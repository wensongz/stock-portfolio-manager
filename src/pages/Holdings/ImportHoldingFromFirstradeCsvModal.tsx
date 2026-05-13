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
import type { Account } from "../../types";
import { useHoldingStore } from "../../stores/holdingStore";
import { invoke } from "@tauri-apps/api/core";

const { Dragger } = Upload;
const { Text, Paragraph } = Typography;

function shareInputProps() {
  return { min: 0.000001, precision: 6 };
}

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

interface ImportHoldingFromFirstradeCsvModalProps {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

// ---------------------------------------------------------------------------
// CSV / TSV parsing helpers
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

/** Auto-detect whether a line uses tabs or commas as separator. */
function splitLine(line: string): string[] {
  const tabCount   = (line.match(/\t/g) ?? []).length;
  const commaCount = (line.match(/,/g) ?? []).length;
  if (tabCount > commaCount) return line.split("\t");
  return splitCsvLine(line);
}

/**
 * Parse a Firstrade holdings CSV produced by pasting the "持有证券" page
 * from the Firstrade website into Excel (as HTML) and saving as CSV.
 *
 * The table has Chinese headers: 代号 (symbol), 股数 (shares), 单位成本 (avg cost).
 * Other columns such as 价格, 变更$, 市值, 成本, 益损$ etc. are ignored.
 *
 * The last row is typically a "Total" / summary row — it is skipped because
 * the 代号 cell will be empty or non-alphabetic.
 */
function parseFirstradeHoldingsCsv(text: string): ParseResult {
  const stripped = text.startsWith("\uFEFF") ? text.slice(1) : text;
  const lines    = stripped.split(/\r?\n/);
  const warnings: string[] = [];

  for (let i = 0; i < lines.length; i++) {
    const cols = splitLine(lines[i]).map((c) => c.trim());
    if (cols.includes("代号") && cols.includes("股数") && cols.includes("单位成本")) {
      const iSymbol    = cols.indexOf("代号");
      const iQty       = cols.indexOf("股数");
      const iCostPrice = cols.indexOf("单位成本");
      // 名称 column is optional — some exports include it, some don't
      const iName      = cols.indexOf("名称");

      const rows: ParsedRow[] = [];
      let idx = 0;

      for (let j = i + 1; j < lines.length; j++) {
        const line = lines[j].trim();
        if (!line) continue;

        const dataCols = splitLine(line);
        const raw      = (dataCols[iSymbol] ?? "").trim();
        // Skip summary / total rows (empty symbol or only digits/special chars)
        if (!raw || !/[A-Za-z]/.test(raw)) continue;

        const qty       = parseNum(dataCols[iQty]);
        const costPrice = parseNum(dataCols[iCostPrice]);
        if (isNaN(qty) || qty <= 0 || isNaN(costPrice)) continue;

        const name = iName !== -1 ? (dataCols[iName] ?? "").trim() : raw;

        rows.push({
          key:      String(idx++),
          selected: true,
          symbol:   raw.toUpperCase(),
          name:     name || raw.toUpperCase(),
          shares:   qty,
          avgCost:  costPrice,
        });
      }

      if (rows.length > 0) return { rows, warnings };
    }
  }

  return {
    rows: [],
    warnings: [
      "未找到持仓数据。请确认 CSV 来自 Firstrade 持仓页面，且包含「代号、股数、单位成本」列",
    ],
  };
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function ImportHoldingFromFirstradeCsvModal({
  open,
  account,
  onClose,
  onImported,
}: ImportHoldingFromFirstradeCsvModalProps) {
  const [step, setStep]                   = useState(0);
  const [rows, setRows]                   = useState<ParsedRow[]>([]);
  const [parseWarnings, setParseWarnings] = useState<string[]>([]);
  const [fileList, setFileList]           = useState<UploadFile[]>([]);
  const [importing, setImporting]         = useState(false);
  const [importResult, setImportResult]   = useState<{
    success: number;
    failed: number;
    errors: { name: string; error: string }[];
  } | null>(null);

  const { createHolding } = useHoldingStore();

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
        reader.onload  = (e) => resolve(e.target?.result as string);
        reader.onerror = reject;
        reader.readAsText(file, encoding);
      });

    const text   = await readAs("utf-8");
    const result = parseFirstradeHoldingsCsv(text);
    // Mark every row as looking-up so the name column shows a spinner
    const withLookup = result.rows.map((r) => ({ ...r, lookingUp: true }));
    setRows(withLookup);
    setParseWarnings(result.warnings);
    setStep(1);
    // Resolve names asynchronously (does not block navigation to step 1)
    resolveStockNames(withLookup);
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

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
          market:    "US",
          shares:    row.shares,
          avgCost:   row.avgCost,
          currency:  "USD",
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
      render: () => <Tag color="blue">US</Tag>,
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
            {...shareInputProps()}
            onChange={(v) => updateRow(record.key, "shares", v ?? 0)}
            style={{ width: "100%" }}
          />
        ),
    },
    {
      title: "单位成本",
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
      title={`从CSV导入持仓（Firstrade 美股）— ${account.name}`}
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
            message="Firstrade 持仓导入"
            description={
              <div>
                <p style={{ marginBottom: 4 }}>
                  请按以下步骤将 Firstrade 持仓数据保存为 CSV 文件后上传：
                </p>
                <ol style={{ paddingLeft: 20, marginBottom: 0 }}>
                  <li>
                    登录 <b>Firstrade 官网</b>，进入<b>持有证券</b>页面；
                  </li>
                  <li>
                    全选（Ctrl+A / Cmd+A）并复制（Ctrl+C / Cmd+C）页面上的持仓表格内容；
                  </li>
                  <li>
                    打开 <b>Excel</b>，在空白单元格右键 →<b>选择性粘贴</b>→ 选择
                    {" "}<b>HTML</b> 格式，确认粘贴；
                  </li>
                  <li>
                    另存为 <b>CSV（逗号分隔）</b> 文件；
                  </li>
                  <li>
                    将生成的 CSV 文件上传到此处。
                  </li>
                </ol>
              </div>
            }
          />
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
              支持 Firstrade 持仓页面粘贴至 Excel 后导出的 CSV 格式
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
                />
              ))}
            </div>
          )}
          <Table
            dataSource={rows}
            columns={[...baseColumns, resultStatusColumn]}
            rowKey="key"
            size="small"
            pagination={false}
            scroll={{ y: 380 }}
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
