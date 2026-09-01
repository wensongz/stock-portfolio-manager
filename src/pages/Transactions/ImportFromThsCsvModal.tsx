import type { Account } from "../../types";
import BrokerTransactionImportModal from "../../features/imports/BrokerTransactionImportModal.tsx";
import { parseThsCsv } from "./thsCsvParser.ts";

interface Props {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

export default function ImportFromThsCsvModal(props: Props) {
  return (
    <BrokerTransactionImportModal
      {...props}
      brokerName="同花顺"
      fixedMarket="CN"
      encodings={["utf-8", "gb18030"]}
      allowPay
      parse={(text) => parseThsCsv(text)}
      uploadDescription="支持同花顺及兼容券商导出的 A 股历史成交 CSV；自动识别买入、卖出和分红。"
    />
  );
}
