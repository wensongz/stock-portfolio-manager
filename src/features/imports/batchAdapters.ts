import type { CreateHoldingPayload, Market } from '../../types';
import type { ImportRow, TransactionImportRow } from './types.ts';
import type { ImportBatchKind, PreviewImportBatchRequest } from './batchTypes.ts';

export function transactionBatchData(row: TransactionImportRow, market: Market): Record<string, unknown> {
  const date = new Date(row.traded_at);
  return { symbol: row.symbol.trim(), name: row.stock_name || row.symbol, market,
    currency: market === 'HK' ? 'HKD' : market === 'CN' ? 'CNY' : 'USD',
    transaction_type: row.transaction_type, shares: row.shares, price: row.price,
    total_amount: row.total_amount, commission: row.commission,
    traded_at: Number.isNaN(date.getTime()) ? row.traded_at : date.toISOString(), notes: row.notes ?? null };
}
export function holdingBatchData(row: CreateHoldingPayload): Record<string, unknown> {
  return { symbol: row.symbol, name: row.name, market: row.market, currency: row.currency,
    shares: row.shares, avg_cost: row.avgCost, category_id: row.categoryId ?? null };
}
export function batchPreviewRequest<Row extends ImportRow>(options: {
  requestId: string; accountId: string; source: string; kind: ImportBatchKind;
  fileName: string; sourceContent: string; rows: Row[];
  toData: (row: Row) => Record<string, unknown>;
}): PreviewImportBatchRequest {
  return { request_id: options.requestId, account_id: options.accountId, source: options.source,
    file_name: options.fileName, source_content: options.sourceContent, parser_version: '2',
    kind: options.kind, rows: options.rows.filter(row => row.selected).map(row => ({
      key: row.key, raw: row.raw ?? row, external_id: row.external_id ?? null, data: options.toData(row),
    })) };
}
