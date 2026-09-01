import type { Account } from "../../types";
import BrokerTransactionImportModal from "../../features/imports/BrokerTransactionImportModal.tsx";
import { parseIbTransactions } from "../../features/imports/brokers/ibTransactions.ts";

interface Props {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

export default function ImportFromIbCsvModal(props: Props) {
  return (
    <BrokerTransactionImportModal
      {...props}
      brokerName="Interactive Brokers"
      allowPay
      parse={parseIbTransactions}
      uploadDescription="支持 IB 活动报表、Flex Query 扁平表和现金分红 CSV。"
    />
  );
}
