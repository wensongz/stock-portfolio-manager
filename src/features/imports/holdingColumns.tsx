import { Checkbox, Input, InputNumber, Spin, Tag, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import type { Market } from "../../types";
import type { HoldingImportRow } from "./types.ts";

const { Text } = Typography;

export function holdingColumns(
  accountMarket: Market,
  updateRow: (key: string, patch: Partial<HoldingImportRow>) => void,
  step: number,
): ColumnsType<HoldingImportRow> {
  const editable = step !== 2;
  return [
    {
      title: "导入", key: "selected", width: 55,
      render: (_, row) => <Checkbox checked={row.selected} disabled={!editable} onChange={(event) => updateRow(row.key, { selected: event.target.checked })} />,
    },
    {
      title: "类型", key: "market", width: 85,
      render: (_, row) => row.isCash
        ? <Tag color="gold">现金</Tag>
        : <Tag color={(row.market ?? accountMarket) === "HK" ? "green" : (row.market ?? accountMarket) === "US" ? "blue" : "red"}>{row.market ?? accountMarket}</Tag>,
    },
    {
      title: "股票代码", key: "symbol", width: 140,
      render: (_, row) => editable && !row.isCash
        ? <Input size="small" value={row.symbol} onChange={(event) => updateRow(row.key, { symbol: event.target.value })} />
        : <Text>{row.symbol}</Text>,
    },
    {
      title: "股票名称", key: "name", width: 180,
      render: (_, row) => row.lookingUp ? <Spin size="small" /> : editable && !row.isCash
        ? <Input size="small" value={row.name} onChange={(event) => updateRow(row.key, { name: event.target.value })} />
        : <Text>{row.name}</Text>,
    },
    {
      title: rowTitle("数量", "现金金额"), key: "shares", width: 140,
      render: (_, row) => editable
        ? <InputNumber size="small" min={row.isCash ? 0 : 0.000001} precision={row.isCash ? 2 : (row.market ?? accountMarket) === "US" ? 6 : 0} value={row.shares} onChange={(value) => updateRow(row.key, { shares: value ?? 0 })} />
        : row.shares,
    },
    {
      title: "平均成本", key: "avgCost", width: 140,
      render: (_, row) => row.isCash ? 1 : editable
        ? <InputNumber size="small" min={0} precision={4} value={row.avgCost} onChange={(value) => updateRow(row.key, { avgCost: value ?? 0 })} />
        : row.avgCost,
    },
    {
      title: "币种", key: "currency", width: 80,
      render: (_, row) => row.currency ?? ((row.market ?? accountMarket) === "HK" ? "HKD" : (row.market ?? accountMarket) === "US" ? "USD" : "CNY"),
    },
  ];
}

function rowTitle(stock: string, cash: string) {
  return <span title={`股票：${stock}；现金：${cash}`}>{stock}</span>;
}
