import type { Account } from "../../types";
import BrokerHoldingImportModal from "../../features/imports/BrokerHoldingImportModal.tsx";
import { parseFirstradeHoldings } from "../../features/imports/brokers/firstradeHoldings.ts";

interface Props {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

export default function ImportHoldingFromFirstradeCsvModal(props: Props) {
  return (
    <BrokerHoldingImportModal
      {...props}
      brokerName="Firstrade"
      fixedMarket="US"
      resolveNames
      parse={(text) => parseFirstradeHoldings(text)}
      uploadDescription="支持 Firstrade 持有证券页面导出的 CSV（代号、股数、单位成本列）。"
    />
  );
}
