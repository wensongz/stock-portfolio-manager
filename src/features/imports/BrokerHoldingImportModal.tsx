import { useMemo } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { Account, CreateHoldingPayload, Currency, Market } from "../../types";
import { holdingBatchData } from "./batchAdapters.ts";
import { readFileAsText } from "./csv.ts";
import { holdingColumns } from "./holdingColumns.tsx";
import ImportWizard from "./ImportWizard.tsx";
import { resolveStockNames, type InvokeFunction } from "./resolveStockNames.ts";
import type { HoldingImportRow, ParseResult } from "./types.ts";
import type { ImportAdapter } from "./useImportWizard.ts";

interface BrokerHoldingImportModalProps {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
  brokerName: string;
  uploadDescription: string;
  parse: (text: string, market: Market) => ParseResult<HoldingImportRow>;
  fixedMarket?: Market;
  encodings?: string[];
  resolveNames?: boolean;
  payloadForRow?: (row: HoldingImportRow, account: Account) => CreateHoldingPayload;
}

function defaultCurrency(market: Market): Currency {
  return market === "HK" ? "HKD" : market === "CN" ? "CNY" : "USD";
}

export default function BrokerHoldingImportModal({
  open,
  account,
  onClose,
  onImported,
  brokerName,
  uploadDescription,
  parse,
  fixedMarket,
  encodings = ["utf-8"],
  resolveNames: shouldResolveNames = false,
  payloadForRow,
}: BrokerHoldingImportModalProps) {
  const accountMarket = (fixedMarket ?? account.market) as Market;
  const adapter = useMemo<ImportAdapter<HoldingImportRow>>(() => ({
    accountId: account.id, source: brokerName, kind: "holdings",
    parseFile: async (file) => {
      const texts = await readFileAsText(file, encodings);
      let lastResult: ParseResult<HoldingImportRow> = { rows: [], warnings: [] };
      for (const text of texts) {
        lastResult = parse(text, accountMarket);
        if (lastResult.rows.length > 0) return { ...lastResult, sourceContent: text };
      }
      return lastResult.warnings.length > 0
        ? lastResult
        : { rows: [], warnings: [`未从 CSV 中识别到 ${brokerName} 持仓记录，请确认导出格式是否正确。`] };
    },
    prepareRows: shouldResolveNames ? async (rows) => {
      const names = await resolveStockNames(rows.map((row) => row.symbol), invoke as InvokeFunction);
      return rows.map((row) => {
        const symbol = row.symbol.toUpperCase();
        const resolved = names.get(symbol);
        return { ...row, name: resolved && resolved !== symbol ? resolved : row.name };
      });
    } : undefined,
    toData: (row) => {
      if (payloadForRow) {
        return holdingBatchData(payloadForRow(row, account));
      }
      const market = row.market ?? accountMarket;
      return holdingBatchData({
        accountId: account.id,
        symbol: row.symbol,
        name: row.name || row.symbol,
        market,
        shares: row.shares,
        avgCost: row.avgCost,
        currency: row.currency ?? defaultCurrency(market),
      });
    },
  }), [account, accountMarket, brokerName, encodings, parse, payloadForRow, shouldResolveNames]);

  return (
    <ImportWizard
      open={open}
      title={`从 ${brokerName} CSV 导入持仓`}
      accountName={account.name}
      uploadTitle={`点击或拖拽 ${brokerName} CSV 文件到此处`}
      uploadDescription={uploadDescription}
      adapter={adapter}
      columns={(updateRow, step) => holdingColumns(accountMarket, updateRow, step)}
      onClose={onClose}
      onImported={onImported}
    />
  );
}
