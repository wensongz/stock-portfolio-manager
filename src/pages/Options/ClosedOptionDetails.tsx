import { Space, Table, Typography } from "antd";
import type { ColumnsType } from "antd/es/table";
import { usePnlColor } from "../../hooks/usePnlColor";
import { formatMoney } from "../../lib/formatMoney";
import type { OptionCloseDetails, OptionCloseRecord } from "../../types";

interface ClosedOptionDetailsProps {
  details: OptionCloseDetails | null | undefined;
  openAmount: number;
  currency: string;
}

function formatCloseTime(value: string | null): string {
  if (!value) return "—";
  // Normalize the recorded date without changing its time or inventing a missing time.
  return value.replace(/^(\d{4})[-/](\d{1,2})[-/](\d{1,2})(?=$|[T,\s])/, (_, year: string, month: string, day: string) =>
    `${year}-${month.padStart(2, "0")}-${day.padStart(2, "0")}`);
}

function closeMethod(code: string): string {
  const codes = code.split(";").map(value => value.trim());
  if (codes.includes("Ep")) return "到期";
  if (codes.includes("A")) return "被执行";
  if (codes.includes("Ex")) return "行权";
  if (codes.includes("C")) return "买入平仓";
  return code || "—";
}

export default function ClosedOptionDetails({ details, openAmount, currency }: ClosedOptionDetailsProps) {
  const { pnlColor } = usePnlColor();

  if (!details?.records.length) {
    return <Typography.Text type="secondary">暂无平仓明细，无法计算已实现盈亏。</Typography.Text>;
  }

  const money = (value: number) => formatMoney(value, currency);
  const columns: ColumnsType<OptionCloseRecord> = [
    {
      title: "平仓时间", dataIndex: "traded_at", key: "traded_at", width: 180,
      render: formatCloseTime,
    },
    {
      title: "合约数", dataIndex: "quantity", key: "quantity", align: "right", width: 80,
      render: (value: number) => value.toLocaleString("en-US"),
    },
    { title: "平仓价", dataIndex: "price", key: "price", align: "right", render: money },
    { title: "平仓支出", dataIndex: "amount", key: "amount", align: "right", render: money },
    {
      title: "平仓费用", key: "closing_fees", align: "right",
      render: (_, record) => money(record.commission + record.fee),
    },
    { title: "方式", dataIndex: "code", key: "code", width: 100, render: closeMethod },
  ];

  return <div style={{
    display: "flex",
    flexDirection: "column",
    gap: 8,
    marginInlineStart: 64,
    width: "calc(100% - 64px)",
  }}>
    <Table<OptionCloseRecord>
      columns={columns.map(column => ({
        ...column,
        onHeaderCell: () => ({ style: { paddingBlock: 0 } }),
      }))}
      dataSource={details.records}
      rowKey="id" size="small" pagination={false} />
    <Space data-option-copy-row size={[24, 8]} wrap style={{ paddingInline: 8 }}>
      <Typography.Text data-option-copy-cell>开仓收入：{money(openAmount)}</Typography.Text>
      <Typography.Text data-option-copy-cell>平仓支出：{money(details.close_cost)}</Typography.Text>
      <Typography.Text data-option-copy-cell>总费用：{money(details.fees)}</Typography.Text>
      <Typography.Text data-option-copy-cell strong style={{ color: pnlColor(details.realized_pnl) }}>
        已实现净盈亏：{formatMoney(details.realized_pnl, currency, 2, { signDisplay: "always" })}
      </Typography.Text>
    </Space>
  </div>;
}
