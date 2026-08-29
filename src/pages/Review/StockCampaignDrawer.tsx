import {
  Alert,
  Button,
  Card,
  Checkbox,
  Descriptions,
  Drawer,
  Empty,
  Input,
  List,
  Modal,
  Select,
  Space,
  Spin,
  Tag,
  Timeline,
  Typography,
  message,
} from "antd";
import { RobotOutlined, SaveOutlined, ToolOutlined } from "@ant-design/icons";
import { useEffect, useMemo, useState } from "react";
import type {
  Currency,
  StockCampaignDetail,
  StockReviewAnnotationInput,
  StockReviewOverrideInput,
} from "../../types";
import { formatStockReviewPercent, getStockActionTypeDisplay, getStockReviewStatusDisplay, sortStockReviewIssues } from "./stockReviewViewModel";

const { Text, Title, Paragraph } = Typography;
const { TextArea } = Input;
type OverrideType = "transfer" | "duplicate" | "same_day_order" | "non_trade";

const OVERRIDE_LABELS: Record<OverrideType, string> = {
  transfer: "跨账户转仓",
  duplicate: "排除重复交易",
  same_day_order: "确认同日反向顺序",
  non_trade: "确认非交易持仓变化",
};
const OVERRIDE_IMPACTS: Record<OverrideType, string> = {
  transfer: "连接来源与目标账户片段；组合级不再把转出/转入评价为买卖时机，也不计入换手。",
  duplicate: "从派生动作评价中排除重复记录；若源账本仍污染共享业绩，影子增益与归因会保持不可用。",
  same_day_order: "按确认顺序重放同日反向交易，可能改变动作分类、Campaign 边界与后续效果。",
  non_trade: "从交易动作与持仓事件中排除所选记录，并重新生成报告。",
};

function money(value: number | null, currency: Currency) {
  return value == null ? "—" : new Intl.NumberFormat("zh-CN", { style: "currency", currency, maximumFractionDigits: 2 }).format(value);
}
function annotationBody(valueJson: string) {
  try {
    const value = JSON.parse(valueJson) as Record<string, unknown>;
    const parts = [
      value.notes ? `季度笔记：${String(value.notes)}` : null,
      value.decision_quality ? `历史手工评价：${String(value.decision_quality)}` : null,
      value.note ? String(value.note) : null,
    ].filter((part): part is string => Boolean(part));
    return parts.join("；") || String(value.label ?? valueJson);
  } catch { return valueJson; }
}
function forwardWindow(detail: StockCampaignDetail, days: 20 | 60 | 120) {
  return days === 20 ? detail.forward_effect_20d : days === 60 ? detail.forward_effect_60d : detail.forward_effect_120d;
}

export default function StockCampaignDrawer({
  open, loading, mutating, detail, currency, reportEndDate, onClose, onAskAi,
  onSaveAnnotation, onConfirmOverride,
}: {
  open: boolean;
  loading: boolean;
  mutating: boolean;
  detail: StockCampaignDetail | null;
  currency: Currency;
  reportEndDate: string;
  onClose: () => void;
  onAskAi: (detail: StockCampaignDetail) => void;
  onSaveAnnotation: (input: StockReviewAnnotationInput) => Promise<unknown>;
  onConfirmOverride: (input: StockReviewOverrideInput) => Promise<unknown>;
}) {
  const [note, setNote] = useState("");
  const [overrideOpen, setOverrideOpen] = useState(false);
  const [overrideType, setOverrideType] = useState<OverrideType>("non_trade");
  const [transactionIds, setTransactionIds] = useState<string[]>([]);
  const allTransactionIds = useMemo(() => [...new Set(detail?.actions.flatMap((action) => action.transaction_ids) ?? [])], [detail]);
  useEffect(() => { setNote(""); setOverrideOpen(false); setTransactionIds([]); }, [detail?.summary.campaign_id]);

  const saveNote = async () => {
    if (!detail || !note.trim()) return;
    const saved = await onSaveAnnotation({
      id: `campaign-note:${detail.summary.campaign_id}:${Date.now()}`,
      scope_type: "campaign",
      scope_key: detail.summary.campaign_id,
      account_id: detail.summary.account_ids.length === 1 ? detail.summary.account_ids[0] : null,
      symbol: detail.summary.symbol,
      annotation_type: "note",
      value_json: JSON.stringify({ note: note.trim(), effective_date: reportEndDate }),
      source: "user",
    });
    if (saved) { setNote(""); message.success("复盘注释已保存"); }
  };
  const confirmOverride = async () => {
    if (!detail || transactionIds.length === 0) return;
    const report = await onConfirmOverride({
      id: `campaign-override:${detail.summary.campaign_id}:${Date.now()}`,
      override_type: overrideType,
      transaction_ids_json: JSON.stringify(transactionIds),
      value_json: overrideType === "same_day_order" ? JSON.stringify(transactionIds) : "{}",
    });
    if (report) { setOverrideOpen(false); message.success("纠正已确认，报告已采用后端返回的新结果"); }
  };

  return (
    <>
      <Drawer
        open={open}
        onClose={onClose}
        size="large"
        title={detail ? `${detail.summary.symbol} · Campaign 详情` : "Campaign 详情"}
        extra={<Button icon={<RobotOutlined />} disabled={!detail} onClick={() => detail && onAskAi(detail)}>请 AI 复盘 Campaign</Button>}
        aria-label="股票 Campaign 详情"
      >
        {loading && !detail ? <div style={{ textAlign: "center", padding: 48 }}><Spin /></div> : !detail ? <Empty description="暂无 Campaign 详情" /> : (
          <Space orientation="vertical" size="large" style={{ width: "100%" }}>
            <Space wrap><Title level={4} style={{ margin: 0 }}>{detail.summary.symbol}</Title><Tag>{detail.summary.market}</Tag><Tag color={detail.summary.campaign_status === "active" ? "blue" : "green"}>{detail.summary.campaign_status === "active" ? "进行中" : "已完成"}</Tag>{detail.fact_labels.map((label) => <Tag key={label}>{label}</Tag>)}</Space>
            {detail.summary.campaign_status === "active" && <Alert type="info" showIcon message="进行中 Campaign 的总盈亏明确包含剩余持仓市值，不是已实现收益。" />}
            <Descriptions bordered size="small" column={{ xs: 1, sm: 2, lg: 3 }}>
              <Descriptions.Item label="开始 / 结束">{detail.summary.started_at.slice(0, 10)} / {detail.summary.ended_at?.slice(0, 10) ?? "进行中"}</Descriptions.Item>
              <Descriptions.Item label="Campaign 总盈亏">{money(detail.pnl.total_pnl_base, currency)}</Descriptions.Item>
              <Descriptions.Item label="超额收益">{formatStockReviewPercent(detail.excess_return)}</Descriptions.Item>
              <Descriptions.Item label="买入支出">{money(detail.pnl.buy_outlays_base, currency)}</Descriptions.Item>
              <Descriptions.Item label="卖出收入">{money(detail.pnl.sell_proceeds_base, currency)}</Descriptions.Item>
              <Descriptions.Item label="剩余持仓市值">{money(detail.pnl.remaining_market_value_base, currency)}</Descriptions.Item>
              <Descriptions.Item label="分红 / 费用">{money(detail.pnl.dividends_base, currency)} / {money(detail.pnl.trading_fees_base, currency)}</Descriptions.Item>
              <Descriptions.Item label="最大投入资本">{money(detail.pnl.max_invested_capital_base, currency)}</Descriptions.Item>
              <Descriptions.Item label="最大持仓金额 / 权重">— / —</Descriptions.Item>
              <Descriptions.Item label="MAE">{money(detail.mae_base, currency)} / {formatStockReviewPercent(detail.mae_percent)}</Descriptions.Item>
              <Descriptions.Item label="MFE">{money(detail.mfe_base, currency)} / {formatStockReviewPercent(detail.mfe_percent)}</Descriptions.Item>
              <Descriptions.Item label="持有期回撤">{formatStockReviewPercent(detail.holding_period_drawdown)}</Descriptions.Item>
              <Descriptions.Item label="组合调仓贡献">{money(detail.summary.contribution, currency)}</Descriptions.Item>
              <Descriptions.Item label="已完成 / 进行中样本">{detail.completed_sample_count} / {detail.active_sample_count}</Descriptions.Item>
            </Descriptions>
            <Text type="secondary">{detail.pnl.label}。后端当前契约未提供单 Campaign 峰值持仓金额与权重，因此该项显示为 —，不从动作记录推算。</Text>

            <Card size="small" title="20 / 60 / 120 个交易日后续效果">
              <Descriptions size="small" column={{ xs: 1, sm: 3 }}>
                {([20, 60, 120] as const).map((days) => {
                  const window = forwardWindow(detail, days);
                  const display = getStockReviewStatusDisplay(window.status.status);
                  return <Descriptions.Item key={days} label={`${days} 日`}><Space wrap><Text>{formatStockReviewPercent(window.amount_weighted_excess_return)}</Text><Tag color={display.color}>{display.label}</Tag><Text type="secondary">{window.matured_actions} 成熟 / {window.pending_actions} 观察中</Text></Space></Descriptions.Item>;
                })}
              </Descriptions>
            </Card>
            <Card size="small" title="账户片段">
              <List size="small" dataSource={detail.summary.fragments} renderItem={(fragment) => <List.Item><Descriptions size="small" column={{ xs: 1, sm: 2 }} style={{ width: "100%" }}><Descriptions.Item label="账户">{fragment.account_id}</Descriptions.Item><Descriptions.Item label="状态">{fragment.status === "active" ? "进行中" : "已完成"}</Descriptions.Item><Descriptions.Item label="区间">{fragment.started_at.slice(0, 10)} → {fragment.ended_at?.slice(0, 10) ?? "进行中"}</Descriptions.Item><Descriptions.Item label="转仓">{fragment.transfer_in ? `转入 ${fragment.transfer_in.transaction_id}` : ""}{fragment.transfer_in && fragment.transfer_out ? "；" : ""}{fragment.transfer_out ? `转出 ${fragment.transfer_out.transaction_id}` : "无"}</Descriptions.Item></Descriptions></List.Item>} />
            </Card>
            <Card size="small" title="操作与现金流时间线">
              <Timeline items={detail.timeline.map((item) => ({ children: <Space wrap><Text>{item.date}</Text><Tag>{item.kind === "buy" ? "买入" : item.kind === "sell" ? "卖出" : item.kind === "dividend" ? "分红" : "费用"}</Tag><Text>{money(item.amount_base, currency)}</Text><Text type="secondary">{item.shares} 股 · {item.account_id}</Text></Space> }))} />
              <List size="small" dataSource={detail.actions} renderItem={(action) => <List.Item><Space wrap><Text>{action.traded_at.slice(0, 10)}</Text><Tag>{getStockActionTypeDisplay(action.action_type)}</Tag><Text>{action.symbol}</Text><Text type="secondary">{action.transaction_ids.join("、")}</Text></Space></List.Item>} />
            </Card>
            <Card size="small" title="季度历史笔记与复盘注释">
              {detail.annotations.length ? <List size="small" dataSource={detail.annotations} renderItem={(annotation) => <List.Item><Space orientation="vertical" size={2}><Space wrap><Tag color={annotation.annotation_type === "historical_manual_assessment" ? "default" : "blue"}>{annotation.annotation_type === "historical_manual_assessment" ? "历史手工评价" : "复盘注释"}</Tag><Text type="secondary">{annotation.updated_at.slice(0, 10)}</Text></Space><Paragraph style={{ margin: 0 }}>{annotationBody(annotation.value_json)}</Paragraph></Space></List.Item>} /> : <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="暂无历史笔记或注释" />}
              <Space.Compact style={{ width: "100%", marginTop: 10 }}><TextArea aria-label="Campaign 复盘注释" value={note} onChange={(event) => setNote(event.target.value)} autoSize={{ minRows: 2, maxRows: 5 }} placeholder="记录操作原因、目标仓位、预期持有周期或投资逻辑；注释不会改变确定性指标。" /><Button icon={<SaveOutlined />} loading={mutating} disabled={!note.trim()} onClick={saveNote}>保存</Button></Space.Compact>
            </Card>
            <Card size="small" title="数据问题与计算纠正" extra={<Button icon={<ToolOutlined />} onClick={() => { setTransactionIds([]); setOverrideOpen(true); }}>发起结构化纠正</Button>}>
              {detail.issues.length ? <List size="small" dataSource={sortStockReviewIssues(detail.issues)} renderItem={(issue) => <List.Item><Space wrap><Tag color={issue.severity === "error" ? "red" : issue.severity === "warning" ? "gold" : "blue"}>{issue.severity === "error" ? "阻断" : issue.severity === "warning" ? "警告" : "信息"}</Tag><Text>{issue.message}</Text><Text type="secondary">{issue.code}</Text></Space></List.Item>} /> : <Text type="secondary">当前 Campaign 没有数据质量问题。</Text>}
            </Card>
          </Space>
        )}
      </Drawer>
      <Modal open={overrideOpen} title="确认股票复盘计算纠正" okText="确认并重新生成报告" cancelText="取消" confirmLoading={mutating} okButtonProps={{ disabled: transactionIds.length === 0 }} onCancel={() => setOverrideOpen(false)} onOk={confirmOverride}>
        <Space orientation="vertical" size="middle" style={{ width: "100%" }}>
          <Alert type="warning" showIcon message="纠正只改变复盘分类与计算覆盖，不会删除或改写原始交易。只有点击下方确认按钮后才会保存。" />
          <Select aria-label="纠正类型" value={overrideType} onChange={setOverrideType} style={{ width: "100%" }} options={(Object.keys(OVERRIDE_LABELS) as OverrideType[]).map((value) => ({ value, label: OVERRIDE_LABELS[value] }))} />
          <Text strong>预期影响</Text><Paragraph>{OVERRIDE_IMPACTS[overrideType]}</Paragraph><Text strong>受影响交易</Text>
          {allTransactionIds.length ? <Checkbox.Group aria-label="选择受影响交易" value={transactionIds} onChange={(values) => setTransactionIds(values as string[])} style={{ display: "flex", flexDirection: "column", gap: 8 }} options={allTransactionIds.map((id) => ({ value: id, label: id }))} /> : <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} description="当前 Campaign 没有可引用的交易记录" />}
          {overrideType === "same_day_order" && transactionIds.length > 0 && <Alert type="info" message={`确认顺序：${transactionIds.join(" → ")}（可按勾选顺序重新选择）`} />}
        </Space>
      </Modal>
    </>
  );
}
