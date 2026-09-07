import { Alert, Button, Space, Spin, Typography } from "antd";
import type { CashBalanceReconciliation, Holding } from "../../types";
import type { HoldingRequestState } from "./cashBalanceEditing";
import { formatCashDelta } from "./cashBalanceEditing";
import { formatMoney } from "../../lib/formatMoney";

export default function CashBalancePreview({ holding, state, onRetry, onAdopt, onDetail, onEdit, disabled = false }: {
  holding: Holding;
  state: HoldingRequestState<CashBalanceReconciliation>;
  onRetry: () => void;
  onAdopt?: (balance: number) => void;
  onDetail?: () => void;
  onEdit?: () => void;
  disabled?: boolean;
}) {
  const data = state.holdingId === holding.id && state.status === "ready" && state.data?.holding_id === holding.id ? state.data : null;
  const loading = state.holdingId !== holding.id || state.status === "idle" || state.status === "loading";
  return <div style={{ marginBottom: 16 }}>
    <Space wrap style={{ marginBottom: 8 }}>
      <Typography.Text>当前余额：{formatMoney(data?.current_balance ?? holding.shares, holding.currency)}</Typography.Text>
      <Typography.Text>流水推荐：{data?.recommended_balance == null ? "—" : formatMoney(data.recommended_balance, holding.currency)}</Typography.Text>
      <Typography.Text>差额（推荐 − 当前）：{data?.difference == null ? "—" : formatCashDelta(data.difference, holding.currency)}</Typography.Text>
    </Space>
    {loading && <div><Spin size="small" /> 正在核对资金流水…</div>}
    {state.holdingId === holding.id && state.status === "error" && <Alert showIcon type="warning" title="余额核对失败，未提供推荐值。草稿已保留。" description={state.error} action={<Button size="small" onClick={onRetry} disabled={disabled}>重试</Button>} />}
    {data && <>
      {data.recommended_balance === null && <Typography.Paragraph type="secondary">暂无有效资金流水，可填写现金期初余额。</Typography.Paragraph>}
      {data.opening_count === 0 && data.rows.length > 0 && <Typography.Paragraph type="secondary">按已记录流水计算，未记录期初按0。</Typography.Paragraph>}
      {data.opening_count > 1 && <Alert showIcon type="warning" title="存在多条现金期初，自定义余额前请先整理期初记录；仍可采用推荐值。" style={{ marginBottom: 8 }} />}
    </>}
    <Space wrap>
      {data && <Button size="small" onClick={onRetry} disabled={disabled}>重新核对</Button>}
      {onAdopt && <Button size="small" disabled={disabled || data?.recommended_balance == null || !Number.isFinite(data.recommended_balance)} onClick={() => {
        if (data?.recommended_balance != null && Number.isFinite(data.recommended_balance)) onAdopt(data.recommended_balance);
      }}>采用推荐值</Button>}
      {onDetail && <Button size="small" onClick={onDetail}>查看资金明细</Button>}
      {onEdit && <Button size="small" onClick={onEdit}>编辑现金余额</Button>}
    </Space>
    {onAdopt && <Typography.Paragraph type="secondary" style={{ marginTop: 8, marginBottom: 0 }}>采用推荐值仅填入表单，确认后保存。自定义余额与推荐值的差额用于校正期初余额，原有买卖和存取款流水保留。</Typography.Paragraph>}
  </div>;
}
