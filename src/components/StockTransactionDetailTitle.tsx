import { useMemo } from "react";
import type { Transaction } from "../types";
import { useQuoteStore } from "../stores/quoteStore";
import { usePnlColor } from "../hooks/usePnlColor";
import { calculateTransactionPnl } from "../lib/transactionPnl";
import { formatMoney } from "../lib/formatMoney";

interface Props {
  title: string;
  accountId: string;
  symbol: string;
  transactions: readonly Transaction[];
  loading: boolean;
}

export default function StockTransactionDetailTitle({ title, accountId, symbol, transactions, loading }: Props) {
  const currentPrice = useQuoteStore((state) => state.holdingQuotes.find((holding) =>
    holding.account_id === accountId && holding.symbol.toUpperCase() === symbol.toUpperCase(),
  )?.quote?.current_price);
  const { pnlColor } = usePnlColor();
  const pnl = useMemo(() => loading ? null : calculateTransactionPnl(transactions, currentPrice), [transactions, currentPrice, loading]);

  return (
    <div style={{ display: "flex", flexWrap: "wrap", alignItems: "baseline", columnGap: 16, rowGap: 8, paddingInlineEnd: 28 }}>
      <span>{title}</span>
      <span aria-label="交易盈亏汇总" style={{ display: "flex", flexWrap: "wrap", justifyContent: "flex-end", gap: 16, marginInlineStart: "auto", textAlign: "right", fontSize: 14, fontWeight: 400 }}>
        {[
          { label: "浮动盈亏", value: pnl?.unrealizedPnl, hint: "当前行情市值减去剩余股份的移动加权平均成本（含买入手续费）；无行情时显示 —" },
          { label: "实现盈亏", value: pnl?.realizedPnl, hint: "从最早交易起累计的卖出净收入减去对应平均成本，加上净分红；股票提取不计作卖出" },
        ].map(({ label, value, hint }) => (
          <span key={label} title={hint} style={{ whiteSpace: "nowrap" }}>
            <span style={{ color: "var(--color-text-secondary)" }}>{label}：</span>
            <span style={{ color: value == null ? undefined : pnlColor(value), fontVariantNumeric: "tabular-nums" }}>
              {value == null || !pnl ? "—" : formatMoney(value, pnl.currency, 2, { signDisplay: "exceptZero" })}
            </span>
          </span>
        ))}
      </span>
    </div>
  );
}
