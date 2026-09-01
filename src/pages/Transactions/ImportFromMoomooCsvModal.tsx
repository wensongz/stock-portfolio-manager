import type { Account } from "../../types";
import BrokerTransactionImportModal from "../../features/imports/BrokerTransactionImportModal.tsx";
import { parseMoomooTransactions } from "../../features/imports/brokers/moomooTransactions.ts";

interface Props {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

export default function ImportFromMoomooCsvModal(props: Props) {
  return (
    <BrokerTransactionImportModal
      {...props}
      brokerName="Moomoo"
      encodings={["utf-8", "gb18030"]}
      parse={parseMoomooTransactions}
      uploadDescription="支持 Moomoo 导出的成交记录 CSV；同一订单的多笔成交会自动合并。"
    />
  );
}
