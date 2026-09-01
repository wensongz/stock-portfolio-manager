import type { Account } from "../../types";
import BrokerHoldingImportModal from "../../features/imports/BrokerHoldingImportModal.tsx";
import { parseIbHoldings } from "../../features/imports/brokers/ibHoldings.ts";

interface Props {
  open: boolean;
  account: Account;
  onClose: () => void;
  onImported: () => void;
}

export default function ImportHoldingFromIbCsvModal(props: Props) {
  return (
    <BrokerHoldingImportModal
      {...props}
      brokerName="Interactive Brokers"
      resolveNames
      parse={parseIbHoldings}
      uploadDescription="支持 IB 活动报表 Open Positions 段落或含 Symbol、Quantity、Cost Price 的扁平表。"
    />
  );
}
