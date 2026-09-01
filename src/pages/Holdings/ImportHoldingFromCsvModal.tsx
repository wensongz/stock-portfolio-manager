import { useCallback } from "react";
import type { Account, CreateHoldingPayload } from "../../types";
import { useCategoryStore } from "../../stores/categoryStore";
import BrokerHoldingImportModal from "../../features/imports/BrokerHoldingImportModal.tsx";
import { parseCnHoldings } from "../../features/imports/brokers/cnHoldings.ts";
import type { HoldingImportRow } from "../../features/imports/types.ts";

interface Props {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

export default function ImportHoldingFromCsvModal(props: Props) {
  const cashCategoryId = useCategoryStore((state) => state.categories.find((category) => category.name === "现金类")?.id);
  const payloadForRow = useCallback((row: HoldingImportRow, account: Account): CreateHoldingPayload => ({
    accountId: account.id,
    symbol: row.isCash ? "$CASH-CNY" : row.symbol,
    name: row.isCash ? "现金 (CNY)" : row.name,
    market: "CN",
    categoryId: row.isCash ? cashCategoryId : undefined,
    shares: row.shares,
    avgCost: row.isCash ? 1 : row.avgCost,
    currency: "CNY",
  }), [cashCategoryId]);

  return (
    <BrokerHoldingImportModal
      {...props}
      brokerName="A 股券商"
      fixedMarket="CN"
      encodings={["utf-8", "gb18030"]}
      parse={(text) => parseCnHoldings(text)}
      payloadForRow={payloadForRow}
      uploadDescription="支持同花顺及兼容券商持仓 CSV/TSV，并保留人民币现金行和现金分类映射。"
    />
  );
}
