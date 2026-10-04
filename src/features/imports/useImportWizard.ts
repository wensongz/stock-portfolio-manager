import { invoke } from "@tauri-apps/api/core";
import type { ImportBatch, ImportBatchKind } from "./batchTypes.ts";
import { batchPreviewRequest } from "./batchAdapters.ts";
import { useCallback, useState } from "react";
import type { UploadFile } from "antd/es/upload";
import type { ImportParseIssue, ImportRow, ParseResult } from "./types.ts";

export interface ImportAdapter<Row extends ImportRow> {
  parseFile: (file: File) => Promise<ParseResult<Row>>;
  accountId: string;
  source: string;
  kind: ImportBatchKind;
  toData: (row: Row) => Record<string, unknown>;
  compareRows?: (left: Row, right: Row) => number;
  prepareRows?: (rows: Row[]) => Promise<Row[]>;
}

export function useImportWizard<Row extends ImportRow>(
  adapter: ImportAdapter<Row>,
) {
  const [step, setStep] = useState(0);
  const [fileList, setFileList] = useState<UploadFile[]>([]);
  const [rows, setRows] = useState<Row[]>([]);
  const [parseError, setParseError] = useState("");
  const [warnings, setWarnings] = useState<string[]>([]);
  const [parseIssues, setParseIssues] = useState<ImportParseIssue[]>([]);
  const [problem, setProblem] = useState<"parse" | "prepare" | "preview" | null>(null);
  const [importing, setImporting] = useState(false);
  const [batch, setBatch] = useState<ImportBatch | null>(null);
  const [sourceContent, setSourceContent] = useState("");
  const [requestId, setRequestId] = useState(() => crypto.randomUUID());

  const updateRow = useCallback((key: string, patch: Partial<Row>) => {
    setRequestId(crypto.randomUUID());
    setRows((current) => current.map((row) => row.key === key ? { ...row, ...patch } : row));
  }, []);

  const reset = useCallback(() => {
    setStep(0);
    setFileList([]);
    setRows([]);
    setParseError("");
    setWarnings([]);
    setParseIssues([]);
    setProblem(null);
    setImporting(false);
    setBatch(null);
    setSourceContent("");
    setRequestId(crypto.randomUUID());
  }, []);

  const beforeUpload = useCallback((file: File) => {
    setFileList([file as unknown as UploadFile]);
    setStep(0);
    setRows([]);
    setBatch(null);
    setParseError("");
    setWarnings([]);
    setParseIssues([]);
    setProblem(null);
    setSourceContent("");
    setImporting(true);
    setRequestId(crypto.randomUUID());
    void (async () => {
      try {
        const parsed = await adapter.parseFile(file);
        setWarnings(parsed.warnings);
        setParseIssues(parsed.issues ?? []);
        setSourceContent(parsed.sourceContent ?? "");
        if (parsed.rows.length === 0) {
          setParseError(parsed.warnings[0] ?? "未从 CSV 中识别到可导入记录");
          setProblem("parse");
          return;
        }
        if (parsed.issues?.length) setProblem("parse");
        const loadingRows = adapter.prepareRows
          ? parsed.rows.map((row) => ({ ...row, lookingUp: true }))
          : parsed.rows;
        setRows(loadingRows);
        setStep(1);
        if (adapter.prepareRows) {
          try {
            const prepared = await adapter.prepareRows(loadingRows);
            setRows(prepared.map((row) => ({ ...row, lookingUp: false })));
          } catch (error) {
            setRows(loadingRows.map((row) => ({ ...row, lookingUp: false })));
            setParseError(`补全证券信息失败: ${String(error)}`);
            setProblem(parsed.issues?.length ? "parse" : "prepare");
          }
        }
      } catch (error) {
        setParseError(`CSV 解析失败: ${String(error)}`);
        setProblem("parse");
      } finally { setImporting(false); }
    })();
    return false;
  }, [adapter]);

  const importRows = useCallback(async () => {
    if (importing || problem || !rows.some((row) => row.selected)) return false;
    setImporting(true);
    try {
      const ordered = adapter.compareRows ? [...rows].sort(adapter.compareRows) : rows;
      const request = batchPreviewRequest({ requestId, accountId: adapter.accountId,
        source: adapter.source, kind: adapter.kind, fileName: fileList[0]?.name ?? "",
        sourceContent, rows: ordered, toData: adapter.toData });
      setBatch(await invoke<ImportBatch>("preview_import_batch", { request }));
      setStep(2);
      setParseError("");
    } catch (error) { setParseError(String(error)); setProblem("preview"); }
    finally { setImporting(false); }
    return true;
  }, [adapter, rows, requestId, fileList, sourceContent, importing, problem]);

  const reviseFailedRows = useCallback(() => {
    if (!batch || batch.conflict) return;
    const failedKeys = new Set(batch.rows.filter((row) => row.status === "failed").map((row) => row.key));
    // Successfully written rows stay in the saved batch and must not be staged again.
    setRows((current) => current.filter((row) => failedKeys.has(row.key)).map((row) => ({ ...row, selected: true })));
    setBatch(null);
    setRequestId(crypto.randomUUID());
    setParseError("");
    setParseIssues([]);
    setWarnings([]);
    setProblem(null);
    setStep(1);
  }, [batch]);

  return {
    step, setStep, fileList, rows, parseError, warnings, importing, setImporting, batch, setBatch,
    parseIssues, problem, dismissProblem: () => setProblem(null), reviseFailedRows,
    updateRow, reset, beforeUpload, importRows,
  };
}
