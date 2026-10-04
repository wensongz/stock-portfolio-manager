import { Select, Space, Typography } from "antd";
import { useExchangeRateStore } from "../stores/exchangeRateStore";
import type { Currency } from "../types";

interface Props {
  onChange?: (currency: Currency) => void;
}

export default function BaseCurrencySelect({ onChange }: Props) {
  const { baseCurrency, setBaseCurrency } = useExchangeRateStore();

  return (
    <Space size="small">
      <Typography.Text type="secondary">基准货币:</Typography.Text>
      <Select<Currency>
        aria-label="基准货币"
        value={baseCurrency}
        onChange={onChange ?? setBaseCurrency}
        size="small"
        style={{ width: 120 }}
        options={[
          { value: "USD", label: "USD 美元" },
          { value: "CNY", label: "CNY 人民币" },
          { value: "HKD", label: "HKD 港元" },
        ]}
      />
    </Space>
  );
}
