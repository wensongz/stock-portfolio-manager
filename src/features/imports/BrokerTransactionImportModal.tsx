import { useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import { transactionBatchData } from "./batchAdapters.ts";
import type { Account, Market } from "../../types";
import { readFileAsText } from "./csv.ts";
import ImportWizard from "./ImportWizard.tsx";
import { resolveStockNames, type InvokeFunction } from "./resolveStockNames.ts";
import { transactionColumns } from "./transactionColumns.tsx";
import type { TransactionImportRow } from "./types.ts";
import type { ImportAdapter } from "./useImportWizard.ts";

interface BrokerTransactionImportModalProps {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
  brokerName: string;
  uploadDescription: string;
  parse: (text: string, market: Market) => TransactionImportRow[];
  fixedMarket?: Market;
  encodings?: string[];
  allowPay?: boolean;
}

export default function BrokerTransactionImportModal({
  open,
  account,
  onClose,
  onImported,
  brokerName,
  uploadDescription,
  parse,
  fixedMarket,
  encodings = ["utf-8"],
  allowPay = false,
}: BrokerTransactionImportModalProps) {
  const accountMarket = (fixedMarket ?? account.market) as Market;
  const adapter = useMemo<ImportAdapter<TransactionImportRow>>(() => ({
    accountId: account.id, source: brokerName, kind: "transactions",
    parseFile: async (file) => {
      const texts = await readFileAsText(file, encodings);
      for (const text of texts) {
        const rows = parse(text, accountMarket);
        if (rows.length > 0) return { rows, warnings: [], sourceContent: text };
      }
      return { rows: [], warnings: [`未从 CSV 中识别到 ${brokerName} 交易记录，请确认导出格式是否正确。`] };
    },
    prepareRows: async (rows) => {
      const names = await resolveStockNames(rows.map((row) => row.symbol), invoke as InvokeFunction);
      return rows.map((row) => {
        const symbol = row.symbol.toUpperCase();
        const resolved = names.get(symbol);
        return { ...row, stock_name: resolved && resolved !== symbol ? resolved : row.stock_name };
      });
    },
    toData: (row) => transactionBatchData(row,
      fixedMarket ?? (row.symbol.endsWith(".HK") ? "HK" : accountMarket)),
    compareRows: (left, right) => left.traded_at.localeCompare(right.traded_at),
  }), [account.id, accountMarket, brokerName, encodings, fixedMarket, parse]);

  return (
    <ImportWizard
      open={open}
      title={`从 ${brokerName} CSV 导入交易`}
      accountName={account.name}
      uploadTitle={`点击或拖拽 ${brokerName} CSV 文件到此处`}
      uploadDescription={uploadDescription}
      adapter={adapter}
      columns={(updateRow, step) => transactionColumns(accountMarket, updateRow, step, allowPay)}
      onClose={onClose}
      onImported={onImported}
    />
  );
}
