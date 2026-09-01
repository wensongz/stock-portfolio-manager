import type { Account } from "../../types";
import BrokerTransactionImportModal from "../../features/imports/BrokerTransactionImportModal.tsx";
import { parseFirstradeTransactions } from "../../features/imports/brokers/firstradeTransactions.ts";

interface Props {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

export default function ImportFromFirstradeCsvModal(props: Props) {
  return (
    <BrokerTransactionImportModal
      {...props}
      brokerName="Firstrade"
      fixedMarket="US"
      parse={(text) => parseFirstradeTransactions(text)}
      uploadDescription="支持 Firstrade 导出的交易历史 CSV（Symbol、Quantity、Price、Action 等列）。"
    />
  );
}
