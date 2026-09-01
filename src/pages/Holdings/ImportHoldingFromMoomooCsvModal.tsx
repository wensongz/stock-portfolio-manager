import type { Account } from "../../types";
import BrokerHoldingImportModal from "../../features/imports/BrokerHoldingImportModal.tsx";
import { parseMoomooHoldings } from "../../features/imports/brokers/moomooHoldings.ts";

interface Props {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

export default function ImportHoldingFromMoomooCsvModal(props: Props) {
  return (
    <BrokerHoldingImportModal
      {...props}
      brokerName="Moomoo"
      encodings={["utf-8", "gb18030"]}
      parse={parseMoomooHoldings}
      uploadDescription="支持 Moomoo 持仓 CSV，并按每行币种识别港股、美股或 A 股。"
    />
  );
}
