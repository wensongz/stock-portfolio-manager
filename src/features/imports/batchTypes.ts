export type ImportBatchKind = "transactions" | "holdings";
export type ImportBatchRowStatus = "ready" | "suspected" | "duplicate" | "failed" | "imported";
export interface ImportBatchInputRow {
  key: string;
  raw: unknown;
  external_id?: string | null;
  data: Record<string, unknown>;
}
export interface PreviewImportBatchRequest {
  request_id: string;
  account_id: string;
  source: string;
  file_name: string;
  source_content: string;
  parser_version: string;
  kind: ImportBatchKind;
  rows: ImportBatchInputRow[];
}
export interface ImportBatchRow extends ImportBatchInputRow {
  external_id: string | null;
  status: ImportBatchRowStatus;
  error: string | null;
  record_id: string | null;
}
export interface ImportBatchReconciliation {
  symbol: string;
  currency: string;
  before_shares: number;
  after_shares: number;
  expected_shares: number | null;
  difference: number | null;
}
export interface ImportBatch {
  id: string;
  account_id: string;
  source: string;
  file_name: string;
  parser_version: string;
  kind: ImportBatchKind;
  status: "preview" | "applied" | "undone";
  created_at: string;
  rows: ImportBatchRow[];
  reconciliation: ImportBatchReconciliation[];
  can_undo: boolean;
  conflict: string | null;
}
