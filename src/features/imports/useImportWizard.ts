import { useCallback, useState } from "react";
import type { UploadFile } from "antd/es/upload";
import type { ImportResult, ImportRow, ParseResult } from "./types.ts";

export interface ImportAdapter<Row extends ImportRow> {
  parseFile: (file: File) => Promise<ParseResult<Row>>;
  importRow: (row: Row) => Promise<void>;
  rowName: (row: Row) => string;
  compareRows?: (left: Row, right: Row) => number;
  prepareRows?: (rows: Row[]) => Promise<Row[]>;
}

interface ImportRowsOptions<Row extends ImportRow> {
  importRow: (row: Row) => Promise<void>;
  rowName: (row: Row) => string;
  compareRows?: (left: Row, right: Row) => number;
  updateRow: (key: string, patch: Partial<Row>) => void;
}

export async function importSelectedRows<Row extends ImportRow>(
  rows: Row[],
  options: ImportRowsOptions<Row>,
): Promise<ImportResult> {
  const selected = rows.filter((row) => row.selected);
  const ordered = options.compareRows ? [...selected].sort(options.compareRows) : selected;
  const result: ImportResult = { success: 0, failed: 0, errors: [] };
  for (const row of ordered) {
    try {
      await options.importRow(row);
      options.updateRow(row.key, { importOk: true, importError: undefined } as Partial<Row>);
      result.success++;
    } catch (error) {
      const detail = String(error);
      options.updateRow(row.key, { importOk: false, importError: detail } as Partial<Row>);
      result.failed++;
      result.errors.push({ name: options.rowName(row), error: detail });
    }
  }
  return result;
}

export function useImportWizard<Row extends ImportRow>(
  adapter: ImportAdapter<Row>,
  onImported: () => void,
) {
  const [step, setStep] = useState(0);
  const [fileList, setFileList] = useState<UploadFile[]>([]);
  const [rows, setRows] = useState<Row[]>([]);
  const [parseError, setParseError] = useState("");
  const [warnings, setWarnings] = useState<string[]>([]);
  const [importing, setImporting] = useState(false);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);

  const updateRow = useCallback((key: string, patch: Partial<Row>) => {
    setRows((current) => current.map((row) => row.key === key ? { ...row, ...patch } : row));
  }, []);

  const reset = useCallback(() => {
    setStep(0);
    setFileList([]);
    setRows([]);
    setParseError("");
    setWarnings([]);
    setImporting(false);
    setImportResult(null);
  }, []);

  const beforeUpload = useCallback((file: File) => {
    setFileList([file as unknown as UploadFile]);
    setParseError("");
    void (async () => {
      try {
        const parsed = await adapter.parseFile(file);
        setWarnings(parsed.warnings);
        if (parsed.rows.length === 0) {
          setParseError(parsed.warnings[0] ?? "未从 CSV 中识别到可导入记录");
          return;
        }
        const loadingRows = adapter.prepareRows
          ? parsed.rows.map((row) => ({ ...row, lookingUp: true }))
          : parsed.rows;
        setRows(loadingRows);
        setStep(1);
        if (adapter.prepareRows) {
          const prepared = await adapter.prepareRows(loadingRows);
          setRows(prepared.map((row) => ({ ...row, lookingUp: false })));
        }
      } catch (error) {
        setParseError(`CSV 解析失败: ${String(error)}`);
      }
    })();
    return false;
  }, [adapter]);

  const importRows = useCallback(async () => {
    if (!rows.some((row) => row.selected)) return false;
    setImporting(true);
    const result = await importSelectedRows(rows, {
      importRow: adapter.importRow,
      rowName: adapter.rowName,
      compareRows: adapter.compareRows,
      updateRow,
    });
    setImportResult(result);
    setImporting(false);
    setStep(2);
    if (result.success > 0) onImported();
    return true;
  }, [adapter, onImported, rows, updateRow]);

  return {
    step, setStep, fileList, rows, parseError, warnings, importing, importResult,
    updateRow, reset, beforeUpload, importRows,
  };
}
