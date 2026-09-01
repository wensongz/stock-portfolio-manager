import { Checkbox, DatePicker, Input, InputNumber, Select, Spin, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import dayjs from "dayjs";
import type { Market } from "../../types";
import type { TransactionImportRow } from "./types.ts";

const { Text } = Typography;

export function transactionColumns(
  market: Market,
  updateRow: (key: string, patch: Partial<TransactionImportRow>) => void,
  step: number,
  allowPay = false,
): ColumnsType<TransactionImportRow> {
  const shareProps = market === "US" ? { min: 0.000001, precision: 6 } : { min: 1, precision: 0 };
  const editable = step !== 2;
  return [
    {
      title: "导入", key: "selected", width: 55, fixed: "left",
      render: (_, row) => <Checkbox checked={row.selected} disabled={!editable} onChange={(event) => updateRow(row.key, { selected: event.target.checked })} />,
    },
    {
      title: "类型", key: "transaction_type", width: 82,
      render: (_, row) => editable ? (
        <Select size="small" value={row.transaction_type} style={{ width: 72 }} onChange={(value) => updateRow(row.key, { transaction_type: value })}>
          <Select.Option value="BUY"><Tag color="green">买入</Tag></Select.Option>
          <Select.Option value="SELL"><Tag color="red">卖出</Tag></Select.Option>
          {allowPay && <Select.Option value="PAY"><Tag color="gold">分红</Tag></Select.Option>}
        </Select>
      ) : <Tag color={row.transaction_type === "BUY" ? "green" : row.transaction_type === "SELL" ? "red" : "gold"}>{row.transaction_type}</Tag>,
    },
    {
      title: "股票代码", key: "symbol", width: 125,
      render: (_, row) => editable
        ? <Input size="small" value={row.symbol} onChange={(event) => updateRow(row.key, { symbol: event.target.value })} />
        : <Text>{row.symbol}</Text>,
    },
    {
      title: "股票名称", key: "stock_name", width: 160,
      render: (_, row) => row.lookingUp ? <Spin size="small" /> : editable
        ? <Input size="small" value={row.stock_name} onChange={(event) => updateRow(row.key, { stock_name: event.target.value })} />
        : <Text>{row.stock_name}</Text>,
    },
    {
      title: "交易时间", key: "traded_at", width: 185,
      render: (_, row) => editable
        ? <DatePicker showTime size="small" value={row.traded_at ? dayjs(row.traded_at) : null} onChange={(value) => updateRow(row.key, { traded_at: value?.format("YYYY-MM-DDTHH:mm:ss") ?? "" })} />
        : <Text>{row.traded_at.replace("T", " ")}</Text>,
    },
    {
      title: "价格", key: "price", width: 115,
      render: (_, row) => editable
        ? <InputNumber size="small" min={0} precision={4} value={row.price} onChange={(value) => updateRow(row.key, { price: value ?? 0 })} />
        : row.price,
    },
    {
      title: "数量", key: "shares", width: 115,
      render: (_, row) => editable
        ? <InputNumber size="small" {...shareProps} value={row.shares} onChange={(value) => updateRow(row.key, { shares: value ?? 0 })} />
        : row.shares,
    },
    {
      title: "成交金额", key: "total_amount", width: 125,
      render: (_, row) => editable
        ? <InputNumber size="small" min={0} precision={2} value={row.total_amount} onChange={(value) => updateRow(row.key, { total_amount: value ?? 0 })} />
        : row.total_amount,
    },
    {
      title: "费用", key: "commission", width: 110,
      render: (_, row) => editable
        ? <InputNumber size="small" min={0} precision={2} value={row.commission} onChange={(value) => updateRow(row.key, { commission: value ?? 0 })} />
        : row.commission,
    },
    {
      title: "备注", key: "notes", width: 160,
      render: (_, row) => editable
        ? <Input size="small" value={row.notes} onChange={(event) => updateRow(row.key, { notes: event.target.value })} />
        : row.notes,
    },
  ];
}

