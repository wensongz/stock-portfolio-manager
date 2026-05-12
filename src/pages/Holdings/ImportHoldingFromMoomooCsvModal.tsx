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
import type { Account, Market, Currency } from "../../types";
import { useHoldingStore } from "../../stores/holdingStore";

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
  currency: Currency;
  market: Market;
  importOk?: boolean;
  importError?: string;
}

interface ParseResult {
  rows: ParsedRow[];
  warnings: string[];
}

interface ImportHoldingFromMoomooCsvModalProps {
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
 * Format a Moomoo symbol for the target market.
 * HK stocks are plain numbers (e.g. "1211") → "1211.HK".
 * US stocks are already tickers (e.g. "AAPL") → upper-cased.
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

/**
 * Parse a Moomoo holdings CSV export.
 *
 * The exported file uses Chinese column headers. Key columns:
 *   代码        – stock code (e.g. "00267" for HK, "AAPL" for US)
 *   名称        – stock name
 *   持有数量    – shares held
 *   摊薄成本价  – diluted average cost price
 *   币种        – currency (HKD / USD / ...)
 */
function parseMoomooHoldingsCsv(text: string, market: Market): ParseResult {
  const stripped = text.startsWith("\uFEFF") ? text.slice(1) : text;
  const lines    = stripped.split(/\r?\n/);
  const warnings: string[] = [];

  for (let i = 0; i < lines.length; i++) {
    const cols = splitLine(lines[i]).map((c) => c.trim());
    if (cols.includes("代码") && cols.includes("持有数量") && cols.includes("摊薄成本价")) {
      const iCode      = cols.indexOf("代码");
      const iName      = cols.indexOf("名称");
      const iQty       = cols.indexOf("持有数量");
      const iCostPrice = cols.indexOf("摊薄成本价");
      const iCurrency  = cols.indexOf("币种");

      const rows: ParsedRow[] = [];
      let idx = 0;

      for (let j = i + 1; j < lines.length; j++) {
        const line = lines[j].trim();
        if (!line) continue;

        const dataCols = splitLine(line);
        const raw      = (dataCols[iCode] ?? "").trim();
        if (!raw) continue;

        const qty       = parseNum(dataCols[iQty]);
        const costPrice = parseNum(dataCols[iCostPrice]);
        if (isNaN(qty) || qty <= 0 || isNaN(costPrice)) continue;

        const name     = iName !== -1 ? (dataCols[iName] ?? "").trim() : raw;
        // Derive currency from the 币种 column when available; fall back to account market
        const currencyRaw = iCurrency !== -1 ? (dataCols[iCurrency] ?? "").trim().toUpperCase() : "";
        const currency: Currency =
          currencyRaw === "HKD" ? "HKD"
          : currencyRaw === "USD" ? "USD"
          : currencyRaw === "CNY" || currencyRaw === "CNH" ? "CNY"
          : market === "HK" ? "HKD"
          : "USD";

        // Determine effective market from currency when account has mixed markets
        const effectiveMarket: Market =
          currency === "HKD" ? "HK"
          : currency === "CNY" ? "CN"
          : market === "HK" ? "US"   // HK account but USD stock → likely US-listed
          : market;

        rows.push({
          key:      String(idx++),
          selected: true,
          symbol:   formatSymbol(raw, effectiveMarket),
          name,
          shares:   qty,
          avgCost:  costPrice,
          currency,
          market:   effectiveMarket,
        });
      }

      if (rows.length > 0) return { rows, warnings };
    }
  }

  return {
    rows: [],
    warnings: [
      "未找到持仓数据。请确认上传的 CSV 是 Moomoo 客户端导出的持仓文件，且包含「代码、持有数量、摊薄成本价」列",
    ],
  };
}

// ---------------------------------------------------------------------------
// Component
// ---------------------------------------------------------------------------

export default function ImportHoldingFromMoomooCsvModal({
  open,
  account,
  onClose,
  onImported,
}: ImportHoldingFromMoomooCsvModalProps) {
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

  const market: Market = account.market as Market;
  const marketLabel    = market === "HK" ? "港股" : "美股";

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
      const result = parseMoomooHoldingsCsv(text, market);
      setRows(result.rows);
      setParseWarnings(result.warnings);
      setStep(1);
    },
    [market],
  );

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
          market:    row.market,
          shares:    row.shares,
          avgCost:   row.avgCost,
          currency:  row.currency,
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
      width: 70,
      render: (_: unknown, record: ParsedRow) => {
        const color =
          record.market === "HK" ? "green"
          : record.market === "CN" ? "red"
          : "blue";
        return <Tag color={color}>{record.market}</Tag>;
      },
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
        step === 2 ? (
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
          <Alert
            type="info"
            className="mb-4"
            message="Moomoo 持仓导入"
            description={
              <div>
                <p style={{ marginBottom: 4 }}>
                  请在 Moomoo 客户端导出持仓 CSV，然后上传：
                </p>
                <ol style={{ paddingLeft: 20, marginBottom: 0 }}>
                  <li>
                    打开 Moomoo 客户端，进入<b>持仓</b>页面；
                  </li>
                  <li>
                    点击右上角<b>导出</b>（或"下载"）按钮，选择导出为
                    {" "}<b>CSV</b>；
                  </li>
                  <li>
                    将导出的 CSV 文件上传到此处。
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
              支持 Moomoo 客户端导出的持仓 CSV 格式
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
